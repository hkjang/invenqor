package httpapi

import (
	"errors"
	"net/http"
	"net/url"
	"strings"

	"github.com/go-chi/chi/v5/middleware"
	"github.com/hkjang/invenqor/server/internal/auth"
	"github.com/hkjang/invenqor/server/internal/diagnostics"
)

// Every Keycloak failure code an administrator can meet, with the action that
// resolves it. The console shows this next to the code.
var keycloakGuidance = map[string]string{
	"KEYCLOAK_DISABLED": "Enable Keycloak login in Settings > Keycloak.",
	"KEYCLOAK_SECRET_REQUIRED": "Enter the Keycloak client secret in " +
		"Settings > Keycloak and save it again.",
	"KEYCLOAK_FLOW_EXPIRED": "The login took longer than ten minutes or the " +
		"link was reused. Start the login again from the console.",
	"KEYCLOAK_NONCE_MISMATCH": "The ID token did not match this login " +
		"attempt. Confirm one Keycloak client is not shared by two servers.",
	"KEYCLOAK_EMAIL_DOMAIN_REJECTED": "Add the domain to the allowed email " +
		"domain list, or remove the restriction.",
	"KEYCLOAK_PROVISIONING_DISABLED": "Enable automatic user creation, or " +
		"create the account locally before the user signs in.",
	"KEYCLOAK_USERNAME_UNUSABLE": "Map a username claim that contains 3 to " +
		"64 letters, digits, dot, underscore or hyphen.",
	"KEYCLOAK_USERNAME_CONFLICT": "A local account already owns this name. " +
		"Rename or delete the local account, or map a different username claim.",
	"KEYCLOAK_USER_INACTIVE": "Reactivate the account on the user page.",
	"KEYCLOAK_ROLE_MISSING": "A role mapping points at a role that no longer " +
		"exists. Correct the mapping in Settings > Keycloak.",
	"KEYCLOAK_UNREACHABLE": "Check the Keycloak URL, realm, DNS and TLS " +
		"trust. Use the connection test in Settings > Keycloak.",
	"KEYCLOAK_PROVIDER_REJECTED": "Keycloak refused the request. Check the " +
		"client's valid redirect URIs and any authentication flow policy.",
	"KEYCLOAK_LOGIN_FAILED": "Open Server 진단 로그 and search for the " +
		"reported request ID.",
}

func keycloakRemediation(code string) string {
	if guidance, found := keycloakGuidance[code]; found {
		return guidance
	}
	return keycloakGuidance["KEYCLOAK_LOGIN_FAILED"]
}

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
	secretConfigured, err := s.oidcService.ClientSecretConfigured(request.Context())
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	// Offering the button without a client secret sends the user into a failed
	// redirect, so report readiness rather than the stored flag alone.
	ready := settings.Enabled && secretConfigured
	writeJSON(response, http.StatusOK, map[string]any{
		"local":                    true,
		"keycloak":                 ready,
		"keycloak_enabled":         settings.Enabled,
		"keycloak_client_secret":   secretConfigured,
		"keycloak_incomplete":      settings.Enabled && !secretConfigured,
		"keycloak_provider_issuer": settings.EffectiveIssuer(),
	})
}

func (s *Server) keycloakStart(response http.ResponseWriter, request *http.Request) {
	start, err := s.oidcService.Start(
		request.Context(),
		request.URL.Query().Get("return_to"),
		clientIP(request),
		request.UserAgent(),
	)
	if err != nil {
		code, message, _ := keycloakFailure(err)
		s.recordKeycloakFailure(request, "keycloak_start", code, message, err)
		// The browser navigates here directly, so hand the console a code it can
		// display instead of a raw JSON body on a blank page.
		s.redirectKeycloakFailure(response, request, code)
		return
	}
	http.Redirect(response, request, start.AuthorizationURL, http.StatusFound)
}

func (s *Server) keycloakCallback(response http.ResponseWriter, request *http.Request) {
	if providerError := strings.TrimSpace(
		request.URL.Query().Get("error"),
	); providerError != "" {
		// Keycloak reports consent denial and policy failures this way; without
		// this branch the user only saw the generic flow-expired message.
		s.recordKeycloakFailure(
			request,
			"keycloak_callback",
			"KEYCLOAK_PROVIDER_REJECTED",
			"Keycloak rejected the login request.",
			errors.New(providerError+": "+request.URL.Query().Get("error_description")),
		)
		s.redirectKeycloakFailure(response, request, "KEYCLOAK_PROVIDER_REJECTED")
		return
	}
	session, returnTo, err := s.oidcService.Callback(
		request.Context(),
		request.URL.Query().Get("state"),
		request.URL.Query().Get("code"),
		clientIP(request),
		request.UserAgent(),
		middleware.GetReqID(request.Context()),
	)
	if err != nil {
		code, message, _ := keycloakFailure(err)
		s.recordKeycloakFailure(request, "keycloak_callback", code, message, err)
		s.redirectKeycloakFailure(response, request, code)
		return
	}
	setSessionCookie(response, request, session)
	http.Redirect(response, request, returnTo, http.StatusFound)
}

// keycloakFailure maps a login failure to a stable code an administrator can
// look up, so an SSO problem is diagnosable without server shell access.
func keycloakFailure(err error) (string, string, int) {
	switch {
	case errors.Is(err, auth.ErrOIDCDisabled):
		return "KEYCLOAK_DISABLED",
			"Keycloak login is not enabled.",
			http.StatusNotFound
	case errors.Is(err, auth.ErrOIDCSecret):
		return "KEYCLOAK_SECRET_REQUIRED",
			"The Keycloak client secret is not configured.",
			http.StatusServiceUnavailable
	case errors.Is(err, auth.ErrOIDCFlow):
		return "KEYCLOAK_FLOW_EXPIRED",
			"The login attempt expired or was already used.",
			http.StatusUnauthorized
	case errors.Is(err, auth.ErrOIDCNonce):
		return "KEYCLOAK_NONCE_MISMATCH",
			"The Keycloak ID token nonce did not match.",
			http.StatusUnauthorized
	case errors.Is(err, auth.ErrOIDCDomain):
		return "KEYCLOAK_EMAIL_DOMAIN_REJECTED",
			"The account email domain is not allowed.",
			http.StatusForbidden
	case errors.Is(err, auth.ErrOIDCProvisioning):
		return "KEYCLOAK_PROVISIONING_DISABLED",
			"Automatic user creation is disabled for Keycloak logins.",
			http.StatusForbidden
	case errors.Is(err, auth.ErrOIDCUsername):
		return "KEYCLOAK_USERNAME_UNUSABLE",
			"The Keycloak username claim is missing or unusable.",
			http.StatusForbidden
	case errors.Is(err, auth.ErrOIDCUsernameTaken):
		return "KEYCLOAK_USERNAME_CONFLICT",
			"A different local account already uses this Keycloak username.",
			http.StatusConflict
	case errors.Is(err, auth.ErrOIDCUserInactive):
		return "KEYCLOAK_USER_INACTIVE",
			"The linked account is deactivated.",
			http.StatusForbidden
	case errors.Is(err, auth.ErrOIDCRole):
		return "KEYCLOAK_ROLE_MISSING",
			"A Keycloak mapping references a role that no longer exists.",
			http.StatusConflict
	case errors.Is(err, auth.ErrOIDCUnreachable):
		return "KEYCLOAK_UNREACHABLE",
			"The Keycloak issuer could not be reached.",
			http.StatusBadGateway
	default:
		return "KEYCLOAK_LOGIN_FAILED",
			"Keycloak login could not be completed.",
			http.StatusUnauthorized
	}
}

func (s *Server) redirectKeycloakFailure(
	response http.ResponseWriter,
	request *http.Request,
	code string,
) {
	http.Redirect(
		response,
		request,
		"/?auth_error="+url.QueryEscape(code)+
			"&request_id="+url.QueryEscape(middleware.GetReqID(request.Context())),
		http.StatusFound,
	)
}

func (s *Server) recordKeycloakFailure(
	request *http.Request,
	component string,
	code string,
	message string,
	err error,
) {
	s.logger.Warn(
		component+"_failed",
		"request_id", middleware.GetReqID(request.Context()),
		"code", code,
		"error", err,
	)
	s.recordDiagnostic(request, diagnostics.Event{
		Level:     "warning",
		Component: "keycloak",
		EventCode: code,
		Message:   message,
		SourceIP:  clientIP(request),
		Details: map[string]any{
			"stage":       component,
			"error":       err.Error(),
			"remediation": keycloakRemediation(code),
		},
	})
}

func (s *Server) getKeycloakSettings(response http.ResponseWriter, request *http.Request) {
	settings, err := s.oidcService.Settings(request.Context())
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	secretConfigured, err := s.oidcService.ClientSecretConfigured(request.Context())
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
			// The specific reason is what the administrator needs; the previous
			// fixed sentence made every field mistake look identical.
			writeAPIError(response, request, http.StatusBadRequest, "INVALID_KEYCLOAK_SETTINGS", err.Error())
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
		// A configuration mistake and an unreachable issuer need different
		// actions, and the previous single 502 hid which one occurred.
		if errors.Is(err, auth.ErrOIDCUnreachable) {
			writeAPIError(
				response, request, http.StatusBadGateway,
				"KEYCLOAK_CONNECTION_FAILED",
				"The Keycloak issuer could not be reached. "+
					keycloakRemediation("KEYCLOAK_UNREACHABLE"),
			)
			return
		}
		if errors.Is(err, auth.ErrOIDCRole) {
			writeAPIError(
				response, request, http.StatusBadRequest,
				"INVALID_KEYCLOAK_ROLE",
				"A Keycloak mapping references an unknown InvenQor role.",
			)
			return
		}
		writeAPIError(
			response, request, http.StatusBadRequest,
			"INVALID_KEYCLOAK_SETTINGS",
			err.Error(),
		)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{
		"connected": true,
		"issuer":    input.Settings.EffectiveIssuer(),
	})
}

func (s *Server) autoConfigureKeycloak(
	response http.ResponseWriter,
	request *http.Request,
) {
	var input struct {
		KeycloakURL    string  `json:"keycloak_url"`
		Realm          string  `json:"realm"`
		ClientID       string  `json:"client_id"`
		ClientSecret   *string `json:"client_secret,omitempty"`
		ApplicationURL string  `json:"application_url"`
		PrivateCAPEM   string  `json:"private_ca_pem"`
		Reason         string  `json:"reason"`
	}
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(
			response,
			request,
			http.StatusBadRequest,
			"INVALID_REQUEST",
			"The request body is invalid.",
		)
		return
	}
	settings, err := s.oidcService.AutomaticSettings(
		request.Context(),
		auth.OIDCAutoConfig{
			KeycloakURL:    input.KeycloakURL,
			Realm:          input.Realm,
			ClientID:       input.ClientID,
			ApplicationURL: input.ApplicationURL,
			PrivateCAPEM:   input.PrivateCAPEM,
		},
	)
	if err != nil {
		s.logger.Warn(
			"keycloak_auto_configuration_failed",
			"request_id", middleware.GetReqID(request.Context()),
			"error", err,
		)
		writeAPIError(
			response,
			request,
			http.StatusBadGateway,
			"KEYCLOAK_DISCOVERY_FAILED",
			"Keycloak discovery failed. Verify the URL, realm, and TLS trust.",
		)
		return
	}
	if err := s.oidcService.SaveSettings(
		request.Context(),
		settings,
		input.ClientSecret,
		principalFromContext(request.Context()).User,
		input.Reason,
	); err != nil {
		if errors.Is(err, auth.ErrOIDCSecret) {
			writeAPIError(
				response,
				request,
				http.StatusBadRequest,
				"KEYCLOAK_SECRET_REQUIRED",
				"A Keycloak client secret is required before enabling login.",
			)
			return
		}
		if errors.Is(err, auth.ErrOIDCRole) {
			writeAPIError(
				response,
				request,
				http.StatusBadRequest,
				"INVALID_KEYCLOAK_ROLE",
				"A Keycloak mapping references an unknown InvenQor role.",
			)
			return
		}
		s.internalError(response, request, err)
		return
	}
	// Report the stored state rather than assuming it: the save above succeeds
	// with a previously stored secret and no secret in this request.
	secretConfigured, err := s.oidcService.ClientSecretConfigured(request.Context())
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{
		"configured":               true,
		"settings":                 settings,
		"client_secret_configured": secretConfigured,
		"discovery_issuer":         settings.EffectiveIssuer(),
		"redirect_uri":             settings.RedirectURI,
	})
}
