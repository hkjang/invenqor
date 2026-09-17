package auth

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"

	"github.com/coreos/go-oidc/v3/oidc"
)

// ErrOIDCUnknownSubject says the Keycloak subject has never signed in to the
// console. An access token is not the moment to decide who somebody is, so
// the caller refuses rather than provisioning.
var ErrOIDCUnknownSubject = errors.New("no account is linked to this Keycloak subject")

// ValidateIssuer reports whether the stored Keycloak settings name a usable
// issuer, whether or not web sign-in is enabled. MCP token verification needs
// only discovery and the key set behind it, not the client secret.
func (settings OIDCSettings) ValidateIssuer() error {
	return settings.validateIssuer()
}

// AccessTokenProvider returns discovery for the configured issuer, cached per
// issuer and private CA. Discovery is a round trip to Keycloak and the JWKS
// behind it verifies every MCP call; doing that per request would put
// Keycloak's latency in front of every tool call. go-oidc refetches the key
// set on an unknown key id, so key rotation needs no invalidation here. The
// provider outlives the request that created it, which is why the context is
// detached from the request's cancellation.
func (service *OIDCService) AccessTokenProvider(
	ctx context.Context,
	settings OIDCSettings,
) (*oidc.Provider, error) {
	if err := settings.validateIssuer(); err != nil {
		return nil, err
	}
	issuer := normalizedOIDCIssuer(settings.EffectiveIssuer())
	caDigest := sha256.Sum256([]byte(settings.PrivateCAPEM))
	cacheKey := issuer + "\n" + hex.EncodeToString(caDigest[:])
	service.providerMutex.Lock()
	defer service.providerMutex.Unlock()
	if provider := service.providers[cacheKey]; provider != nil {
		return provider, nil
	}
	oidcContext, err := oidcHTTPContext(context.WithoutCancel(ctx), settings.PrivateCAPEM)
	if err != nil {
		return nil, err
	}
	provider, err := oidc.NewProvider(oidcContext, issuer)
	if err != nil {
		return nil, fmt.Errorf("%w: %s", ErrOIDCUnreachable, err.Error())
	}
	if service.providers == nil {
		service.providers = map[string]*oidc.Provider{}
	}
	service.providers[cacheKey] = provider
	return provider, nil
}

// SubjectUser finds the account a Keycloak subject was linked to when that
// person signed in to the console, with the roles and permissions the console
// would give them. It is the web sign-in's lookup without the provisioning
// half: an unknown subject is refused, an inactive or deleted account stays
// closed, and nothing in the token changes what the account may do.
func (service *OIDCService) SubjectUser(
	ctx context.Context,
	settings OIDCSettings,
	subject string,
) (User, error) {
	issuer := normalizedOIDCIssuer(settings.EffectiveIssuer())
	var user User
	var active, notDeleted bool
	err := service.db.QueryRowContext(
		ctx,
		`SELECT u.id, u.username, u.display_name, u.email, u.super_admin,
		        u.active, CASE WHEN u.deleted_at IS NULL THEN TRUE ELSE FALSE END
		 FROM users u
		 JOIN external_identities e ON e.user_id = u.id
		 WHERE e.provider = 'keycloak' AND e.issuer = $1 AND e.subject = $2`,
		issuer,
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
	if errors.Is(err, sql.ErrNoRows) {
		return User{}, ErrOIDCUnknownSubject
	}
	if err != nil {
		return User{}, fmt.Errorf("look up Keycloak subject: %w", err)
	}
	if !active || !notDeleted {
		return User{}, ErrOIDCUserInactive
	}
	user.Roles, user.Permissions, err = service.localAuth.rolesAndPermissions(ctx, user.ID)
	if err != nil {
		return User{}, err
	}
	return user, nil
}
