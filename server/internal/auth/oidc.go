package auth

import (
	"context"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"sort"
	"strings"
	"time"

	"github.com/coreos/go-oidc/v3/oidc"
	"github.com/google/uuid"
	"github.com/hkjang/invenqor/server/internal/audit"
	"github.com/hkjang/invenqor/server/internal/bootstrap"
	"golang.org/x/oauth2"
)

var (
	ErrOIDCDisabled     = errors.New("Keycloak login is disabled")
	ErrOIDCFlow         = errors.New("OIDC login flow is invalid or expired")
	ErrOIDCNonce        = errors.New("OIDC nonce validation failed")
	ErrOIDCDomain       = errors.New("OIDC email domain is not allowed")
	ErrOIDCProvisioning = errors.New("OIDC user provisioning is disabled")
	ErrOIDCUsername     = errors.New("OIDC response has no usable username")
	ErrOIDCUserInactive = errors.New("Keycloak-linked user is inactive")
	ErrOIDCSecret       = errors.New("Keycloak client secret is required")
	ErrOIDCRole         = errors.New("Keycloak role mapping references an unknown role")
)

const keycloakSettingKey = "auth.keycloak"

type OIDCSettings struct {
	Enabled              bool              `json:"enabled"`
	IssuerURL            string            `json:"issuer_url"`
	Realm                string            `json:"realm"`
	ClientID             string            `json:"client_id"`
	RedirectURI          string            `json:"redirect_uri"`
	LogoutRedirectURI    string            `json:"logout_redirect_uri"`
	Scopes               []string          `json:"scopes"`
	UsernameClaim        string            `json:"username_claim"`
	EmailClaim           string            `json:"email_claim"`
	NameClaim            string            `json:"name_claim"`
	GroupClaim           string            `json:"group_claim"`
	RoleClaim            string            `json:"role_claim"`
	RoleMappings         map[string]string `json:"role_mappings"`
	GroupMappings        map[string]string `json:"group_mappings"`
	AutoCreateUsers      bool              `json:"auto_create_users"`
	DefaultRole          string            `json:"default_role"`
	AllowedEmailDomains  []string          `json:"allowed_email_domains"`
	PrivateCAPEM         string            `json:"private_ca_pem,omitempty"`
	LastConnectionTestAt *time.Time        `json:"last_connection_test_at,omitempty"`
	LastConnectionOK     bool              `json:"last_connection_ok"`
}

func DefaultOIDCSettings() OIDCSettings {
	return OIDCSettings{
		Scopes:          []string{oidc.ScopeOpenID, "profile", "email"},
		UsernameClaim:   "preferred_username",
		EmailClaim:      "email",
		NameClaim:       "name",
		GroupClaim:      "groups",
		RoleClaim:       "roles",
		RoleMappings:    map[string]string{},
		GroupMappings:   map[string]string{},
		AutoCreateUsers: true,
		DefaultRole:     "viewer",
	}
}

func (settings OIDCSettings) EffectiveIssuer() string {
	issuer := strings.TrimRight(strings.TrimSpace(settings.IssuerURL), "/")
	realm := strings.Trim(strings.TrimSpace(settings.Realm), "/")
	if realm != "" && !strings.Contains(issuer, "/realms/") {
		return issuer + "/realms/" + url.PathEscape(realm)
	}
	return issuer
}

func (settings OIDCSettings) Validate() error {
	if !settings.Enabled {
		return nil
	}
	issuer, err := url.Parse(settings.EffectiveIssuer())
	if err != nil || issuer.Scheme != "https" || issuer.Host == "" {
		return errors.New("Keycloak issuer must be an HTTPS URL")
	}
	if strings.TrimSpace(settings.ClientID) == "" {
		return errors.New("Keycloak client ID is required")
	}
	redirect, err := url.Parse(settings.RedirectURI)
	if err != nil ||
		!containsString([]string{"https", "http"}, redirect.Scheme) ||
		redirect.Host == "" {
		return errors.New("Keycloak redirect URI is invalid")
	}
	if strings.TrimSpace(settings.LogoutRedirectURI) != "" {
		logoutRedirect, logoutErr := url.Parse(settings.LogoutRedirectURI)
		if logoutErr != nil ||
			!containsString([]string{"https", "http"}, logoutRedirect.Scheme) ||
			logoutRedirect.Host == "" {
			return errors.New("Keycloak logout redirect URI is invalid")
		}
	}
	if settings.UsernameClaim == "" {
		return errors.New("Keycloak username claim is required")
	}
	if len(settings.AllowedEmailDomains) > 0 && strings.TrimSpace(settings.EmailClaim) == "" {
		return errors.New("Keycloak email claim is required when domain filtering is enabled")
	}
	if len(settings.Scopes) == 0 || !containsString(settings.Scopes, oidc.ScopeOpenID) {
		return errors.New("Keycloak scopes must include openid")
	}
	for _, domain := range settings.AllowedEmailDomains {
		if strings.ContainsAny(domain, "@/ ") || strings.TrimSpace(domain) == "" {
			return errors.New("allowed email domain is invalid")
		}
	}
	if len(settings.RoleMappings) > 0 && strings.TrimSpace(settings.RoleClaim) == "" {
		return errors.New("Keycloak role claim is required when role mappings are configured")
	}
	if len(settings.GroupMappings) > 0 && strings.TrimSpace(settings.GroupClaim) == "" {
		return errors.New("Keycloak group claim is required when group mappings are configured")
	}
	for external, internal := range settings.RoleMappings {
		if strings.TrimSpace(external) == "" || strings.TrimSpace(internal) == "" {
			return errors.New("Keycloak role mapping contains an empty value")
		}
	}
	for external, internal := range settings.GroupMappings {
		if strings.TrimSpace(external) == "" || strings.TrimSpace(internal) == "" {
			return errors.New("Keycloak group mapping contains an empty value")
		}
	}
	if strings.TrimSpace(settings.PrivateCAPEM) != "" {
		roots := x509.NewCertPool()
		if !roots.AppendCertsFromPEM([]byte(settings.PrivateCAPEM)) {
			return errors.New("Keycloak private CA PEM is invalid")
		}
	}
	return nil
}

type OIDCStart struct {
	AuthorizationURL string `json:"authorization_url"`
}

type OIDCService struct {
	db             *sql.DB
	bootstrapStore *bootstrap.Store
	localAuth      *Service
	audit          audit.Recorder
}

func NewOIDCService(
	db *sql.DB,
	bootstrapStore *bootstrap.Store,
	localAuth *Service,
) *OIDCService {
	return &OIDCService{
		db:             db,
		bootstrapStore: bootstrapStore,
		localAuth:      localAuth,
	}
}

func (service *OIDCService) Settings(ctx context.Context) (OIDCSettings, error) {
	settings := DefaultOIDCSettings()
	var raw any
	if err := service.db.QueryRowContext(
		ctx,
		"SELECT value_json FROM settings WHERE key = $1",
		keycloakSettingKey,
	).Scan(&raw); errors.Is(err, sql.ErrNoRows) {
		return settings, nil
	} else if err != nil {
		return OIDCSettings{}, fmt.Errorf("read Keycloak settings: %w", err)
	}
	bytes, err := jsonBytes(raw)
	if err != nil {
		return OIDCSettings{}, err
	}
	if err := json.Unmarshal(bytes, &settings); err != nil {
		return OIDCSettings{}, fmt.Errorf("decode Keycloak settings: %w", err)
	}
	if settings.RoleMappings == nil {
		settings.RoleMappings = map[string]string{}
	}
	if settings.GroupMappings == nil {
		settings.GroupMappings = map[string]string{}
	}
	return settings, nil
}

func (service *OIDCService) ClientSecretConfigured() (bool, error) {
	values, err := service.bootstrapStore.Load()
	if err != nil {
		return false, err
	}
	return values.KeycloakClientSecret != "", nil
}

func (service *OIDCService) SaveSettings(
	ctx context.Context,
	settings OIDCSettings,
	clientSecret *string,
	actor User,
	reason string,
) error {
	if err := settings.Validate(); err != nil {
		return err
	}
	if err := service.validateRoleMappings(ctx, settings); err != nil {
		return err
	}
	values, err := service.bootstrapStore.Load()
	if err != nil {
		return err
	}
	if settings.Enabled {
		if clientSecret == nil && strings.TrimSpace(values.KeycloakClientSecret) == "" {
			return ErrOIDCSecret
		}
		if clientSecret != nil && strings.TrimSpace(*clientSecret) == "" {
			return ErrOIDCSecret
		}
	}
	if clientSecret != nil {
		values.KeycloakClientSecret = *clientSecret
		if err := service.bootstrapStore.Save(values); err != nil {
			return err
		}
	}
	bytes, err := json.Marshal(settings)
	if err != nil {
		return fmt.Errorf("encode Keycloak settings: %w", err)
	}
	transaction, err := service.db.BeginTx(ctx, nil)
	if err != nil {
		return fmt.Errorf("begin Keycloak settings update: %w", err)
	}
	defer transaction.Rollback()
	var previous any
	_ = transaction.QueryRowContext(
		ctx,
		"SELECT value_json FROM settings WHERE key = $1",
		keycloakSettingKey,
	).Scan(&previous)
	if _, err := transaction.ExecContext(
		ctx,
		`INSERT INTO settings(key, value_json, secret, apply_mode, version, updated_by)
		 VALUES ($1, $2, FALSE, 'new_login', 1, $3)
		 ON CONFLICT (key) DO UPDATE SET
		   value_json = excluded.value_json,
		   version = settings.version + 1,
		   updated_by = excluded.updated_by,
		   updated_at = CURRENT_TIMESTAMP`,
		keycloakSettingKey,
		string(bytes),
		actor.ID,
	); err != nil {
		return fmt.Errorf("save Keycloak settings: %w", err)
	}
	if err := service.audit.Record(ctx, transaction, audit.Entry{
		ActorType:    "user",
		ActorID:      actor.ID,
		ActorName:    actor.Username,
		Action:       "settings.keycloak.update",
		ResourceType: "setting",
		ResourceID:   keycloakSettingKey,
		Result:       "success",
		Reason:       reason,
		Before:       maskKeycloakSettings(previous),
		After:        settings,
		Metadata: map[string]any{
			"client_secret_changed": clientSecret != nil,
		},
	}); err != nil {
		return err
	}
	return transaction.Commit()
}

func (service *OIDCService) TestConnection(
	ctx context.Context,
	settings OIDCSettings,
) error {
	if err := settings.Validate(); err != nil {
		return err
	}
	if err := service.validateRoleMappings(ctx, settings); err != nil {
		return err
	}
	oidcContext, err := oidcHTTPContext(ctx, settings.PrivateCAPEM)
	if err != nil {
		return err
	}
	if _, err := oidc.NewProvider(oidcContext, settings.EffectiveIssuer()); err != nil {
		return fmt.Errorf("discover Keycloak issuer: %w", err)
	}
	return nil
}

func (service *OIDCService) LogoutURL(
	ctx context.Context,
	userID string,
) (string, error) {
	settings, err := service.Settings(ctx)
	if err != nil {
		return "", err
	}
	if strings.TrimSpace(settings.LogoutRedirectURI) == "" ||
		strings.TrimSpace(settings.ClientID) == "" ||
		strings.TrimSpace(settings.EffectiveIssuer()) == "" {
		return "", nil
	}
	var linked int
	if err := service.db.QueryRowContext(
		ctx,
		`SELECT COUNT(*) FROM external_identities
		  WHERE user_id=$1 AND provider='keycloak'`,
		userID,
	).Scan(&linked); err != nil {
		return "", fmt.Errorf("check Keycloak identity for logout: %w", err)
	}
	if linked == 0 {
		return "", nil
	}
	endpoint, err := url.Parse(
		strings.TrimRight(settings.EffectiveIssuer(), "/") +
			"/protocol/openid-connect/logout",
	)
	if err != nil {
		return "", fmt.Errorf("construct Keycloak logout URL: %w", err)
	}
	query := endpoint.Query()
	query.Set("client_id", settings.ClientID)
	query.Set("post_logout_redirect_uri", settings.LogoutRedirectURI)
	endpoint.RawQuery = query.Encode()
	return endpoint.String(), nil
}

func (service *OIDCService) validateRoleMappings(
	ctx context.Context,
	settings OIDCSettings,
) error {
	names := map[string]struct{}{}
	if role := strings.TrimSpace(settings.DefaultRole); role != "" {
		names[role] = struct{}{}
	}
	for _, role := range settings.RoleMappings {
		names[strings.TrimSpace(role)] = struct{}{}
	}
	for _, role := range settings.GroupMappings {
		names[strings.TrimSpace(role)] = struct{}{}
	}
	for name := range names {
		var count int
		if err := service.db.QueryRowContext(
			ctx,
			"SELECT COUNT(*) FROM roles WHERE name=$1",
			name,
		).Scan(&count); err != nil {
			return fmt.Errorf("validate Keycloak role mapping: %w", err)
		}
		if count != 1 {
			return fmt.Errorf("%w: %s", ErrOIDCRole, name)
		}
	}
	return nil
}

func (service *OIDCService) Start(
	ctx context.Context,
	returnTo string,
	sourceIP string,
	userAgent string,
) (OIDCStart, error) {
	settings, _, oauthConfig, err := service.provider(ctx)
	if err != nil {
		return OIDCStart{}, err
	}
	state, _, err := newSecret()
	if err != nil {
		return OIDCStart{}, err
	}
	nonce, _, err := newSecret()
	if err != nil {
		return OIDCStart{}, err
	}
	verifier, _, err := newSecret()
	if err != nil {
		return OIDCStart{}, err
	}
	encryptedVerifier, err := service.bootstrapStore.SealString("oidc.pkce", verifier)
	if err != nil {
		return OIDCStart{}, err
	}
	if !safeReturnTo(returnTo) {
		returnTo = "/"
	}
	expiresAt := time.Now().UTC().Add(10 * time.Minute)
	if _, err := service.db.ExecContext(
		ctx,
		`INSERT INTO oidc_flows(
			id, state_hash, nonce_hash, pkce_verifier, redirect_uri,
			return_to, source_ip, user_agent, expires_at
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`,
		uuid.NewString(),
		hashSecret(state),
		hashSecret(nonce),
		encryptedVerifier,
		settings.RedirectURI,
		returnTo,
		sourceIP,
		userAgent,
		expiresAt,
	); err != nil {
		return OIDCStart{}, fmt.Errorf("store OIDC flow: %w", err)
	}
	challengeHash := sha256.Sum256([]byte(verifier))
	challenge := base64.RawURLEncoding.EncodeToString(challengeHash[:])
	authorizationURL := oauthConfig.AuthCodeURL(
		state,
		oidc.Nonce(nonce),
		oauth2.SetAuthURLParam("code_challenge", challenge),
		oauth2.SetAuthURLParam("code_challenge_method", "S256"),
	)
	return OIDCStart{AuthorizationURL: authorizationURL}, nil
}

func (service *OIDCService) Callback(
	ctx context.Context,
	state string,
	code string,
	sourceIP string,
	userAgent string,
	requestID string,
) (Session, string, error) {
	settings, provider, oauthConfig, err := service.provider(ctx)
	if err != nil {
		return Session{}, "", err
	}
	var flowID, nonceHash, encryptedVerifier, redirectURI, returnTo string
	var expiresAt, consumedAt flexibleTime
	err = service.db.QueryRowContext(
		ctx,
		`SELECT id, nonce_hash, pkce_verifier, redirect_uri, return_to,
		        expires_at, consumed_at
		 FROM oidc_flows
		 WHERE state_hash = $1`,
		hashSecret(state),
	).Scan(
		&flowID,
		&nonceHash,
		&encryptedVerifier,
		&redirectURI,
		&returnTo,
		&expiresAt,
		&consumedAt,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return Session{}, "", ErrOIDCFlow
	}
	if err != nil {
		return Session{}, "", fmt.Errorf("load OIDC flow: %w", err)
	}
	if !expiresAt.Valid || time.Now().UTC().After(expiresAt.Time) || consumedAt.Valid {
		return Session{}, "", ErrOIDCFlow
	}
	verifier, err := service.bootstrapStore.OpenString("oidc.pkce", encryptedVerifier)
	if err != nil {
		return Session{}, "", err
	}
	oidcContext, err := oidcHTTPContext(ctx, settings.PrivateCAPEM)
	if err != nil {
		return Session{}, "", err
	}
	token, err := oauthConfig.Exchange(
		oidcContext,
		code,
		oauth2.SetAuthURLParam("code_verifier", verifier),
		oauth2.SetAuthURLParam("redirect_uri", redirectURI),
	)
	if err != nil {
		return Session{}, "", fmt.Errorf("exchange Keycloak authorization code: %w", err)
	}
	rawIDToken, ok := token.Extra("id_token").(string)
	if !ok {
		return Session{}, "", errors.New("Keycloak token response omitted id_token")
	}
	idToken, err := provider.Verifier(&oidc.Config{
		ClientID: settings.ClientID,
	}).Verify(oidcContext, rawIDToken)
	if err != nil {
		return Session{}, "", fmt.Errorf("verify Keycloak ID token: %w", err)
	}
	var claims map[string]any
	if err := idToken.Claims(&claims); err != nil {
		return Session{}, "", fmt.Errorf("decode Keycloak ID token claims: %w", err)
	}
	nonce, _ := claims["nonce"].(string)
	if nonce == "" || !subtleHashCompare(hashSecret(nonce), nonceHash) {
		return Session{}, "", ErrOIDCNonce
	}
	user, err := service.provisionUser(ctx, settings, idToken.Subject, claims)
	if err != nil {
		return Session{}, "", err
	}
	user.Roles, user.Permissions, err = service.localAuth.rolesAndPermissions(ctx, user.ID)
	if err != nil {
		return Session{}, "", err
	}
	session, err := service.localAuth.createSession(
		ctx,
		user,
		sourceIP,
		userAgent,
		time.Now().UTC(),
	)
	if err != nil {
		return Session{}, "", err
	}
	result, err := service.db.ExecContext(
		ctx,
		`UPDATE oidc_flows
		 SET consumed_at = CURRENT_TIMESTAMP
		 WHERE id = $1 AND consumed_at IS NULL`,
		flowID,
	)
	if err != nil {
		return Session{}, "", fmt.Errorf("consume OIDC flow: %w", err)
	}
	if affected, _ := result.RowsAffected(); affected != 1 {
		return Session{}, "", ErrOIDCFlow
	}
	if err := service.audit.Record(ctx, service.db, audit.Entry{
		ActorType:    "user",
		ActorID:      user.ID,
		ActorName:    user.Username,
		Action:       "auth.keycloak.login",
		ResourceType: "session",
		ResourceID:   session.ID,
		RequestID:    requestID,
		SourceIP:     sourceIP,
		UserAgent:    userAgent,
		Result:       "success",
		Metadata: map[string]any{
			"issuer":  settings.EffectiveIssuer(),
			"subject": idToken.Subject,
		},
	}); err != nil {
		return Session{}, "", err
	}
	return session, returnTo, nil
}

func (service *OIDCService) provider(
	ctx context.Context,
) (OIDCSettings, *oidc.Provider, *oauth2.Config, error) {
	settings, err := service.Settings(ctx)
	if err != nil {
		return OIDCSettings{}, nil, nil, err
	}
	if !settings.Enabled {
		return settings, nil, nil, ErrOIDCDisabled
	}
	if err := settings.Validate(); err != nil {
		return OIDCSettings{}, nil, nil, err
	}
	values, err := service.bootstrapStore.Load()
	if err != nil {
		return OIDCSettings{}, nil, nil, err
	}
	if values.KeycloakClientSecret == "" {
		return OIDCSettings{}, nil, nil, errors.New("Keycloak client secret is not configured")
	}
	oidcContext, err := oidcHTTPContext(ctx, settings.PrivateCAPEM)
	if err != nil {
		return OIDCSettings{}, nil, nil, err
	}
	provider, err := oidc.NewProvider(oidcContext, settings.EffectiveIssuer())
	if err != nil {
		return OIDCSettings{}, nil, nil, fmt.Errorf("discover Keycloak issuer: %w", err)
	}
	oauthConfig := &oauth2.Config{
		ClientID:     settings.ClientID,
		ClientSecret: values.KeycloakClientSecret,
		Endpoint:     provider.Endpoint(),
		RedirectURL:  settings.RedirectURI,
		Scopes:       settings.Scopes,
	}
	return settings, provider, oauthConfig, nil
}

func (service *OIDCService) provisionUser(
	ctx context.Context,
	settings OIDCSettings,
	subject string,
	claims map[string]any,
) (User, error) {
	var user User
	var active, notDeleted bool
	err := service.db.QueryRowContext(
		ctx,
		`SELECT u.id, u.username, u.display_name, u.email, u.super_admin,
		        u.active, CASE WHEN u.deleted_at IS NULL THEN TRUE ELSE FALSE END
		 FROM users u
		 JOIN external_identities e ON e.user_id = u.id
		 WHERE e.provider = 'keycloak' AND e.subject = $1`,
		subject,
	).Scan(
		&user.ID,
		&user.Username,
		&user.DisplayName,
		&user.Email,
		&user.SuperAdmin,
		&active,
		&notDeleted,
	)
	if err == nil {
		if !active || !notDeleted {
			return User{}, ErrOIDCUserInactive
		}
		email := claimString(claims, settings.EmailClaim)
		if !allowedEmail(email, settings.AllowedEmailDomains) {
			return User{}, ErrOIDCDomain
		}
		if email == "" {
			email = user.Email
		}
		displayName := claimString(claims, settings.NameClaim)
		if displayName == "" {
			displayName = user.DisplayName
		}
		claimsJSON, _ := json.Marshal(claims)
		transaction, beginErr := service.db.BeginTx(ctx, nil)
		if beginErr != nil {
			return User{}, fmt.Errorf("begin Keycloak user synchronization: %w", beginErr)
		}
		defer transaction.Rollback()
		if _, updateErr := transaction.ExecContext(
			ctx,
			`UPDATE external_identities
			 SET claims_json = $1, last_login_at = CURRENT_TIMESTAMP
			 WHERE provider = 'keycloak' AND subject = $2`,
			string(claimsJSON),
			subject,
		); updateErr != nil {
			return User{}, fmt.Errorf("update Keycloak identity: %w", updateErr)
		}
		superAdmin, syncErr := replaceKeycloakRoles(
			ctx,
			transaction,
			user.ID,
			mappedRoles(settings, claims),
		)
		if syncErr != nil {
			return User{}, syncErr
		}
		if _, updateErr := transaction.ExecContext(
			ctx,
			`UPDATE users
			    SET display_name=$1,email=$2,super_admin=$3,
			        updated_at=CURRENT_TIMESTAMP
			  WHERE id=$4`,
			displayName,
			email,
			superAdmin,
			user.ID,
		); updateErr != nil {
			return User{}, fmt.Errorf("synchronize Keycloak user profile: %w", updateErr)
		}
		if commitErr := transaction.Commit(); commitErr != nil {
			return User{}, fmt.Errorf("commit Keycloak user synchronization: %w", commitErr)
		}
		user.DisplayName = displayName
		user.Email = email
		user.SuperAdmin = superAdmin
		return user, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return User{}, fmt.Errorf("lookup Keycloak identity: %w", err)
	}
	if !settings.AutoCreateUsers {
		return User{}, ErrOIDCProvisioning
	}
	username := claimString(claims, settings.UsernameClaim)
	username, err = validateUsername(username)
	if err != nil {
		return User{}, ErrOIDCUsername
	}
	email := claimString(claims, settings.EmailClaim)
	if !allowedEmail(email, settings.AllowedEmailDomains) {
		return User{}, ErrOIDCDomain
	}
	displayName := claimString(claims, settings.NameClaim)
	transaction, err := service.db.BeginTx(ctx, nil)
	if err != nil {
		return User{}, fmt.Errorf("begin Keycloak user provisioning: %w", err)
	}
	defer transaction.Rollback()
	user = User{
		ID:          uuid.NewString(),
		Username:    username,
		DisplayName: displayName,
		Email:       email,
	}
	if _, err := transaction.ExecContext(
		ctx,
		`INSERT INTO users(
			id, username, normalized_username, display_name, email,
			active, super_admin
		) VALUES ($1, $2, $3, $4, $5, TRUE, FALSE)`,
		user.ID,
		user.Username,
		normalizeUsername(user.Username),
		user.DisplayName,
		user.Email,
	); err != nil {
		return User{}, fmt.Errorf("create Keycloak user: %w", err)
	}
	claimsJSON, _ := json.Marshal(claims)
	if _, err := transaction.ExecContext(
		ctx,
		`INSERT INTO external_identities(
			id, user_id, provider, subject, claims_json, last_login_at
		) VALUES ($1, $2, 'keycloak', $3, $4, CURRENT_TIMESTAMP)`,
		uuid.NewString(),
		user.ID,
		subject,
		string(claimsJSON),
	); err != nil {
		return User{}, fmt.Errorf("link Keycloak identity: %w", err)
	}
	superAdmin, err := replaceKeycloakRoles(
		ctx,
		transaction,
		user.ID,
		mappedRoles(settings, claims),
	)
	if err != nil {
		return User{}, err
	}
	if _, err := transaction.ExecContext(
		ctx,
		"UPDATE users SET super_admin=$1 WHERE id=$2",
		superAdmin,
		user.ID,
	); err != nil {
		return User{}, fmt.Errorf("synchronize Keycloak super administrator flag: %w", err)
	}
	user.SuperAdmin = superAdmin
	if err := transaction.Commit(); err != nil {
		return User{}, fmt.Errorf("commit Keycloak user provisioning: %w", err)
	}
	return user, nil
}

func replaceKeycloakRoles(
	ctx context.Context,
	transaction *sql.Tx,
	userID string,
	roleNames []string,
) (bool, error) {
	if _, err := transaction.ExecContext(
		ctx,
		"DELETE FROM user_roles WHERE user_id=$1 AND source='keycloak'",
		userID,
	); err != nil {
		return false, fmt.Errorf("clear Keycloak role grants: %w", err)
	}
	for _, roleName := range roleNames {
		result, err := transaction.ExecContext(
			ctx,
			`INSERT INTO user_roles(user_id, role_id, source)
			 SELECT $1,id,'keycloak' FROM roles WHERE name=$2
			 ON CONFLICT (user_id, role_id, source) DO NOTHING`,
			userID,
			roleName,
		)
		if err != nil {
			return false, fmt.Errorf("grant mapped Keycloak role: %w", err)
		}
		if affected, _ := result.RowsAffected(); affected != 1 {
			return false, fmt.Errorf("%w: %s", ErrOIDCRole, roleName)
		}
	}
	var superAdmin bool
	if err := transaction.QueryRowContext(
		ctx,
		`SELECT CASE WHEN EXISTS(
		    SELECT 1 FROM user_roles ur JOIN roles r ON r.id=ur.role_id
		     WHERE ur.user_id=$1 AND r.name='super_admin'
		  ) THEN TRUE ELSE FALSE END`,
		userID,
	).Scan(&superAdmin); err != nil {
		return false, fmt.Errorf("resolve super administrator role: %w", err)
	}
	return superAdmin, nil
}

func oidcHTTPContext(ctx context.Context, privateCAPEM string) (context.Context, error) {
	transport := http.DefaultTransport.(*http.Transport).Clone()
	if strings.TrimSpace(privateCAPEM) != "" {
		roots, err := x509.SystemCertPool()
		if err != nil {
			return nil, fmt.Errorf("load system CA pool: %w", err)
		}
		if !roots.AppendCertsFromPEM([]byte(privateCAPEM)) {
			return nil, errors.New("Keycloak private CA PEM is invalid")
		}
		if transport.TLSClientConfig == nil {
			transport.TLSClientConfig = new(tls.Config)
		} else {
			transport.TLSClientConfig = transport.TLSClientConfig.Clone()
		}
		transport.TLSClientConfig.RootCAs = roots
	}
	client := &http.Client{Transport: transport, Timeout: 15 * time.Second}
	return oidc.ClientContext(ctx, client), nil
}

func jsonBytes(value any) ([]byte, error) {
	switch typed := value.(type) {
	case []byte:
		return typed, nil
	case string:
		return []byte(typed), nil
	default:
		return nil, fmt.Errorf("unsupported JSON database type %T", value)
	}
}

func maskKeycloakSettings(value any) any {
	if value == nil {
		return nil
	}
	bytes, err := jsonBytes(value)
	if err != nil {
		return map[string]any{"masked": true}
	}
	var decoded map[string]any
	if json.Unmarshal(bytes, &decoded) != nil {
		return map[string]any{"masked": true}
	}
	delete(decoded, "private_ca_pem")
	return decoded
}

func claimString(claims map[string]any, name string) string {
	value, _ := claimValue(claims, name).(string)
	return strings.TrimSpace(value)
}

func claimStrings(claims map[string]any, name string) []string {
	switch value := claimValue(claims, name).(type) {
	case string:
		return []string{value}
	case []any:
		var result []string
		for _, item := range value {
			if text, ok := item.(string); ok {
				result = append(result, text)
			}
		}
		return result
	case []string:
		return value
	default:
		return nil
	}
}

func claimValue(claims map[string]any, name string) any {
	var value any = claims
	for _, component := range strings.Split(name, ".") {
		current, ok := value.(map[string]any)
		if !ok {
			return nil
		}
		value, ok = current[component]
		if !ok {
			return nil
		}
	}
	return value
}

func mappedRoles(settings OIDCSettings, claims map[string]any) []string {
	unique := map[string]struct{}{}
	for _, external := range claimStrings(claims, settings.RoleClaim) {
		if internal := settings.RoleMappings[external]; internal != "" {
			unique[internal] = struct{}{}
		}
	}
	for _, external := range claimStrings(claims, settings.GroupClaim) {
		if internal := settings.GroupMappings[external]; internal != "" {
			unique[internal] = struct{}{}
		}
	}
	if len(unique) == 0 && settings.DefaultRole != "" {
		unique[settings.DefaultRole] = struct{}{}
	}
	result := make([]string, 0, len(unique))
	for role := range unique {
		result = append(result, role)
	}
	sort.Strings(result)
	return result
}

func allowedEmail(email string, domains []string) bool {
	if len(domains) == 0 {
		return true
	}
	_, domain, found := strings.Cut(strings.ToLower(email), "@")
	if !found {
		return false
	}
	for _, allowed := range domains {
		if domain == strings.ToLower(strings.TrimSpace(allowed)) {
			return true
		}
	}
	return false
}

func safeReturnTo(value string) bool {
	return strings.HasPrefix(value, "/") && !strings.HasPrefix(value, "//")
}

func containsString(values []string, target string) bool {
	for _, value := range values {
		if value == target {
			return true
		}
	}
	return false
}

func subtleHashCompare(left, right string) bool {
	if len(left) != len(right) {
		return false
	}
	var result byte
	for index := range left {
		result |= left[index] ^ right[index]
	}
	return result == 0
}
