package httpapi

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/go-chi/chi/v5"
	"github.com/go-chi/chi/v5/middleware"
	"github.com/hkjang/invenqor/server/internal/agents"
	"github.com/hkjang/invenqor/server/internal/ingest"
	"github.com/hkjang/invenqor/server/internal/storage"
)

const maxAgentEventBytes = 16 * 1024 * 1024

func (s *Server) receiveAgentEvent(
	response http.ResponseWriter,
	request *http.Request,
) {
	agent, err := s.authenticateAgent(request)
	switch {
	case errors.Is(err, agents.ErrUnauthorized):
		writeAPIError(
			response, request, http.StatusUnauthorized,
			"AGENT_UNAUTHORIZED", "The agent credential is invalid.",
		)
		return
	case errors.Is(err, agents.ErrBlocked):
		writeAPIError(
			response, request, http.StatusForbidden,
			"AGENT_BLOCKED", "The agent is blocked.",
		)
		return
	case err != nil:
		context, cancel := context.WithTimeout(request.Context(), time.Second)
		defer cancel()
		if pingErr := s.database.Ping(context); pingErr != nil {
			writeAPIError(
				response, request, http.StatusServiceUnavailable,
				"AGENT_AUTH_UNAVAILABLE",
				"Agent authentication is temporarily unavailable.",
			)
			return
		}
		s.internalError(response, request, err)
		return
	}
	headerAgentID := strings.TrimSpace(
		request.Header.Get("X-Invenqor-Agent-Id"),
	)
	headerEventID := strings.TrimSpace(
		request.Header.Get("X-Invenqor-Event-Id"),
	)
	if headerAgentID == "" || headerEventID == "" {
		writeAPIError(
			response, request, http.StatusBadRequest,
			"MISSING_EVENT_IDENTITY",
			"X-Invenqor-Agent-Id and X-Invenqor-Event-Id are required.",
		)
		return
	}
	if headerAgentID != agent.AgentID {
		writeAPIError(
			response, request, http.StatusForbidden,
			"AGENT_ID_MISMATCH",
			"The authenticated agent does not match the request header.",
		)
		return
	}
	if !s.agentRateLimit.Allow(agent.ID) {
		response.Header().Set("Retry-After", "60")
		writeAPIError(
			response, request, http.StatusTooManyRequests,
			"AGENT_RATE_LIMITED", "The agent request rate limit was exceeded.",
		)
		return
	}
	request.Body = http.MaxBytesReader(
		response, request.Body, maxAgentEventBytes,
	)
	raw, err := io.ReadAll(request.Body)
	if err != nil {
		code := "INVALID_EVENT"
		message := "The event body could not be read."
		if strings.Contains(err.Error(), "request body too large") {
			code = "EVENT_TOO_LARGE"
			message = "The event exceeds the maximum allowed size."
		}
		writeAPIError(response, request, http.StatusBadRequest, code, message)
		return
	}
	envelope, err := ingest.DecodeEnvelope(raw)
	if err != nil {
		s.logger.Info(
			"agent_event_rejected",
			"request_id", middleware.GetReqID(request.Context()),
			"agent_id", agent.AgentID,
			"reason", err.Error(),
		)
		writeAPIError(
			response, request, http.StatusBadRequest,
			"INVALID_EVENT", err.Error(),
		)
		return
	}
	if envelope.AgentID != headerAgentID ||
		envelope.EventID != headerEventID {
		writeAPIError(
			response, request, http.StatusBadRequest,
			"EVENT_IDENTITY_MISMATCH",
			"Event identity headers do not match the body.",
		)
		return
	}
	if s.database.Mode() == storage.ModePostgresDegraded {
		s.spoolAgentEvent(response, request, envelope, raw)
		return
	}
	result, err := s.ingestService.Process(
		request.Context(),
		agent,
		envelope,
		raw,
		agentVersion(request.UserAgent()),
	)
	switch {
	case err == nil:
		writeJSON(response, http.StatusOK, map[string]any{
			"accepted":       true,
			"duplicate":      result.Duplicate,
			"policy_version": result.PolicyVersion,
		})
	case errors.Is(err, ingest.ErrInProgress):
		response.Header().Set("Retry-After", "1")
		writeAPIError(
			response, request, http.StatusConflict,
			"EVENT_IN_PROGRESS", "The event is already being processed.",
		)
	default:
		context, cancel := context.WithTimeout(request.Context(), time.Second)
		pingErr := s.database.Ping(context)
		cancel()
		if pingErr != nil {
			s.spoolAgentEvent(response, request, envelope, raw)
			return
		}
		s.internalError(response, request, err)
	}
}

func (s *Server) spoolAgentEvent(
	response http.ResponseWriter,
	request *http.Request,
	envelope ingest.Envelope,
	raw []byte,
) {
	if s.spool == nil {
		writeAPIError(
			response, request, http.StatusServiceUnavailable,
			"EVENT_SPOOL_UNAVAILABLE",
			"The durable event spool is not configured.",
		)
		return
	}
	duplicate, err := s.spool.Append(
		envelope.AgentID, envelope.EventID, raw,
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	writeJSON(response, http.StatusAccepted, map[string]any{
		"accepted":  true,
		"spooled":   true,
		"duplicate": duplicate,
	})
}

func (s *Server) authenticateAgent(
	request *http.Request,
) (agents.Agent, error) {
	if token := bearerToken(request); token != "" {
		return s.agentService.AuthenticateBearer(request.Context(), token)
	}
	if request.TLS == nil || len(request.TLS.PeerCertificates) == 0 {
		return agents.Agent{}, agents.ErrUnauthorized
	}
	sum := sha256.Sum256(request.TLS.PeerCertificates[0].Raw)
	return s.agentService.AuthenticateCertificate(
		request.Context(), hex.EncodeToString(sum[:]),
	)
}

func (s *Server) provisionAgent(
	response http.ResponseWriter,
	request *http.Request,
) {
	var input struct {
		AgentID  string `json:"agent_id"`
		Hostname string `json:"hostname"`
	}
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, 400, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	result, err := s.agentService.ProvisionBearer(
		request.Context(),
		input.AgentID,
		input.Hostname,
		principalFromContext(request.Context()).User.ID,
	)
	if err != nil {
		writeAPIError(response, request, 400, "INVALID_AGENT", err.Error())
		return
	}
	response.Header().Set("Cache-Control", "no-store")
	writeJSON(response, http.StatusCreated, result)
}

func (s *Server) rotateAgentToken(
	response http.ResponseWriter,
	request *http.Request,
) {
	var input struct {
		GraceSeconds int64 `json:"grace_seconds"`
	}
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, 400, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	if input.GraceSeconds < 0 || input.GraceSeconds > 7*24*60*60 {
		writeAPIError(response, request, 400, "INVALID_GRACE_PERIOD", "Grace period must be between zero and seven days.")
		return
	}
	token, err := s.agentService.RotateBearer(
		request.Context(),
		chi.URLParam(request, "agentID"),
		time.Duration(input.GraceSeconds)*time.Second,
		principalFromContext(request.Context()).User.ID,
	)
	if errors.Is(err, sql.ErrNoRows) {
		writeAPIError(response, request, 404, "AGENT_NOT_FOUND", "The agent does not exist.")
		return
	}
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	response.Header().Set("Cache-Control", "no-store")
	writeJSON(response, http.StatusOK, map[string]any{"token": token})
}

func (s *Server) blockAgent(
	response http.ResponseWriter,
	request *http.Request,
) {
	s.setAgentBlocked(response, request, true)
}

func (s *Server) unblockAgent(
	response http.ResponseWriter,
	request *http.Request,
) {
	s.setAgentBlocked(response, request, false)
}

func (s *Server) setAgentBlocked(
	response http.ResponseWriter,
	request *http.Request,
	blocked bool,
) {
	err := s.agentService.SetBlocked(
		request.Context(),
		chi.URLParam(request, "agentID"),
		blocked,
		principalFromContext(request.Context()).User.ID,
	)
	if errors.Is(err, sql.ErrNoRows) {
		writeAPIError(response, request, 404, "AGENT_NOT_FOUND", "The agent does not exist.")
		return
	}
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	writeJSON(response, http.StatusOK, map[string]any{"blocked": blocked})
}

func (s *Server) registerAgentCertificate(
	response http.ResponseWriter,
	request *http.Request,
) {
	var input struct {
		Fingerprint string `json:"fingerprint"`
		ExpiresAt   string `json:"expires_at"`
	}
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, 400, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	var expiresAt *time.Time
	if input.ExpiresAt != "" {
		parsed, err := time.Parse(time.RFC3339, input.ExpiresAt)
		if err != nil {
			writeAPIError(response, request, 400, "INVALID_EXPIRY", "expires_at must be RFC 3339.")
			return
		}
		expiresAt = &parsed
	}
	err := s.agentService.RegisterCertificate(
		request.Context(),
		chi.URLParam(request, "agentID"),
		input.Fingerprint,
		expiresAt,
		principalFromContext(request.Context()).User.ID,
	)
	if err != nil {
		writeAPIError(response, request, 400, "INVALID_CERTIFICATE", err.Error())
		return
	}
	writeJSON(response, http.StatusCreated, map[string]any{"registered": true})
}

func (s *Server) listAgents(
	response http.ResponseWriter,
	request *http.Request,
) {
	rows, err := s.database.DB().QueryContext(
		request.Context(),
		`SELECT id, agent_id, hostname, status, version, os_name,
		 architecture, auth_method, policy_version
		 FROM agents ORDER BY hostname, agent_id`,
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	defer rows.Close()
	result := make([]agents.Agent, 0)
	for rows.Next() {
		var agent agents.Agent
		if err := rows.Scan(
			&agent.ID, &agent.AgentID, &agent.Hostname, &agent.Status,
			&agent.Version, &agent.OSName, &agent.Architecture,
			&agent.AuthMethod, &agent.PolicyVersion,
		); err != nil {
			s.internalError(response, request, err)
			return
		}
		result = append(result, agent)
	}
	writeJSON(response, http.StatusOK, map[string]any{"agents": result})
}

func agentVersion(userAgent string) string {
	const prefix = "invenqor-agent/"
	for _, item := range strings.Fields(userAgent) {
		if strings.HasPrefix(item, prefix) {
			return strings.TrimPrefix(item, prefix)
		}
	}
	return ""
}

type rateWindow struct {
	start time.Time
	count int
}

type agentRateLimiter struct {
	mu      sync.Mutex
	limit   int
	window  time.Duration
	entries map[string]rateWindow
}

func newAgentRateLimiter(limit int, window time.Duration) *agentRateLimiter {
	return &agentRateLimiter{
		limit: limit, window: window, entries: make(map[string]rateWindow),
	}
}

func (l *agentRateLimiter) Allow(key string) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	now := time.Now()
	entry := l.entries[key]
	if entry.start.IsZero() || now.Sub(entry.start) >= l.window {
		l.entries[key] = rateWindow{start: now, count: 1}
		return true
	}
	if entry.count >= l.limit {
		return false
	}
	entry.count++
	l.entries[key] = entry
	return true
}

// Marshal helper is intentionally unused by handlers but gives integration
// callers one canonical representation for replaying an accepted event.
func CanonicalAgentEvent(envelope ingest.Envelope) ([]byte, error) {
	bytes, err := json.Marshal(envelope)
	if err != nil {
		return nil, fmt.Errorf("marshal agent event: %w", err)
	}
	return bytes, nil
}
