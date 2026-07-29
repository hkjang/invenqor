package httpapi

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"time"
)

const agentEnrollmentPolicyKey = "agent_enrollment_policy"

var errAgentEnrollmentPolicyConflict = errors.New(
	"agent enrollment policy changed concurrently",
)
var errAgentEnrollmentTokenNotConfigured = errors.New(
	"registration token is not configured",
)

type agentEnrollmentPolicy struct {
	Enabled      bool      `json:"enabled"`
	RequireToken bool      `json:"require_token"`
	TokenHash    string    `json:"token_hash,omitempty"`
	Version      int       `json:"version"`
	UpdatedAt    time.Time `json:"updated_at"`
	UpdatedBy    string    `json:"updated_by"`
}

func (policy agentEnrollmentPolicy) mode() string {
	if !policy.Enabled {
		return "disabled"
	}
	if policy.RequireToken {
		return "token"
	}
	return "open"
}

func (policy agentEnrollmentPolicy) publicValue() map[string]any {
	return map[string]any{
		"enabled":          policy.Enabled,
		"mode":             policy.mode(),
		"token_configured": policy.TokenHash != "",
		"version":          policy.Version,
		"updated_at":       policy.UpdatedAt,
		"updated_by":       policy.UpdatedBy,
		"source":           "database",
	}
}

func (s *Server) initialAgentEnrollmentPolicy() agentEnrollmentPolicy {
	policy := agentEnrollmentPolicy{
		Enabled:   s.agentEnrollmentEnabled,
		Version:   1,
		UpdatedAt: time.Now().UTC(),
		UpdatedBy: "startup-environment",
	}
	if s.agentEnrollmentTokenRequired {
		policy.RequireToken = true
		policy.TokenHash = hex.EncodeToString(s.agentEnrollmentTokenHash[:])
	}
	return policy
}

func (s *Server) loadAgentEnrollmentPolicy(
	ctx context.Context,
) (agentEnrollmentPolicy, string, error) {
	var raw string
	err := s.database.DB().QueryRowContext(
		ctx,
		`SELECT value FROM server_metadata WHERE key=$1`,
		agentEnrollmentPolicyKey,
	).Scan(&raw)
	if errors.Is(err, sql.ErrNoRows) {
		initial := s.initialAgentEnrollmentPolicy()
		encoded, encodeErr := json.Marshal(initial)
		if encodeErr != nil {
			return agentEnrollmentPolicy{}, "", encodeErr
		}
		_, insertErr := s.database.DB().ExecContext(
			ctx,
			`INSERT INTO server_metadata(key,value,updated_at)
			 VALUES($1,$2,$3) ON CONFLICT(key) DO NOTHING`,
			agentEnrollmentPolicyKey,
			string(encoded),
			initial.UpdatedAt,
		)
		if insertErr != nil {
			return agentEnrollmentPolicy{}, "", fmt.Errorf(
				"initialize agent enrollment policy: %w",
				insertErr,
			)
		}
		err = s.database.DB().QueryRowContext(
			ctx,
			`SELECT value FROM server_metadata WHERE key=$1`,
			agentEnrollmentPolicyKey,
		).Scan(&raw)
	}
	if err != nil {
		return agentEnrollmentPolicy{}, "", fmt.Errorf(
			"load agent enrollment policy: %w",
			err,
		)
	}
	var policy agentEnrollmentPolicy
	if err := json.Unmarshal([]byte(raw), &policy); err != nil {
		return agentEnrollmentPolicy{}, "", fmt.Errorf(
			"decode agent enrollment policy: %w",
			err,
		)
	}
	if policy.Version < 1 {
		return agentEnrollmentPolicy{}, "", errors.New(
			"agent enrollment policy has an invalid version",
		)
	}
	if policy.TokenHash != "" {
		decoded, err := hex.DecodeString(policy.TokenHash)
		if err != nil || len(decoded) != sha256.Size {
			return agentEnrollmentPolicy{}, "", errors.New(
				"agent enrollment policy has an invalid token digest",
			)
		}
	}
	if policy.RequireToken && policy.TokenHash == "" {
		return agentEnrollmentPolicy{}, "", errors.New(
			"agent enrollment policy requires a missing token",
		)
	}
	return policy, raw, nil
}

func (s *Server) updateAgentEnrollmentPolicy(
	ctx context.Context,
	updatedBy string,
	mutate func(*agentEnrollmentPolicy) error,
) (agentEnrollmentPolicy, agentEnrollmentPolicy, error) {
	for range 5 {
		before, raw, err := s.loadAgentEnrollmentPolicy(ctx)
		if err != nil {
			return agentEnrollmentPolicy{}, agentEnrollmentPolicy{}, err
		}
		after := before
		if err := mutate(&after); err != nil {
			return before, agentEnrollmentPolicy{}, err
		}
		after.Version = before.Version + 1
		after.UpdatedAt = time.Now().UTC()
		after.UpdatedBy = updatedBy
		encoded, err := json.Marshal(after)
		if err != nil {
			return before, agentEnrollmentPolicy{}, err
		}
		result, err := s.database.DB().ExecContext(
			ctx,
			`UPDATE server_metadata SET value=$1,updated_at=$2
			 WHERE key=$3 AND value=$4`,
			string(encoded),
			after.UpdatedAt,
			agentEnrollmentPolicyKey,
			raw,
		)
		if err != nil {
			return before, agentEnrollmentPolicy{}, fmt.Errorf(
				"save agent enrollment policy: %w",
				err,
			)
		}
		changed, err := result.RowsAffected()
		if err != nil {
			return before, agentEnrollmentPolicy{}, err
		}
		if changed == 1 {
			return before, after, nil
		}
	}
	return agentEnrollmentPolicy{}, agentEnrollmentPolicy{},
		errAgentEnrollmentPolicyConflict
}

func (s *Server) getAgentEnrollmentSettings(
	response http.ResponseWriter,
	request *http.Request,
) {
	policy, _, err := s.loadAgentEnrollmentPolicy(request.Context())
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	writeJSON(response, http.StatusOK, policy.publicValue())
}

func (s *Server) updateAgentEnrollmentSettings(
	response http.ResponseWriter,
	request *http.Request,
) {
	var input struct {
		Mode   string `json:"mode"`
		Reason string `json:"reason"`
	}
	if decodeJSON(request, &input) != nil {
		writeAPIError(
			response, request, http.StatusBadRequest,
			"INVALID_AGENT_ENROLLMENT_POLICY",
			"The request body is invalid.",
		)
		return
	}
	input.Mode = strings.ToLower(strings.TrimSpace(input.Mode))
	if input.Mode != "disabled" && input.Mode != "open" &&
		input.Mode != "token" {
		writeAPIError(
			response, request, http.StatusBadRequest,
			"INVALID_AGENT_ENROLLMENT_MODE",
			"mode must be disabled, open, or token.",
		)
		return
	}
	principal := principalFromContext(request.Context())
	before, after, err := s.updateAgentEnrollmentPolicy(
		request.Context(),
		principal.User.ID,
		func(policy *agentEnrollmentPolicy) error {
			switch input.Mode {
			case "disabled":
				policy.Enabled = false
			case "open":
				policy.Enabled = true
				policy.RequireToken = false
			case "token":
				if policy.TokenHash == "" {
					return errAgentEnrollmentTokenNotConfigured
				}
				policy.Enabled = true
				policy.RequireToken = true
			}
			return nil
		},
	)
	if err != nil {
		if errors.Is(err, errAgentEnrollmentTokenNotConfigured) {
			writeAPIError(
				response, request, http.StatusConflict,
				"AGENT_ENROLLMENT_TOKEN_REQUIRED",
				"Issue a registration token before enabling protected mode.",
			)
			return
		}
		if errors.Is(err, errAgentEnrollmentPolicyConflict) {
			writeAPIError(
				response, request, http.StatusConflict,
				"AGENT_ENROLLMENT_POLICY_CONFLICT",
				"The policy changed concurrently. Reload and try again.",
			)
			return
		}
		s.internalError(response, request, err)
		return
	}
	s.recordAdminAudit(
		request,
		"agent.enrollment_policy.update",
		"agent_enrollment_policy",
		agentEnrollmentPolicyKey,
		before.publicValue(),
		after.publicValue(),
		input.Reason,
	)
	writeJSON(response, http.StatusOK, after.publicValue())
}

func (s *Server) issueAgentEnrollmentToken(
	response http.ResponseWriter,
	request *http.Request,
) {
	var input struct {
		Reason string `json:"reason"`
	}
	if decodeJSON(request, &input) != nil {
		writeAPIError(
			response, request, http.StatusBadRequest,
			"INVALID_AGENT_ENROLLMENT_TOKEN_REQUEST",
			"The request body is invalid.",
		)
		return
	}
	token, err := newAgentEnrollmentToken()
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	sum := sha256.Sum256([]byte(token))
	principal := principalFromContext(request.Context())
	before, after, err := s.updateAgentEnrollmentPolicy(
		request.Context(),
		principal.User.ID,
		func(policy *agentEnrollmentPolicy) error {
			policy.Enabled = true
			policy.RequireToken = true
			policy.TokenHash = hex.EncodeToString(sum[:])
			return nil
		},
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	s.recordAdminAudit(
		request,
		"agent.enrollment_token.issue",
		"agent_enrollment_policy",
		agentEnrollmentPolicyKey,
		before.publicValue(),
		after.publicValue(),
		input.Reason,
	)
	payload := after.publicValue()
	payload["registration_token"] = token
	payload["shown_once"] = true
	response.Header().Set("Cache-Control", "no-store")
	writeJSON(response, http.StatusCreated, payload)
}

func (s *Server) deleteAgentEnrollmentToken(
	response http.ResponseWriter,
	request *http.Request,
) {
	var input struct {
		Reason string `json:"reason"`
	}
	if decodeJSON(request, &input) != nil {
		writeAPIError(
			response, request, http.StatusBadRequest,
			"INVALID_AGENT_ENROLLMENT_TOKEN_REQUEST",
			"The request body is invalid.",
		)
		return
	}
	principal := principalFromContext(request.Context())
	before, after, err := s.updateAgentEnrollmentPolicy(
		request.Context(),
		principal.User.ID,
		func(policy *agentEnrollmentPolicy) error {
			policy.TokenHash = ""
			policy.RequireToken = false
			return nil
		},
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	s.recordAdminAudit(
		request,
		"agent.enrollment_token.delete",
		"agent_enrollment_policy",
		agentEnrollmentPolicyKey,
		before.publicValue(),
		after.publicValue(),
		input.Reason,
	)
	writeJSON(response, http.StatusOK, after.publicValue())
}

func newAgentEnrollmentToken() (string, error) {
	bytes := make([]byte, 32)
	if _, err := rand.Read(bytes); err != nil {
		return "", fmt.Errorf("generate registration token: %w", err)
	}
	return "ivq_et_" + base64.RawURLEncoding.EncodeToString(bytes), nil
}
