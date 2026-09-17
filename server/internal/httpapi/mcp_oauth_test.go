package httpapi

import (
	"crypto"
	"crypto/hmac"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"math/big"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/hkjang/invenqor/server/internal/storage"
)

// fakeIdP is a Keycloak stand-in: discovery, a JWKS with one RSA key, and the
// private half to mint tokens with. It answers over TLS because the issuer
// policy insists on HTTPS.
type fakeIdP struct {
	*httptest.Server
	issuer string
	caPEM  string
	key    *rsa.PrivateKey
}

func newFakeIdP(t *testing.T) *fakeIdP {
	t.Helper()
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	idp := &fakeIdP{key: key}
	idp.Server = httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/realms/inventory/.well-known/openid-configuration":
			w.Header().Set("Content-Type", "application/json")
			_ = json.NewEncoder(w).Encode(map[string]any{
				"issuer":                                idp.issuer,
				"authorization_endpoint":                idp.issuer + "/protocol/openid-connect/auth",
				"token_endpoint":                        idp.issuer + "/protocol/openid-connect/token",
				"jwks_uri":                              idp.issuer + "/protocol/openid-connect/certs",
				"response_types_supported":              []string{"code"},
				"subject_types_supported":               []string{"public"},
				"id_token_signing_alg_values_supported": []string{"RS256"},
				"code_challenge_methods_supported":      []string{"S256"},
			})
		case "/realms/inventory/protocol/openid-connect/certs":
			w.Header().Set("Content-Type", "application/json")
			_ = json.NewEncoder(w).Encode(map[string]any{
				"keys": []map[string]string{{
					"kty": "RSA", "kid": "realm-key", "use": "sig", "alg": "RS256",
					"n": base64.RawURLEncoding.EncodeToString(key.PublicKey.N.Bytes()),
					"e": base64.RawURLEncoding.EncodeToString(big.NewInt(int64(key.PublicKey.E)).Bytes()),
				}},
			})
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(idp.Close)
	idp.issuer = idp.URL + "/realms/inventory"
	idp.caPEM = string(pem.EncodeToMemory(&pem.Block{
		Type: "CERTIFICATE", Bytes: idp.Certificate().Raw,
	}))
	return idp
}

// accessToken mints what Keycloak 26 mints for a public MCP client: aud is
// only "account", the client sits in azp, typ is Bearer. Overrides adjust or
// remove (nil) claims.
func (idp *fakeIdP) accessToken(subject string, overrides map[string]any) string {
	now := time.Now().UTC()
	claims := map[string]any{
		"iss": idp.issuer, "sub": subject, "aud": []string{"account"}, "azp": "claude-mcp",
		"typ": "Bearer", "iat": now.Unix(), "exp": now.Add(5 * time.Minute).Unix(),
		"preferred_username": "sso.person", "scope": "openid profile",
	}
	for name, value := range overrides {
		if value == nil {
			delete(claims, name)
		} else {
			claims[name] = value
		}
	}
	return idp.sign(map[string]any{"alg": "RS256", "kid": "realm-key", "typ": "JWT"}, claims)
}

func (idp *fakeIdP) sign(header map[string]any, claims map[string]any) string {
	encodedHeader, _ := json.Marshal(header)
	payload, _ := json.Marshal(claims)
	signingInput := base64.RawURLEncoding.EncodeToString(encodedHeader) + "." +
		base64.RawURLEncoding.EncodeToString(payload)
	digest := sha256.Sum256([]byte(signingInput))
	signature, err := rsa.SignPKCS1v15(rand.Reader, idp.key, crypto.SHA256, digest[:])
	if err != nil {
		panic(err)
	}
	return signingInput + "." + base64.RawURLEncoding.EncodeToString(signature)
}

// hmacToken is the classic algorithm-confusion attempt: HS256 with a secret
// nobody here shares. It must be refused on the algorithm alone.
func hmacToken(claims map[string]any) string {
	encodedHeader, _ := json.Marshal(map[string]string{"alg": "HS256", "typ": "JWT"})
	payload, _ := json.Marshal(claims)
	signingInput := base64.RawURLEncoding.EncodeToString(encodedHeader) + "." +
		base64.RawURLEncoding.EncodeToString(payload)
	mac := hmac.New(sha256.New, []byte("guess"))
	mac.Write([]byte(signingInput))
	return signingInput + "." + base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
}

const testResource = "https://invenqor.example.test/mcp"

func configureKeycloakWith(t *testing.T, server *Server, idp *fakeIdP, cookie *http.Cookie, csrf string) {
	t.Helper()
	response := performAuthenticatedJSON(
		t, server, http.MethodPost, "/api/v1/admin/settings/keycloak/auto-configure",
		map[string]any{
			"keycloak_url": idp.URL, "realm": "inventory", "client_id": "invenqor-web",
			"client_secret": "minimum-secret", "application_url": "https://invenqor.example.test",
			"private_ca_pem": idp.caPEM, "reason": "mcp oauth test",
		}, cookie, csrf,
	)
	if response.Code != http.StatusOK {
		t.Fatalf("auto configure status = %d body = %s", response.Code, response.Body.String())
	}
}

func patchMCPOAuth(t *testing.T, server *Server, cookie *http.Cookie, csrf string, body map[string]any) *httptest.ResponseRecorder {
	t.Helper()
	return performAuthenticatedJSON(t, server, http.MethodPatch, "/api/v1/admin/settings/mcp-oauth", body, cookie, csrf)
}

// linkSubject is what signing in to the console once leaves behind: the row
// that ties a Keycloak subject to an account. The tests create it directly
// because the code path under test must never create it itself.
func linkSubject(t *testing.T, runtime *storage.Runtime, issuer string, subject string, userID string) {
	t.Helper()
	if _, err := runtime.DB().Exec(
		`INSERT INTO external_identities(id, user_id, provider, issuer, subject, claims_json)
		 VALUES($1, $2, 'keycloak', $3, $4, '{}')`,
		uuid.NewString(), userID, issuer, subject,
	); err != nil {
		t.Fatalf("link subject: %v", err)
	}
}

func userIDByUsername(t *testing.T, runtime *storage.Runtime, username string) string {
	t.Helper()
	var id string
	if err := runtime.DB().QueryRow(`SELECT id FROM users WHERE username=$1`, username).Scan(&id); err != nil {
		t.Fatalf("user %q: %v", username, err)
	}
	return id
}

func countRows(t *testing.T, runtime *storage.Runtime, table string) int {
	t.Helper()
	var count int
	if err := runtime.DB().QueryRow(`SELECT COUNT(*) FROM ` + table).Scan(&count); err != nil {
		t.Fatal(err)
	}
	return count
}

func mcpToolsList(t *testing.T, server *Server, bearer string) *httptest.ResponseRecorder {
	t.Helper()
	return performMCPRequest(t, server, bearer, map[string]any{
		"jsonrpc": "2.0", "id": 1, "method": "tools/list",
		"params": map[string]any{"_meta": modernMCPMetadata()},
	}, map[string]string{
		"MCP-Protocol-Version": mcpModernProtocolVersion, "Mcp-Method": "tools/list",
	})
}

func toolNames(t *testing.T, response *httptest.ResponseRecorder) []string {
	t.Helper()
	tools, _ := decodeMCPResult(t, response)["tools"].([]any)
	var names []string
	for _, item := range tools {
		names = append(names, item.(map[string]any)["name"].(string))
	}
	return names
}

// Off is the default, and off has to be invisible: no metadata, and a token
// is refused with exactly the words a bad key gets, so an installation that
// never turned this on says nothing new to anybody probing it.
func TestMCPOAuthIsOffByDefaultAndLeavesTheKeyPathUntouched(t *testing.T) {
	runtime := newRuntime(t)
	defer runtime.Close()
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	idp := newFakeIdP(t)

	for _, path := range []string{protectedResourcePath, protectedResourceMCPPath} {
		if response := getJSON(t, server, path, nil); response.Code != http.StatusNotFound ||
			errorCode(t, response) != "MCP_OAUTH_DISABLED" {
			t.Fatalf("%s while off = %d %s", path, response.Code, response.Body)
		}
	}
	settings := performAuthenticatedJSON(t, server, http.MethodGet, "/api/v1/admin/settings/mcp-oauth", nil, cookie, csrf)
	var current map[string]any
	if err := json.Unmarshal(settings.Body.Bytes(), &current); err != nil {
		t.Fatal(err)
	}
	if current["enabled"] != false || current["active"] != false {
		t.Fatalf("fresh install settings = %s", settings.Body)
	}
	if got := countRows(t, runtime, "settings WHERE key LIKE 'mcp.oauth.%'"); got != 0 {
		t.Fatalf("a read wrote %d mcp.oauth rows", got)
	}

	refused := mcpToolsList(t, server, idp.accessToken("subject-1", nil))
	if refused.Code != http.StatusUnauthorized || errorCode(t, refused) != "INVALID_API_KEY" {
		t.Fatalf("token while off = %d %s", refused.Code, refused.Body)
	}
	if challenge := refused.Header().Get("WWW-Authenticate"); challenge != `Bearer realm="invenqor-api"` {
		t.Fatalf("challenge while off = %q", challenge)
	}

	// Turning it on needs a Keycloak issuer; a half-configured switch is
	// refused now rather than behaving as off later.
	if response := patchMCPOAuth(t, server, cookie, csrf, map[string]any{"enabled": true}); response.Code != http.StatusBadRequest ||
		errorCode(t, response) != "MCP_OAUTH_OIDC_REQUIRED" {
		t.Fatalf("enable without Keycloak = %d %s", response.Code, response.Body)
	}
	for name, body := range map[string]map[string]any{
		"unknown scope":     {"scopes": []string{"assets.read", "everything"}},
		"relative resource": {"resource": "/mcp"},
		"resource w/ query": {"resource": "https://invenqor.example.test/mcp?x=1"},
	} {
		if response := patchMCPOAuth(t, server, cookie, csrf, body); response.Code != http.StatusBadRequest ||
			errorCode(t, response) != "INVALID_MCP_OAUTH_SETTINGS" {
			t.Fatalf("%s = %d %s", name, response.Code, response.Body)
		}
	}

	// A key on /mcp is exactly as before.
	secret := createAPIKeySecret(t, server, cookie, csrf, "still-works", "mcp.access", "assets.read")
	if listed := mcpToolsList(t, server, secret); listed.Code != http.StatusOK {
		t.Fatalf("key on /mcp = %d %s", listed.Code, listed.Body)
	}
}

func TestMCPOAuthPublishesMetadataAndPointsRefusedClientsAtIt(t *testing.T) {
	runtime := newRuntime(t)
	defer runtime.Close()
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	idp := newFakeIdP(t)
	configureKeycloakWith(t, server, idp, cookie, csrf)

	enabled := patchMCPOAuth(t, server, cookie, csrf, map[string]any{
		"enabled": true, "audience": []string{"claude-mcp"}, "reason": "open sso for mcp",
	})
	if enabled.Code != http.StatusOK {
		t.Fatalf("enable = %d %s", enabled.Code, enabled.Body)
	}
	var settings map[string]any
	if err := json.Unmarshal(enabled.Body.Bytes(), &settings); err != nil {
		t.Fatal(err)
	}
	// The resource comes from the public address the Keycloak callback was
	// configured with, not from whatever Host the request carried.
	if settings["active"] != true || settings["effective_resource"] != testResource ||
		settings["metadata_url"] != "https://invenqor.example.test/.well-known/oauth-protected-resource/mcp" ||
		settings["oidc_issuer"] != idp.issuer {
		t.Fatalf("settings after enabling = %s", enabled.Body)
	}

	for _, path := range []string{protectedResourcePath, protectedResourceMCPPath} {
		response := getJSON(t, server, path, nil)
		if response.Code != http.StatusOK {
			t.Fatalf("%s = %d %s", path, response.Code, response.Body)
		}
		if response.Header().Get("Access-Control-Allow-Origin") != "*" {
			t.Fatalf("%s is not readable from a browser client", path)
		}
		var document map[string]any
		if err := json.Unmarshal(response.Body.Bytes(), &document); err != nil {
			t.Fatal(err)
		}
		if _, enveloped := document["error"]; enveloped {
			t.Fatalf("%s used the API envelope: %s", path, response.Body)
		}
		servers, _ := document["authorization_servers"].([]any)
		if document["resource"] != testResource || len(servers) != 1 || servers[0] != idp.issuer {
			t.Fatalf("%s document = %s", path, response.Body)
		}
		methods, _ := document["bearer_methods_supported"].([]any)
		scopes, _ := document["scopes_supported"].([]any)
		if len(methods) != 1 || methods[0] != "header" || len(scopes) != 4 {
			t.Fatalf("%s document = %s", path, response.Body)
		}
	}

	// No token on /mcp: the 401 says where to go. The same 401 on REST does
	// not, because a browser or a REST client would go there and be lost.
	noToken := mcpToolsList(t, server, "")
	challenge := noToken.Header().Get("WWW-Authenticate")
	if noToken.Code != http.StatusUnauthorized ||
		!strings.Contains(challenge, `resource_metadata="https://invenqor.example.test/.well-known/oauth-protected-resource/mcp"`) ||
		strings.Contains(challenge, "invalid_token") {
		t.Fatalf("no-token 401 on /mcp: status %d challenge %q", noToken.Code, challenge)
	}
	rest := performWithAPIKey(t, server, http.MethodGet, "/api/v1/external/assets", nil, "")
	if rest.Code != http.StatusUnauthorized || rest.Header().Get("WWW-Authenticate") != `Bearer realm="invenqor-api"` {
		t.Fatalf("REST 401 = %d %q", rest.Code, rest.Header().Get("WWW-Authenticate"))
	}

	// An explicit resource wins over the derived one.
	explicit := patchMCPOAuth(t, server, cookie, csrf, map[string]any{"resource": "https://mcp.example.test/mcp"})
	if explicit.Code != http.StatusOK {
		t.Fatalf("set resource = %d %s", explicit.Code, explicit.Body)
	}
	document := getJSON(t, server, protectedResourceMCPPath, nil)
	if !strings.Contains(document.Body.String(), `"resource":"https://mcp.example.test/mcp"`) {
		t.Fatalf("explicit resource not advertised: %s", document.Body)
	}
	var audit int
	if err := runtime.DB().QueryRow(
		`SELECT COUNT(*) FROM audit_logs WHERE action='settings.mcp_oauth.update'`,
	).Scan(&audit); err != nil || audit != 2 {
		t.Fatalf("audit entries = %d err = %v", audit, err)
	}
}

func TestMCPOAuthAcceptsOnlyTokensMintedForThisServerAndKnownAccounts(t *testing.T) {
	runtime := newRuntime(t)
	defer runtime.Close()
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	idp := newFakeIdP(t)
	configureKeycloakWith(t, server, idp, cookie, csrf)
	if response := patchMCPOAuth(t, server, cookie, csrf, map[string]any{
		"enabled": true, "audience": []string{"claude-mcp"},
		"scopes": []string{"mcp.access", "assets.read"},
	}); response.Code != http.StatusOK {
		t.Fatalf("enable = %d %s", response.Code, response.Body)
	}
	adminID := userIDByUsername(t, runtime, "admin.user")
	linkSubject(t, runtime, idp.issuer, "admin-subject", adminID)
	usersBefore := countRows(t, runtime, "users")

	// The compatibility path: aud is only "account", azp names the client
	// the administrator listed.
	listed := mcpToolsList(t, server, idp.accessToken("admin-subject", nil))
	if listed.Code != http.StatusOK {
		t.Fatalf("token via azp = %d %s", listed.Code, listed.Body)
	}
	// A super administrator's token is still held to the configured scopes:
	// asset tools only, no agents_list and no asset_relations.
	if names := toolNames(t, listed); len(names) != 3 || contains(names, "agents_list") || contains(names, "asset_relations") {
		t.Fatalf("tools for an sso subject = %v", names)
	}
	// The formal path: an Audience mapper put the resource in aud.
	mapped := mcpToolsList(t, server, idp.accessToken("admin-subject", map[string]any{
		"aud": []string{"account", testResource}, "azp": "some-other-client",
	}))
	if mapped.Code != http.StatusOK {
		t.Fatalf("token via aud = %d %s", mapped.Code, mapped.Body)
	}
	// A tool call is recorded against the SSO subject, not against a key.
	called := performMCPRequest(t, server, idp.accessToken("admin-subject", nil), map[string]any{
		"jsonrpc": "2.0", "id": 3, "method": "tools/call",
		"params": map[string]any{
			"name": "asset_search", "arguments": map[string]any{"limit": 1}, "_meta": modernMCPMetadata(),
		},
	}, map[string]string{
		"MCP-Protocol-Version": mcpModernProtocolVersion, "Mcp-Method": "tools/call",
		"Mcp-Name": "=?base64?YXNzZXRfc2VhcmNo?=",
	})
	if called.Code != http.StatusOK {
		t.Fatalf("tools/call with a token = %d %s", called.Code, called.Body)
	}

	// The token never opens anything but /mcp.
	rest := performWithAPIKey(t, server, http.MethodGet, "/api/v1/external/assets", nil, idp.accessToken("admin-subject", nil))
	if rest.Code != http.StatusUnauthorized || errorCode(t, rest) != "INVALID_API_KEY" {
		t.Fatalf("token on REST = %d %s", rest.Code, rest.Body)
	}

	// Another application's token: refused, and the message says what was
	// seen and what to write.
	foreign := mcpToolsList(t, server, idp.accessToken("admin-subject", map[string]any{"azp": "payroll-app"}))
	if foreign.Code != http.StatusUnauthorized || errorCode(t, foreign) != "MCP_OAUTH_AUDIENCE_REJECTED" {
		t.Fatalf("foreign token = %d %s", foreign.Code, foreign.Body)
	}
	body := foreign.Body.String()
	if !strings.Contains(body, `aud=[account]`) || !strings.Contains(body, `azp=\"payroll-app\"`) ||
		!strings.Contains(body, "mcp.oauth.audience") || !strings.Contains(body, testResource) {
		t.Fatalf("audience refusal does not tell the operator what to fix: %s", body)
	}
	if challenge := foreign.Header().Get("WWW-Authenticate"); !strings.Contains(challenge, `error="invalid_token"`) ||
		!strings.Contains(challenge, "resource_metadata=") {
		t.Fatalf("refused-token challenge = %q", challenge)
	}

	now := time.Now().UTC()
	for name, token := range map[string]string{
		"expired":        idp.accessToken("admin-subject", map[string]any{"exp": now.Add(-time.Minute).Unix()}),
		"not yet valid":  idp.accessToken("admin-subject", map[string]any{"nbf": now.Add(10 * time.Minute).Unix()}),
		"other issuer":   idp.accessToken("admin-subject", map[string]any{"iss": "https://sso.elsewhere.test/realms/x"}),
		"id token":       idp.accessToken("admin-subject", map[string]any{"typ": "ID"}),
		"bound (cnf)":    idp.accessToken("admin-subject", map[string]any{"cnf": map[string]any{"jkt": "abc"}}),
		"no subject":     idp.accessToken("admin-subject", map[string]any{"sub": nil}),
		"hmac signature": hmacToken(map[string]any{"iss": idp.issuer, "sub": "admin-subject", "azp": "claude-mcp", "exp": now.Add(time.Minute).Unix()}),
		"garbage":        "eyJhbGciOiJSUzI1NiJ9.not-json.signature",
	} {
		response := mcpToolsList(t, server, token)
		if response.Code != http.StatusUnauthorized || errorCode(t, response) != "MCP_OAUTH_TOKEN_REJECTED" {
			t.Fatalf("%s token = %d %s", name, response.Code, response.Body)
		}
	}
	idToken := mcpToolsList(t, server, idp.accessToken("admin-subject", map[string]any{"typ": "ID"}))
	if !strings.Contains(idToken.Body.String(), "ID token") {
		t.Fatalf("an ID token should be named as such: %s", idToken.Body)
	}

	// Somebody who never signed in to the console: refused and told to, and
	// no account appears.
	unknown := mcpToolsList(t, server, idp.accessToken("never-seen", nil))
	if unknown.Code != http.StatusUnauthorized || errorCode(t, unknown) != "MCP_OAUTH_ACCOUNT_UNKNOWN" ||
		!strings.Contains(unknown.Body.String(), "web console") {
		t.Fatalf("unknown subject = %d %s", unknown.Code, unknown.Body)
	}
	if countRows(t, runtime, "users") != usersBefore {
		t.Fatal("a token created an account")
	}

	// A viewer who did sign in walks through the same door a key would: the
	// account holds assets.read but not mcp.access, so the token cannot be
	// given more than a key issued to that viewer could carry.
	created := performAuthenticatedJSON(t, server, http.MethodPost, "/api/v1/admin/users", map[string]any{
		"username": "viewer.person", "display_name": "Viewer", "email": "viewer@example.test",
		"password": "CorrectHorse!42", "roles": []string{"viewer"},
	}, cookie, csrf)
	if created.Code != http.StatusCreated {
		t.Fatalf("create viewer = %d %s", created.Code, created.Body)
	}
	viewerID := userIDByUsername(t, runtime, "viewer.person")
	linkSubject(t, runtime, idp.issuer, "viewer-subject", viewerID)
	viewer := mcpToolsList(t, server, idp.accessToken("viewer-subject", nil))
	if viewer.Code != http.StatusForbidden || errorCode(t, viewer) != "FORBIDDEN" {
		t.Fatalf("viewer without mcp.access = %d %s", viewer.Code, viewer.Body)
	}
	// Deactivated accounts stay closed; the token does not revive them.
	if _, err := runtime.DB().Exec(`UPDATE users SET active=FALSE WHERE id=$1`, viewerID); err != nil {
		t.Fatal(err)
	}
	inactive := mcpToolsList(t, server, idp.accessToken("viewer-subject", nil))
	if inactive.Code != http.StatusUnauthorized || errorCode(t, inactive) != "MCP_OAUTH_ACCOUNT_INACTIVE" {
		t.Fatalf("inactive account = %d %s", inactive.Code, inactive.Body)
	}

	// Switching it off closes the door on the next request and hides the
	// metadata again, without touching the key path.
	if response := patchMCPOAuth(t, server, cookie, csrf, map[string]any{"enabled": false}); response.Code != http.StatusOK {
		t.Fatalf("disable = %d %s", response.Code, response.Body)
	}
	if off := mcpToolsList(t, server, idp.accessToken("admin-subject", nil)); off.Code != http.StatusUnauthorized ||
		errorCode(t, off) != "INVALID_API_KEY" {
		t.Fatalf("token after disabling = %d %s", off.Code, off.Body)
	}
	if response := getJSON(t, server, protectedResourceMCPPath, nil); response.Code != http.StatusNotFound {
		t.Fatalf("metadata after disabling = %d", response.Code)
	}
}

// An unreachable issuer is an outage, not a bad credential: the client must
// not be sent back through a login it cannot complete.
func TestMCPOAuthReportsAnUnreachableIssuerAsUnavailable(t *testing.T) {
	runtime := newRuntime(t)
	defer runtime.Close()
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	idp := newFakeIdP(t)
	configureKeycloakWith(t, server, idp, cookie, csrf)
	if response := patchMCPOAuth(t, server, cookie, csrf, map[string]any{"enabled": true}); response.Code != http.StatusOK {
		t.Fatalf("enable = %d %s", response.Code, response.Body)
	}
	token := idp.accessToken("admin-subject", nil)
	idp.Close()

	response := mcpToolsList(t, server, token)
	if response.Code != http.StatusServiceUnavailable || errorCode(t, response) != "MCP_OAUTH_ISSUER_UNREACHABLE" {
		t.Fatalf("unreachable issuer = %d %s", response.Code, response.Body)
	}
	if response.Header().Get("WWW-Authenticate") != "" {
		t.Fatal("a 503 must not carry a challenge")
	}
}
