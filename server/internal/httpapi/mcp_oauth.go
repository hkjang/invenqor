package httpapi

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"slices"
	"strings"
	"time"

	"github.com/coreos/go-oidc/v3/oidc"
	"github.com/go-chi/chi/v5/middleware"
	"github.com/hkjang/invenqor/server/internal/apikeys"
	"github.com/hkjang/invenqor/server/internal/auth"
)

// MCP by SSO - a Keycloak access token instead of a personal key.
//
// The MCP authorization specification (2025-06-18 and later) is OAuth 2.1: the
// MCP server is a resource server that publishes where its authorization
// server is (RFC 9728, /.well-known/oauth-protected-resource), and a client
// refused with 401 reads that document, sends the person through Keycloak
// with PKCE and comes back with an access token whose audience (RFC 8707) is
// this server. Nothing about issuing tokens happens here. This file answers
// two questions only: where is the authorization server, and was this token
// issued for us.
//
// The personal key stays as it is. A token is a second door into the same
// room: it authenticates an existing account, carries the scopes the
// administrator chose, and is held to the same permission checks a key is. It
// never creates an account, never revives an inactive one, and never turns a
// role claim into a permission. It is accepted on /mcp and nowhere else.

const (
	mcpOAuthKeyEnabled  = "mcp.oauth.enabled"
	mcpOAuthKeyResource = "mcp.oauth.resource"
	mcpOAuthKeyAudience = "mcp.oauth.audience"
	mcpOAuthKeyScopes   = "mcp.oauth.scopes"
	mcpOAuthKeyPrefix   = "mcp.oauth."

	mcpPath                  = "/mcp"
	protectedResourcePath    = "/.well-known/oauth-protected-resource"
	protectedResourceMCPPath = protectedResourcePath + mcpPath

	// mcpOAuthClockSkew is the leeway allowed on nbf, which go-oidc does not
	// check itself. Keycloak stamps nbf=0 on ordinary tokens; the leeway is
	// for realms that do set it.
	mcpOAuthClockSkew = 30 * time.Second
)

// mcpOAuthDefaultScopes are the read-only scopes the MCP tools need. An SSO
// subject gets these unless the administrator narrows or widens them; the
// token's own scope claim is not consulted, so Keycloak never has to learn
// this product's scope vocabulary.
var mcpOAuthDefaultScopes = []string{"agents.read", "assets.read", "mcp.access", "relations.read"}

var mcpOAuthSigningAlgorithms = []string{
	oidc.RS256, oidc.RS384, oidc.RS512,
	oidc.ES256, oidc.ES384, oidc.ES512,
	oidc.PS256, oidc.PS384, oidc.PS512,
}

// mcpOAuthConfig is what the administrator stores under the company-standard
// mcp.oauth.* keys. The issuer, client and username claim are the web
// sign-in's own settings and are not duplicated here.
type mcpOAuthConfig struct {
	Enabled  bool
	Resource string
	Audience []string
	Scopes   []string
}

func defaultMCPOAuthConfig() mcpOAuthConfig {
	return mcpOAuthConfig{Scopes: slices.Clone(mcpOAuthDefaultScopes)}
}

func (c mcpOAuthConfig) values() map[string]any {
	return map[string]any{
		mcpOAuthKeyEnabled:  c.Enabled,
		mcpOAuthKeyResource: c.Resource,
		mcpOAuthKeyAudience: strings.Join(c.Audience, " "),
		mcpOAuthKeyScopes:   strings.Join(c.Scopes, " "),
	}
}

func (c mcpOAuthConfig) normalize() mcpOAuthConfig {
	c.Resource = strings.TrimSpace(c.Resource)
	c.Audience = uniqueFields(c.Audience)
	c.Scopes = uniqueFields(c.Scopes)
	if len(c.Scopes) == 0 {
		c.Scopes = slices.Clone(mcpOAuthDefaultScopes)
	}
	return c
}

func (c mcpOAuthConfig) validate() error {
	if c.Resource != "" {
		parsed, err := url.Parse(c.Resource)
		if err != nil || (parsed.Scheme != "https" && parsed.Scheme != "http") ||
			parsed.Host == "" || parsed.User != nil || parsed.RawQuery != "" ||
			parsed.Fragment != "" {
			return errors.New("resource must be an absolute http(s) URL without query or fragment")
		}
	}
	if _, err := apikeys.RequireScopes(c.Scopes); err != nil {
		return err
	}
	return nil
}

// uniqueFields splits every entry on whitespace so "a b" and ["a","b"] mean
// the same thing, then drops duplicates while keeping the order given.
func uniqueFields(values []string) []string {
	var result []string
	for _, value := range values {
		for _, field := range strings.Fields(value) {
			if !slices.Contains(result, field) {
				result = append(result, field)
			}
		}
	}
	return result
}

// loadMCPOAuthConfig reads every mcp.oauth.* row. A fresh installation has
// none and gets the default, which is off; nothing is written by a read.
func (s *Server) loadMCPOAuthConfig(ctx context.Context) (mcpOAuthConfig, error) {
	rows, err := s.database.DB().QueryContext(ctx,
		`SELECT key,value_json FROM settings WHERE key LIKE 'mcp.oauth.%'`)
	if err != nil {
		return mcpOAuthConfig{}, fmt.Errorf("load mcp oauth settings: %w", err)
	}
	defer rows.Close()
	config := defaultMCPOAuthConfig()
	for rows.Next() {
		var key string
		var raw any
		if err := rows.Scan(&key, &raw); err != nil {
			return mcpOAuthConfig{}, fmt.Errorf("load mcp oauth settings: %w", err)
		}
		var value any
		if json.Unmarshal([]byte(valueString(raw)), &value) != nil {
			continue
		}
		switch key {
		case mcpOAuthKeyEnabled:
			enabled, _ := value.(bool)
			config.Enabled = enabled
		case mcpOAuthKeyResource:
			text, _ := value.(string)
			config.Resource = text
		case mcpOAuthKeyAudience:
			config.Audience = settingFields(value)
		case mcpOAuthKeyScopes:
			config.Scopes = settingFields(value)
		}
	}
	if err := rows.Err(); err != nil {
		return mcpOAuthConfig{}, fmt.Errorf("load mcp oauth settings: %w", err)
	}
	return config.normalize(), nil
}

// settingFields accepts the space-separated string the standard names and a
// JSON array, which is what somebody writing through the generic settings
// API will reasonably send.
func settingFields(value any) []string {
	switch typed := value.(type) {
	case string:
		return strings.Fields(typed)
	case []any:
		var result []string
		for _, item := range typed {
			if text, ok := item.(string); ok {
				result = append(result, strings.Fields(text)...)
			}
		}
		return result
	default:
		return nil
	}
}

func (s *Server) saveMCPOAuthConfig(
	ctx context.Context, updatedBy string, config mcpOAuthConfig,
) error {
	tx, err := s.database.DB().BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	now := time.Now().UTC()
	for key, value := range config.values() {
		encoded, err := json.Marshal(value)
		if err != nil {
			return err
		}
		if _, err := tx.ExecContext(ctx,
			`INSERT INTO settings(
			 key,value_json,secret,apply_mode,pending_value_json,version,
			 updated_by,updated_at
			 ) VALUES($1,$2,FALSE,'immediate',NULL,1,$3,$4)
			 ON CONFLICT(key) DO UPDATE SET
			 value_json=excluded.value_json,secret=FALSE,
			 apply_mode='immediate',pending_value_json=NULL,
			 version=settings.version+1,updated_by=excluded.updated_by,
			 updated_at=excluded.updated_at`,
			key, string(encoded), updatedBy, now); err != nil {
			return fmt.Errorf("save %s: %w", key, err)
		}
	}
	return tx.Commit()
}

// mcpOAuthState is the stored configuration joined with the web sign-in's
// Keycloak settings and reduced to one answer: is SSO for MCP live. Enabled
// without a usable issuer behaves as off and says why, so a half-configured
// installation never publishes metadata it cannot honour - metadata that
// leads to a refused token puts the client in a login loop.
type mcpOAuthState struct {
	Config mcpOAuthConfig
	OIDC   auth.OIDCSettings
	Active bool
	// Inactive is why Enabled did not become Active; empty otherwise.
	Inactive string
}

func (s *Server) mcpOAuthState(ctx context.Context) (mcpOAuthState, error) {
	config, err := s.loadMCPOAuthConfig(ctx)
	if err != nil {
		return mcpOAuthState{}, err
	}
	state := mcpOAuthState{Config: config}
	if s.oidcService == nil {
		state.Inactive = "Keycloak is not configured"
		return state, nil
	}
	state.OIDC, err = s.oidcService.Settings(ctx)
	if err != nil {
		return mcpOAuthState{}, err
	}
	if err := state.OIDC.ValidateIssuer(); err != nil {
		state.Inactive = err.Error()
		return state, nil
	}
	state.Active = config.Enabled
	return state, nil
}

func (state mcpOAuthState) issuer() string {
	return strings.TrimRight(strings.TrimSpace(state.OIDC.EffectiveIssuer()), "/")
}

// resource is the identifier this deployment claims for its MCP endpoint:
// what the metadata advertises and what a token's aud may name. It has to be
// the public address the client actually connects to, so the explicit
// setting wins, then the public origin the Keycloak callback was configured
// with, and only then the request's Host - anybody can set a Host header.
func (state mcpOAuthState) resource(r *http.Request) string {
	if state.Config.Resource != "" {
		return state.Config.Resource
	}
	if redirect, err := url.Parse(strings.TrimSpace(state.OIDC.RedirectURI)); err == nil &&
		redirect.Scheme != "" && redirect.Host != "" {
		return redirect.Scheme + "://" + redirect.Host + mcpPath
	}
	scheme := "http"
	if requestIsHTTPS(r) {
		scheme = "https"
	}
	return scheme + "://" + r.Host + mcpPath
}

// metadataURL is where a refused client is sent to learn the above.
func metadataURL(resource string) string {
	return strings.TrimSuffix(resource, mcpPath) + protectedResourceMCPPath
}

func (state mcpOAuthState) public(r *http.Request) map[string]any {
	resource := state.resource(r)
	payload := map[string]any{
		"enabled":            state.Config.Enabled,
		"resource":           state.Config.Resource,
		"audience":           append([]string{}, state.Config.Audience...),
		"scopes":             append([]string{}, state.Config.Scopes...),
		"active":             state.Active,
		"inactive_reason":    state.Inactive,
		"oidc_issuer":        state.issuer(),
		"effective_resource": resource,
		"metadata_url":       metadataURL(resource),
	}
	return payload
}

// protectedResourceMetadata is RFC 9728: the document a refused MCP client
// reads to find the authorization server. Public by design - it says where
// to sign in, not who is signed in - and bare JSON rather than this API's
// envelope, because the reader is an OAuth client library.
func (s *Server) protectedResourceMetadata(w http.ResponseWriter, r *http.Request) {
	state, err := s.mcpOAuthState(r.Context())
	if err != nil {
		s.internalError(w, r, err)
		return
	}
	if !state.Active {
		if state.Config.Enabled {
			s.logger.Warn("mcp_oauth_inactive",
				"request_id", middleware.GetReqID(r.Context()), "reason", state.Inactive)
		}
		writeAPIError(w, r, http.StatusNotFound, "MCP_OAUTH_DISABLED",
			"This server's MCP endpoint does not accept SSO access tokens. Use a personal API key.")
		return
	}
	resource := state.resource(r)
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "public, max-age=300")
	w.Header().Set("Access-Control-Allow-Origin", "*")
	_ = json.NewEncoder(w).Encode(map[string]any{
		"resource":                 resource,
		"authorization_servers":    []string{state.issuer()},
		"bearer_methods_supported": []string{"header"},
		"scopes_supported":         state.Config.Scopes,
		"resource_name":            "Invenqor MCP",
	})
}

// mcpChallenge turns a 401 on the MCP path into an invitation: the client
// reads resource_metadata and starts the OAuth flow from there. Without it
// a refusal is a dead end. It is attached on /mcp only; a REST 401 carrying
// it would send browsers and other clients somewhere they cannot use.
func (s *Server) mcpChallenge(w http.ResponseWriter, r *http.Request, state mcpOAuthState, invalidToken bool) {
	if r.URL.Path != mcpPath || !state.Active {
		w.Header().Set("WWW-Authenticate", `Bearer realm="invenqor-api"`)
		return
	}
	challenge := fmt.Sprintf(`Bearer realm="invenqor-api", resource_metadata=%q`,
		metadataURL(state.resource(r)))
	if invalidToken {
		challenge += `, error="invalid_token"`
	}
	w.Header().Set("WWW-Authenticate", challenge)
}

// looksLikeJWT is the cheap shape test that separates "not a key" from "a
// token we can try", so a bearer that is neither gets exactly the refusal
// it always did.
func looksLikeJWT(token string) bool {
	if strings.HasPrefix(token, "ivq_sk_") || len(token) > 32*1024 {
		return false
	}
	parts := strings.Split(token, ".")
	return len(parts) == 3 && parts[0] != "" && parts[1] != "" && parts[2] != ""
}

// mcpOAuthRefusal is a refusal the caller can act on: the status and code
// the API reports and a message that says what was seen and what to change.
type mcpOAuthRefusal struct {
	status  int
	code    string
	message string
	// token is whether a token was presented and rejected, which the
	// challenge header reports as error="invalid_token".
	token bool
}

// oauthPrincipal turns a bearer access token into a principal for /mcp, or
// says exactly why it will not.
func (s *Server) oauthPrincipal(
	r *http.Request, state mcpOAuthState, token string,
) (auth.Principal, *mcpOAuthRefusal) {
	ctx := r.Context()
	provider, err := s.oidcService.AccessTokenProvider(ctx, state.OIDC)
	if err != nil {
		s.logger.Warn("mcp_oauth_discovery_failed",
			"request_id", middleware.GetReqID(ctx), "issuer", state.issuer(), "error", err)
		return auth.Principal{}, &mcpOAuthRefusal{
			status: http.StatusServiceUnavailable, code: "MCP_OAUTH_ISSUER_UNREACHABLE",
			message: "The Keycloak issuer could not be read, so SSO tokens cannot be verified. Retry later or tell an administrator.",
		}
	}
	// Signature, issuer and expiry. The audience is checked below by hand
	// because more than one value is acceptable and go-oidc compares one.
	verified, err := provider.Verifier(&oidc.Config{
		SkipClientIDCheck:    true,
		SupportedSigningAlgs: mcpOAuthSigningAlgorithms,
	}).Verify(ctx, token)
	if err != nil {
		return auth.Principal{}, &mcpOAuthRefusal{
			status: http.StatusUnauthorized, code: "MCP_OAUTH_TOKEN_REJECTED", token: true,
			message: "The SSO access token is not valid (signature, issuer, or expiry). Sign in again from the client.",
		}
	}
	var claims map[string]any
	if err := verified.Claims(&claims); err != nil {
		return auth.Principal{}, &mcpOAuthRefusal{
			status: http.StatusUnauthorized, code: "MCP_OAUTH_TOKEN_REJECTED", token: true,
			message: "The SSO access token's claims could not be read.",
		}
	}
	if refusal := rejectNonAccessToken(claims); refusal != nil {
		return auth.Principal{}, refusal
	}
	// Whom the token was minted for. A real Keycloak 26 puts the client a
	// token was issued to in azp and only "account" in aud - the client id is
	// not in aud, whatever an ID token does. So the binding is "aud names our
	// resource, or aud/azp is a client the administrator trusts". Either
	// means the token is for this deployment rather than passed through from
	// some other application in the realm, which is what RFC 8707 guards
	// against.
	resource := state.resource(r)
	azp := claimText(claims, "azp")
	accepted := append([]string{resource}, state.Config.Audience...)
	bound := append(slices.Clone(verified.Audience), azp)
	if !slices.ContainsFunc(bound, func(value string) bool {
		return value != "" && slices.Contains(accepted, value)
	}) {
		return auth.Principal{}, &mcpOAuthRefusal{
			status: http.StatusUnauthorized, code: "MCP_OAUTH_AUDIENCE_REJECTED", token: true,
			message: fmt.Sprintf(
				"The SSO token was not issued for this server (aud=%v, azp=%q). Add %q to mcp.oauth.audience, or add an Audience mapper for %q to the Keycloak client.",
				verified.Audience, azp, azp, resource),
		}
	}
	subject := strings.TrimSpace(verified.Subject)
	if subject == "" {
		return auth.Principal{}, &mcpOAuthRefusal{
			status: http.StatusUnauthorized, code: "MCP_OAUTH_TOKEN_REJECTED", token: true,
			message: "The SSO access token has no subject.",
		}
	}
	user, err := s.oidcService.SubjectUser(ctx, state.OIDC, subject)
	switch {
	case errors.Is(err, auth.ErrOIDCUnknownSubject):
		return auth.Principal{}, &mcpOAuthRefusal{
			status: http.StatusUnauthorized, code: "MCP_OAUTH_ACCOUNT_UNKNOWN", token: true,
			message: "This SSO account is not registered here. Sign in to the web console once first, then reconnect.",
		}
	case errors.Is(err, auth.ErrOIDCUserInactive):
		return auth.Principal{}, &mcpOAuthRefusal{
			status: http.StatusUnauthorized, code: "MCP_OAUTH_ACCOUNT_INACTIVE", token: true,
			message: "The account linked to this SSO subject is inactive.",
		}
	case err != nil:
		s.logger.Error("mcp_oauth_subject_lookup_failed",
			"request_id", middleware.GetReqID(ctx), "error", err)
		return auth.Principal{}, &mcpOAuthRefusal{
			status: http.StatusInternalServerError, code: "INTERNAL_ERROR",
			message: "The server could not complete the request.",
		}
	}
	// The same ceiling a key has: the administrator's scopes, and never more
	// than the account itself holds. A super administrator's token is still
	// only these scopes - the token is a tool credential, not a session.
	scopes := make([]string, 0, len(state.Config.Scopes))
	for _, scope := range state.Config.Scopes {
		if user.SuperAdmin || slices.Contains(user.Permissions, scope) {
			scopes = append(scopes, scope)
		}
	}
	return auth.Principal{
		SessionID: "oauth:" + user.ID,
		User: auth.User{
			ID: user.ID, Username: "sso:" + user.Username,
			DisplayName: user.DisplayName, Email: user.Email, Permissions: scopes,
		},
	}, nil
}

// rejectNonAccessToken applies what go-oidc leaves to the caller. An ID token
// is proof of sign-in, not an API credential; a cnf claim binds the token to a
// key this server cannot check (DPoP, mTLS); nbf is simply not verified by the
// library.
func rejectNonAccessToken(claims map[string]any) *mcpOAuthRefusal {
	if strings.EqualFold(claimText(claims, "typ"), "ID") {
		return &mcpOAuthRefusal{
			status: http.StatusUnauthorized, code: "MCP_OAUTH_TOKEN_REJECTED", token: true,
			message: "An ID token was presented. MCP needs the access token issued for this server, not the sign-in token.",
		}
	}
	if _, bound := claims["cnf"]; bound {
		return &mcpOAuthRefusal{
			status: http.StatusUnauthorized, code: "MCP_OAUTH_TOKEN_REJECTED", token: true,
			message: "The token is bound to a proof-of-possession key (cnf), which this server cannot verify.",
		}
	}
	if notBefore, ok := claims["nbf"].(float64); ok &&
		time.Unix(int64(notBefore), 0).After(time.Now().Add(mcpOAuthClockSkew)) {
		return &mcpOAuthRefusal{
			status: http.StatusUnauthorized, code: "MCP_OAUTH_TOKEN_REJECTED", token: true,
			message: "The token is not valid yet (nbf is in the future).",
		}
	}
	return nil
}

func claimText(claims map[string]any, name string) string {
	value, _ := claims[name].(string)
	return strings.TrimSpace(value)
}

func (s *Server) getMCPOAuthSettings(w http.ResponseWriter, r *http.Request) {
	state, err := s.mcpOAuthState(r.Context())
	if err != nil {
		s.internalError(w, r, err)
		return
	}
	writeJSON(w, http.StatusOK, state.public(r))
}

func (s *Server) updateMCPOAuthSettings(w http.ResponseWriter, r *http.Request) {
	var input struct {
		Enabled  *bool     `json:"enabled"`
		Resource *string   `json:"resource"`
		Audience *[]string `json:"audience"`
		Scopes   *[]string `json:"scopes"`
		Reason   string    `json:"reason"`
	}
	if decodeJSON(r, &input) != nil {
		writeAPIError(w, r, http.StatusBadRequest,
			"INVALID_MCP_OAUTH_SETTINGS", "The request body is invalid.")
		return
	}
	before, err := s.mcpOAuthState(r.Context())
	if err != nil {
		s.internalError(w, r, err)
		return
	}
	after := before.Config
	if input.Enabled != nil {
		after.Enabled = *input.Enabled
	}
	if input.Resource != nil {
		after.Resource = *input.Resource
	}
	if input.Audience != nil {
		after.Audience = *input.Audience
	}
	if input.Scopes != nil {
		after.Scopes = *input.Scopes
	}
	after = after.normalize()
	if err := after.validate(); err != nil {
		writeAPIError(w, r, http.StatusBadRequest, "INVALID_MCP_OAUTH_SETTINGS", err.Error())
		return
	}
	// Refuse at save time rather than behave as off afterwards: the
	// administrator is here now and can fix the Keycloak settings first.
	if after.Enabled {
		if s.oidcService == nil {
			writeAPIError(w, r, http.StatusBadRequest, "MCP_OAUTH_OIDC_REQUIRED",
				"Keycloak is not configured on this server.")
			return
		}
		if err := before.OIDC.ValidateIssuer(); err != nil {
			writeAPIError(w, r, http.StatusBadRequest, "MCP_OAUTH_OIDC_REQUIRED",
				"Configure the Keycloak issuer and client ID first: "+err.Error())
			return
		}
	}
	principal := principalFromContext(r.Context())
	if err := s.saveMCPOAuthConfig(r.Context(), principal.User.ID, after); err != nil {
		s.internalError(w, r, err)
		return
	}
	state, err := s.mcpOAuthState(r.Context())
	if err != nil {
		s.internalError(w, r, err)
		return
	}
	s.recordAdminAudit(
		r, "settings.mcp_oauth.update", "setting", mcpOAuthKeyPrefix+"*",
		before.Config.values(), after.values(), input.Reason,
	)
	writeJSON(w, http.StatusOK, state.public(r))
}
