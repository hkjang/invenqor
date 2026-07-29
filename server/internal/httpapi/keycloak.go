package httpapi

import (
	"errors"
	"net/http"

	"github.com/go-chi/chi/v5/middleware"
	"github.com/hkjang/invenqor/server/internal/auth"
)

type keycloakSettingsUpdate struct {
	Settings     auth.OIDCSettings `json:"settings"`
	ClientSecret *string           `json:"client_secret,omitempty"`
	Reason       string            `json:"reason"`
}

func (s *Server) authMethods(response http.ResponseWriter, request *http.Request) {
	settings, err := s.oidcService.Settings(request.Context())
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{
		"local":    true,
		"keycloak": settings.Enabled,
	})
}

func (s *Server) keycloakStart(response http.ResponseWriter, request *http.Request) {
	start, err := s.oidcService.Start(
		request.Context(),
		request.URL.Query().Get("return_to"),
		clientIP(request),
		request.UserAgent(),
	)
	if errors.Is(err, auth.ErrOIDCDisabled) {
		writeAPIError(response, request, http.StatusNotFound, "KEYCLOAK_DISABLED", "Keycloak login is not enabled.")
		return
	}
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	http.Redirect(response, request, start.AuthorizationURL, http.StatusFound)
}

func (s *Server) keycloakCallback(response http.ResponseWriter, request *http.Request) {
	session, returnTo, err := s.oidcService.Callback(
		request.Context(),
		request.URL.Query().Get("state"),
		request.URL.Query().Get("code"),
		clientIP(request),
		request.UserAgent(),
		middleware.GetReqID(request.Context()),
	)
	if err != nil {
		s.logger.Warn(
			"keycloak_callback_failed",
			"request_id", middleware.GetReqID(request.Context()),
			"error", err,
		)
		writeAPIError(response, request, http.StatusUnauthorized, "KEYCLOAK_LOGIN_FAILED", "Keycloak login could not be completed.")
		return
	}
	setSessionCookie(response, session)
	http.Redirect(response, request, returnTo, http.StatusFound)
}

func (s *Server) getKeycloakSettings(response http.ResponseWriter, request *http.Request) {
	settings, err := s.oidcService.Settings(request.Context())
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	secretConfigured, err := s.oidcService.ClientSecretConfigured()
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{
		"settings":                 settings,
		"client_secret_configured": secretConfigured,
	})
}

func (s *Server) updateKeycloakSettings(response http.ResponseWriter, request *http.Request) {
	var input keycloakSettingsUpdate
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, http.StatusBadRequest, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	if err := s.oidcService.SaveSettings(
		request.Context(),
		input.Settings,
		input.ClientSecret,
		principalFromContext(request.Context()).User,
		input.Reason,
	); err != nil {
		s.logger.Warn(
			"keycloak_settings_rejected",
			"request_id", middleware.GetReqID(request.Context()),
			"error", err,
		)
		switch {
		case errors.Is(err, auth.ErrOIDCSecret):
			writeAPIError(response, request, http.StatusBadRequest, "KEYCLOAK_SECRET_REQUIRED", "A Keycloak client secret is required before enabling login.")
		case errors.Is(err, auth.ErrOIDCRole):
			writeAPIError(response, request, http.StatusBadRequest, "INVALID_KEYCLOAK_ROLE", "A Keycloak mapping references an unknown InvenQor role.")
		default:
			writeAPIError(response, request, http.StatusBadRequest, "INVALID_KEYCLOAK_SETTINGS", "The Keycloak settings are invalid.")
		}
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"saved": true})
}

func (s *Server) testKeycloakSettings(response http.ResponseWriter, request *http.Request) {
	var input struct {
		Settings auth.OIDCSettings `json:"settings"`
	}
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, http.StatusBadRequest, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	if err := s.oidcService.TestConnection(request.Context(), input.Settings); err != nil {
		writeAPIError(response, request, http.StatusBadGateway, "KEYCLOAK_CONNECTION_FAILED", "The Keycloak issuer connection test failed.")
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{
		"connected": true,
		"issuer":    input.Settings.EffectiveIssuer(),
	})
}
