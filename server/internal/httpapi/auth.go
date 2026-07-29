package httpapi

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"strings"
	"time"

	"github.com/go-chi/chi/v5/middleware"
	"github.com/hkjang/invenqor/server/internal/auth"
)

type principalContextKey struct{}

func (s *Server) bootstrapStatus(response http.ResponseWriter, request *http.Request) {
	status, err := s.bootstrapManager.Ensure(request.Context())
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	writeJSON(response, http.StatusOK, status)
}

func (s *Server) createInitialAdmin(response http.ResponseWriter, request *http.Request) {
	var input auth.InitialAdminInput
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, http.StatusBadRequest, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	user, err := s.bootstrapManager.CreateInitialAdmin(
		request.Context(),
		request.Header.Get("X-Invenqor-Bootstrap-Token"),
		input,
		clientIP(request),
		request.UserAgent(),
		middleware.GetReqID(request.Context()),
	)
	switch {
	case err == nil:
		writeJSON(response, http.StatusCreated, map[string]any{"user": user})
	case errors.Is(err, auth.ErrBootstrapComplete):
		writeAPIError(response, request, http.StatusConflict, "BOOTSTRAP_COMPLETE", "Initial setup is already complete.")
	case errors.Is(err, auth.ErrBootstrapToken):
		writeAPIError(response, request, http.StatusUnauthorized, "INVALID_BOOTSTRAP_TOKEN", "The bootstrap token is invalid.")
	default:
		s.logger.Warn(
			"initial_admin_rejected",
			"request_id", middleware.GetReqID(request.Context()),
			"error", err,
		)
		writeAPIError(response, request, http.StatusBadRequest, "INVALID_ADMIN", "The initial administrator details do not satisfy policy.")
	}
}

func (s *Server) localLogin(response http.ResponseWriter, request *http.Request) {
	var input auth.LoginInput
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, http.StatusBadRequest, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	session, err := s.authService.Authenticate(
		request.Context(),
		input,
		clientIP(request),
		request.UserAgent(),
		middleware.GetReqID(request.Context()),
	)
	switch {
	case err == nil:
		setSessionCookie(response, session)
		writeJSON(response, http.StatusOK, map[string]any{
			"user":                session.User,
			"csrf_token":          session.CSRFToken,
			"idle_expires_at":     session.IdleExpiresAt,
			"absolute_expires_at": session.AbsoluteExpiresAt,
		})
	case errors.Is(err, auth.ErrInvalidCredentials):
		writeAPIError(response, request, http.StatusUnauthorized, "INVALID_CREDENTIALS", "The username or password is invalid.")
	case errors.Is(err, auth.ErrAccountLocked):
		writeAPIError(response, request, http.StatusLocked, "ACCOUNT_LOCKED", "The account is temporarily locked.")
	case errors.Is(err, auth.ErrAccountInactive):
		writeAPIError(response, request, http.StatusForbidden, "ACCOUNT_INACTIVE", "The account is inactive.")
	case errors.Is(err, auth.ErrMFARequired):
		writeAPIError(response, request, http.StatusUnauthorized, "MFA_REQUIRED", "A TOTP code is required.")
	case errors.Is(err, auth.ErrMFAInvalid):
		writeAPIError(response, request, http.StatusUnauthorized, "MFA_INVALID", "The TOTP code is invalid.")
	default:
		s.internalError(response, request, err)
	}
}

func (s *Server) setupTOTP(response http.ResponseWriter, request *http.Request) {
	setup, err := s.totpService.Setup(
		request.Context(),
		principalFromContext(request.Context()).User,
	)
	switch {
	case err == nil:
		writeJSON(response, http.StatusOK, setup)
	case errors.Is(err, auth.ErrMFAAlreadyEnabled):
		writeAPIError(response, request, http.StatusConflict, "MFA_ALREADY_ENABLED", "TOTP is already enabled.")
	default:
		s.internalError(response, request, err)
	}
}

func (s *Server) enableTOTP(response http.ResponseWriter, request *http.Request) {
	var input struct {
		Code string `json:"code"`
	}
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, http.StatusBadRequest, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	if err := s.totpService.Enable(
		request.Context(),
		principalFromContext(request.Context()).User.ID,
		input.Code,
	); err != nil {
		writeAPIError(response, request, http.StatusBadRequest, "MFA_INVALID", "The TOTP code is invalid.")
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"totp_enabled": true})
}

func (s *Server) disableTOTP(response http.ResponseWriter, request *http.Request) {
	var input struct {
		Code string `json:"code"`
	}
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, http.StatusBadRequest, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	if err := s.totpService.Disable(
		request.Context(),
		principalFromContext(request.Context()).User.ID,
		input.Code,
	); err != nil {
		writeAPIError(response, request, http.StatusBadRequest, "MFA_INVALID", "The TOTP code is invalid.")
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"totp_enabled": false})
}

func setSessionCookie(response http.ResponseWriter, session auth.Session) {
	http.SetCookie(response, &http.Cookie{
		Name:     auth.SessionCookie,
		Value:    session.Token,
		Path:     "/",
		Secure:   true,
		HttpOnly: true,
		SameSite: http.SameSiteStrictMode,
		Expires:  session.AbsoluteExpiresAt,
		MaxAge:   int(time.Until(session.AbsoluteExpiresAt).Seconds()),
	})
	http.SetCookie(response, &http.Cookie{
		Name:     auth.CSRFCookie,
		Value:    session.CSRFToken,
		Path:     "/",
		Secure:   true,
		HttpOnly: false,
		SameSite: http.SameSiteStrictMode,
		Expires:  session.AbsoluteExpiresAt,
		MaxAge:   int(time.Until(session.AbsoluteExpiresAt).Seconds()),
	})
}

func (s *Server) me(response http.ResponseWriter, request *http.Request) {
	principal := principalFromContext(request.Context())
	writeJSON(response, http.StatusOK, map[string]any{
		"user":                principal.User,
		"idle_expires_at":     principal.IdleExpiresAt,
		"absolute_expires_at": principal.AbsoluteExpiresAt,
	})
}

func (s *Server) logout(response http.ResponseWriter, request *http.Request) {
	principal := principalFromContext(request.Context())
	logoutURL, logoutURLError := s.oidcService.LogoutURL(
		request.Context(),
		principal.User.ID,
	)
	if err := s.authService.RevokeSession(
		request.Context(),
		principal,
		clientIP(request),
		request.UserAgent(),
		middleware.GetReqID(request.Context()),
	); err != nil {
		s.internalError(response, request, err)
		return
	}
	if logoutURLError != nil {
		s.logger.Warn(
			"keycloak_logout_url_failed",
			"request_id", middleware.GetReqID(request.Context()),
			"error", logoutURLError,
		)
	}
	http.SetCookie(response, &http.Cookie{
		Name:     auth.SessionCookie,
		Value:    "",
		Path:     "/",
		Secure:   true,
		HttpOnly: true,
		SameSite: http.SameSiteStrictMode,
		MaxAge:   -1,
		Expires:  time.Unix(1, 0),
	})
	http.SetCookie(response, &http.Cookie{
		Name:     auth.CSRFCookie,
		Value:    "",
		Path:     "/",
		Secure:   true,
		HttpOnly: false,
		SameSite: http.SameSiteStrictMode,
		MaxAge:   -1,
		Expires:  time.Unix(1, 0),
	})
	writeJSON(response, http.StatusOK, map[string]any{
		"logged_out": true,
		"logout_url": logoutURL,
	})
}

func (s *Server) changePassword(response http.ResponseWriter, request *http.Request) {
	var input auth.ChangePasswordInput
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, http.StatusBadRequest, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	err := s.authService.ChangePassword(
		request.Context(),
		principalFromContext(request.Context()),
		input,
		clientIP(request),
		request.UserAgent(),
		middleware.GetReqID(request.Context()),
	)
	switch {
	case err == nil:
		writeJSON(response, http.StatusOK, map[string]any{"password_changed": true})
	case errors.Is(err, auth.ErrInvalidCredentials):
		writeAPIError(response, request, http.StatusUnauthorized, "INVALID_CREDENTIALS", "The current password is invalid.")
	case errors.Is(err, auth.ErrPasswordUnchanged):
		writeAPIError(response, request, http.StatusBadRequest, "PASSWORD_UNCHANGED", "The new password must differ from the current password.")
	default:
		s.logger.Warn(
			"password_change_rejected",
			"request_id", middleware.GetReqID(request.Context()),
			"error", err,
		)
		writeAPIError(response, request, http.StatusBadRequest, "PASSWORD_POLICY", "The new password does not satisfy policy.")
	}
}

func (s *Server) authenticate(next http.Handler) http.Handler {
	return http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		cookie, err := request.Cookie(auth.SessionCookie)
		if err != nil {
			writeAPIError(response, request, http.StatusUnauthorized, "UNAUTHORIZED", "Authentication is required.")
			return
		}
		principal, err := s.authService.PrincipalByToken(request.Context(), cookie.Value)
		if err != nil {
			writeAPIError(response, request, http.StatusUnauthorized, "UNAUTHORIZED", "Authentication is required.")
			return
		}
		context := context.WithValue(request.Context(), principalContextKey{}, principal)
		next.ServeHTTP(response, request.WithContext(context))
	})
}

func (s *Server) requireCSRF(next http.Handler) http.Handler {
	return http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		principal := principalFromContext(request.Context())
		if err := s.authService.VerifyCSRF(
			principal,
			request.Header.Get("X-CSRF-Token"),
		); err != nil {
			writeAPIError(response, request, http.StatusForbidden, "CSRF_VALIDATION_FAILED", "The CSRF token is invalid.")
			return
		}
		next.ServeHTTP(response, request)
	})
}

func (s *Server) requirePermission(permission string) func(http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
			if !principalFromContext(request.Context()).HasPermission(permission) {
				writeAPIError(response, request, http.StatusForbidden, "FORBIDDEN", "The current user lacks the required permission.")
				return
			}
			next.ServeHTTP(response, request)
		})
	}
}

func principalFromContext(ctx context.Context) auth.Principal {
	principal, _ := ctx.Value(principalContextKey{}).(auth.Principal)
	return principal
}

func decodeJSON(request *http.Request, destination any) error {
	request.Body = http.MaxBytesReader(nil, request.Body, 1024*1024)
	decoder := json.NewDecoder(request.Body)
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(destination); err != nil {
		return err
	}
	if err := decoder.Decode(&struct{}{}); !errors.Is(err, io.EOF) {
		return errors.New("request body must contain one JSON value")
	}
	return nil
}

func clientIP(request *http.Request) string {
	host, _, err := net.SplitHostPort(request.RemoteAddr)
	if err == nil {
		return host
	}
	return strings.TrimSpace(request.RemoteAddr)
}

func (s *Server) internalError(
	response http.ResponseWriter,
	request *http.Request,
	err error,
) {
	s.logger.Error(
		"http_internal_error",
		"request_id", middleware.GetReqID(request.Context()),
		"error", err,
	)
	writeAPIError(response, request, http.StatusInternalServerError, "INTERNAL_ERROR", "The server could not complete the request.")
}

func writeAPIError(
	response http.ResponseWriter,
	request *http.Request,
	status int,
	code string,
	message string,
) {
	writeJSON(response, status, map[string]any{
		"error": map[string]string{
			"code":    code,
			"message": message,
		},
		"request_id": middleware.GetReqID(request.Context()),
	})
}
