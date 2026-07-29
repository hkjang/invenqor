package httpapi

import (
	"errors"
	"net/http"
	"strings"
	"time"

	"github.com/go-chi/chi/v5"
	"github.com/hkjang/invenqor/server/internal/apikeys"
)

func (s *Server) apiKeyScopes(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, 200, map[string]any{"scopes": apikeys.Scopes()})
}

func (s *Server) listAPIKeys(w http.ResponseWriter, r *http.Request) {
	keys, err := s.apiKeyService.List(r.Context())
	if err != nil {
		s.internalError(w, r, err)
		return
	}
	writeJSON(w, 200, map[string]any{"api_keys": keys})
}

func (s *Server) getAPIKey(w http.ResponseWriter, r *http.Request) {
	key, err := s.apiKeyService.Get(r.Context(), chi.URLParam(r, "keyID"))
	if errors.Is(err, apikeys.ErrNotFound) {
		writeAPIError(w, r, 404, "API_KEY_NOT_FOUND", "The API key does not exist.")
		return
	}
	if err != nil {
		s.internalError(w, r, err)
		return
	}
	writeJSON(w, 200, map[string]any{"api_key": key})
}

func (s *Server) createAPIKey(w http.ResponseWriter, r *http.Request) {
	var input struct {
		Name      string   `json:"name"`
		Scopes    []string `json:"scopes"`
		ExpiresAt string   `json:"expires_at,omitempty"`
	}
	if err := decodeJSON(r, &input); err != nil {
		writeAPIError(w, r, 400, "INVALID_API_KEY", "The request body is invalid.")
		return
	}
	if !allowedToGrant(r, input.Scopes) {
		writeAPIError(w, r, 403, "SCOPE_ESCALATION", "A key cannot receive a scope the current user does not hold.")
		return
	}
	var expiry *time.Time
	if strings.TrimSpace(input.ExpiresAt) != "" {
		parsed, err := time.Parse(time.RFC3339, input.ExpiresAt)
		if err != nil {
			writeAPIError(w, r, 400, "INVALID_EXPIRY", "expires_at must be RFC 3339.")
			return
		}
		expiry = &parsed
	}
	created, err := s.apiKeyService.Create(
		r.Context(),
		principalFromContext(r.Context()).User.ID,
		input.Name,
		input.Scopes,
		expiry,
	)
	if err != nil {
		writeAPIError(w, r, 400, "INVALID_API_KEY", err.Error())
		return
	}
	w.Header().Set("Cache-Control", "no-store")
	s.recordAdminAudit(r, "api_key.create", "api_key", created.Key.ID, nil, created.Key, "")
	writeJSON(w, 201, created)
}

func (s *Server) updateAPIKey(w http.ResponseWriter, r *http.Request) {
	var input struct {
		Name   *string   `json:"name,omitempty"`
		Scopes *[]string `json:"scopes,omitempty"`
	}
	if err := decodeJSON(r, &input); err != nil || input.Name == nil && input.Scopes == nil {
		writeAPIError(w, r, 400, "INVALID_API_KEY", "name or scopes is required.")
		return
	}
	if input.Scopes != nil {
		if _, err := apikeys.ValidScopes(*input.Scopes); err != nil {
			writeAPIError(w, r, 400, "INVALID_SCOPES", err.Error())
			return
		}
	}
	id := chi.URLParam(r, "keyID")
	before, err := s.apiKeyService.Get(r.Context(), id)
	if err != nil {
		apiKeyError(w, r, err)
		return
	}
	key := before
	if input.Name != nil {
		key, err = s.apiKeyService.UpdateName(r.Context(), id, *input.Name)
	}
	if err == nil && input.Scopes != nil {
		if !allowedToGrant(r, *input.Scopes) {
			writeAPIError(w, r, 403, "SCOPE_ESCALATION", "A key cannot receive a scope the current user does not hold.")
			return
		}
		key, err = s.apiKeyService.ReplaceScopes(r.Context(), id, *input.Scopes)
	}
	if err != nil {
		apiKeyError(w, r, err)
		return
	}
	s.recordAdminAudit(r, "api_key.update", "api_key", id, before, key, "")
	writeJSON(w, 200, map[string]any{"api_key": key})
}

func (s *Server) addAPIKeyScopes(w http.ResponseWriter, r *http.Request) {
	var input struct {
		Scopes []string `json:"scopes"`
	}
	if err := decodeJSON(r, &input); err != nil || len(input.Scopes) == 0 {
		writeAPIError(w, r, 400, "INVALID_SCOPES", "At least one scope is required.")
		return
	}
	if !allowedToGrant(r, input.Scopes) {
		writeAPIError(w, r, 403, "SCOPE_ESCALATION", "A key cannot receive a scope the current user does not hold.")
		return
	}
	id := chi.URLParam(r, "keyID")
	before, _ := s.apiKeyService.Get(r.Context(), id)
	key, err := s.apiKeyService.AddScopes(r.Context(), id, input.Scopes)
	if err != nil {
		apiKeyError(w, r, err)
		return
	}
	s.recordAdminAudit(r, "api_key.scopes.add", "api_key", id, before.Scopes, key.Scopes, "")
	writeJSON(w, 200, map[string]any{"api_key": key})
}

func (s *Server) removeAPIKeyScope(w http.ResponseWriter, r *http.Request) {
	id := chi.URLParam(r, "keyID")
	before, _ := s.apiKeyService.Get(r.Context(), id)
	key, err := s.apiKeyService.RemoveScope(
		r.Context(), id, chi.URLParam(r, "scope"),
	)
	if err != nil {
		apiKeyError(w, r, err)
		return
	}
	s.recordAdminAudit(r, "api_key.scope.remove", "api_key", id, before.Scopes, key.Scopes, "")
	writeJSON(w, 200, map[string]any{"api_key": key})
}

func (s *Server) rotateAPIKey(w http.ResponseWriter, r *http.Request) {
	var input struct {
		GraceSeconds int64 `json:"grace_seconds"`
	}
	if err := decodeJSON(r, &input); err != nil {
		writeAPIError(w, r, 400, "INVALID_ROTATION", "The request body is invalid.")
		return
	}
	if input.GraceSeconds < 0 || input.GraceSeconds > 7*24*60*60 {
		writeAPIError(w, r, 400, "INVALID_ROTATION", "grace_seconds must be between 0 and 604800.")
		return
	}
	created, err := s.apiKeyService.Rotate(
		r.Context(),
		chi.URLParam(r, "keyID"),
		time.Duration(input.GraceSeconds)*time.Second,
	)
	if err != nil {
		apiKeyError(w, r, err)
		return
	}
	w.Header().Set("Cache-Control", "no-store")
	s.recordAdminAudit(r, "api_key.rotate", "api_key", created.Key.ID, nil,
		map[string]any{"prefix": created.Key.Prefix, "grace_seconds": input.GraceSeconds}, "")
	writeJSON(w, 200, created)
}

func (s *Server) revokeAPIKey(w http.ResponseWriter, r *http.Request) {
	id := chi.URLParam(r, "keyID")
	before, _ := s.apiKeyService.Get(r.Context(), id)
	if err := s.apiKeyService.Revoke(r.Context(), id); err != nil {
		apiKeyError(w, r, err)
		return
	}
	s.recordAdminAudit(r, "api_key.revoke", "api_key", id, before, nil, "")
	w.WriteHeader(http.StatusNoContent)
}

func allowedToGrant(r *http.Request, scopes []string) bool {
	principal := principalFromContext(r.Context())
	for _, scope := range scopes {
		if !principal.HasPermission(scope) {
			return false
		}
	}
	return true
}

func apiKeyError(w http.ResponseWriter, r *http.Request, err error) {
	switch {
	case errors.Is(err, apikeys.ErrNotFound):
		writeAPIError(w, r, 404, "API_KEY_NOT_FOUND", "The API key does not exist or is inactive.")
	case errors.Is(err, apikeys.ErrInvalid):
		writeAPIError(w, r, 400, "INVALID_API_KEY", err.Error())
	default:
		writeAPIError(w, r, 500, "API_KEY_ERROR", "The API key operation failed.")
	}
}
