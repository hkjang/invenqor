package httpapi

import (
	"context"
	"errors"
	"net/http"

	"github.com/hkjang/invenqor/server/internal/apikeys"
	"github.com/hkjang/invenqor/server/internal/auth"
)

func (s *Server) authenticateAPIKey(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		token := bearerToken(r)
		credential, err := s.apiKeyService.Authenticate(r.Context(), token)
		if errors.Is(err, apikeys.ErrUnauthorized) {
			// One header, two kinds of credential. A key is refused exactly
			// as before; a JWT on /mcp is tried as an SSO access token when
			// the administrator has turned that on. Anywhere else, and in
			// an installation where it is off, a token is just a bad key.
			state, stateErr := s.mcpOAuthState(r.Context())
			if stateErr != nil {
				s.internalError(w, r, stateErr)
				return
			}
			if r.URL.Path == mcpPath && state.Active && looksLikeJWT(token) {
				s.authenticateMCPOAuth(w, r, state, token, next)
				return
			}
			s.mcpChallenge(w, r, state, false)
			writeAPIError(w, r, 401, "INVALID_API_KEY", "The API key is invalid, expired, or revoked.")
			return
		}
		if err != nil {
			s.internalError(w, r, err)
			return
		}
		if !s.apiRateLimit.Allow(credential.KeyID) {
			w.Header().Set("Retry-After", "60")
			writeAPIError(w, r, 429, "API_RATE_LIMITED", "The API key rate limit was exceeded.")
			return
		}
		principal := auth.Principal{
			SessionID: "api_key:" + credential.KeyID,
			User: auth.User{
				ID: credential.UserID, Username: "api-key:" + credential.Name,
				DisplayName: credential.Name, Permissions: credential.Scopes,
			},
		}
		ctx := context.WithValue(r.Context(), principalContextKey{}, principal)
		ctx = context.WithValue(ctx, apiKeyContextKey{}, credential)
		w.Header().Set("Cache-Control", "no-store")
		next.ServeHTTP(w, r.WithContext(ctx))
	})
}

// authenticateMCPOAuth is the token half of authenticateAPIKey. The principal
// it builds walks through the same permission checks a key does; the only
// difference downstream is that no API-key credential is in the context.
func (s *Server) authenticateMCPOAuth(
	w http.ResponseWriter, r *http.Request, state mcpOAuthState, token string, next http.Handler,
) {
	principal, refusal := s.oauthPrincipal(r, state, token)
	if refusal != nil {
		if refusal.status == http.StatusUnauthorized {
			s.mcpChallenge(w, r, state, refusal.token)
		}
		writeAPIError(w, r, refusal.status, refusal.code, refusal.message)
		return
	}
	if !s.apiRateLimit.Allow(principal.SessionID) {
		w.Header().Set("Retry-After", "60")
		writeAPIError(w, r, 429, "API_RATE_LIMITED", "The API rate limit was exceeded.")
		return
	}
	ctx := context.WithValue(r.Context(), principalContextKey{}, principal)
	w.Header().Set("Cache-Control", "no-store")
	next.ServeHTTP(w, r.WithContext(ctx))
}

type apiKeyContextKey struct{}

func apiKeyFromContext(ctx context.Context) apikeys.Credential {
	credential, _ := ctx.Value(apiKeyContextKey{}).(apikeys.Credential)
	return credential
}
