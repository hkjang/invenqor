package httpapi

import (
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"

	"github.com/go-chi/chi/v5"
	"github.com/hkjang/invenqor/server/internal/agents"
	"github.com/hkjang/invenqor/server/internal/updates"
)

func (s *Server) agentUpdateManifest(w http.ResponseWriter, r *http.Request) {
	agent, err := s.authenticateAgent(r)
	if errors.Is(err, agents.ErrUnauthorized) {
		writeAPIError(w, r, 401, "AGENT_UNAUTHORIZED", "The agent credential is invalid.")
		return
	}
	if err != nil {
		s.internalError(w, r, err)
		return
	}
	if s.updateStore == nil {
		writeAPIError(w, r, 503, "UPDATE_STORE_UNAVAILABLE", "The update store is unavailable.")
		return
	}
	if r.URL.Query().Get("agent_id") != agent.AgentID {
		writeAPIError(w, r, 403, "AGENT_ID_MISMATCH", "The agent identity does not match.")
		return
	}
	manifest, err := s.updateStore.Latest(
		r.URL.Query().Get("channel"),
		r.URL.Query().Get("os"),
		r.URL.Query().Get("arch"),
		agent.AgentID,
	)
	if err != nil {
		s.internalError(w, r, err)
		return
	}
	current := r.URL.Query().Get("current_version")
	// A rollback is the one case where "not newer" is the intent. It stays safe
	// because the artifact is signed and hash-checked either way.
	if manifest == nil ||
		(!newerVersion(current, manifest.Version) &&
			!(manifest.AllowDowngrade && manifest.Version != current)) {
		w.WriteHeader(http.StatusNoContent)
		return
	}
	w.Header().Set("Cache-Control", "no-store")
	writeJSON(w, 200, manifest)
}

func (s *Server) agentUpdateArtifact(w http.ResponseWriter, r *http.Request) {
	if _, err := s.authenticateAgent(r); err != nil {
		writeAPIError(w, r, 401, "AGENT_UNAUTHORIZED", "The agent credential is invalid.")
		return
	}
	if s.updateStore == nil {
		writeAPIError(w, r, 503, "UPDATE_STORE_UNAVAILABLE", "The update store is unavailable.")
		return
	}
	path, err := s.updateStore.Artifact(chi.URLParam(r, "artifact"))
	if err != nil {
		writeAPIError(w, r, 404, "UPDATE_NOT_FOUND", "The update artifact does not exist.")
		return
	}
	w.Header().Set("Content-Type", "application/octet-stream")
	w.Header().Set("Cache-Control", "private, max-age=300")
	http.ServeFile(w, r, path)
}

func (s *Server) publishAgentUpdate(w http.ResponseWriter, r *http.Request) {
	if s.updateStore == nil {
		writeAPIError(w, r, 503, "UPDATE_STORE_UNAVAILABLE", "The update store is unavailable.")
		return
	}
	r.Body = http.MaxBytesReader(w, r.Body, 130*1024*1024)
	if err := r.ParseMultipartForm(130 * 1024 * 1024); err != nil {
		writeAPIError(w, r, 400, "INVALID_UPDATE", "The multipart update is invalid.")
		return
	}
	file, _, err := r.FormFile("artifact")
	if err != nil {
		writeAPIError(w, r, 400, "MISSING_ARTIFACT", "The artifact file is required.")
		return
	}
	defer file.Close()
	signature, allowDowngrade, notes, rollout, err := publishOptions(r)
	if err != nil {
		writeAPIError(w, r, 400, "INVALID_UPDATE", err.Error())
		return
	}
	// Default the platform fields: a publisher who omits them means "linux, this
	// architecture", and making them mandatory only invited typos.
	osName := strings.TrimSpace(r.FormValue("os"))
	if osName == "" {
		osName = "linux"
	}
	channel := strings.TrimSpace(r.FormValue("channel"))
	if channel == "" {
		channel = "stable"
	}
	manifest, err := s.updateStore.Publish(updates.Manifest{
		Version:        strings.TrimSpace(r.FormValue("version")),
		Channel:        channel,
		OS:             osName,
		Architecture:   strings.TrimSpace(r.FormValue("architecture")),
		Signature:      signature,
		Rollout:        rollout,
		AllowDowngrade: allowDowngrade,
		Notes:          notes,
		PublishedBy:    principalFromContext(r.Context()).User.Username,
	}, io.LimitReader(file, 128*1024*1024+1))
	if err != nil {
		// A rejected signature is the one failure worth its own code: it means the
		// operator signed the wrong file or pasted the wrong value, and every
		// agent would otherwise have failed silently.
		code := "INVALID_UPDATE"
		status := 400
		switch {
		case errors.Is(err, updates.ErrSignatureRejected):
			code = "UPDATE_SIGNATURE_REJECTED"
		case errors.Is(err, updates.ErrSignatureUnverifiable):
			code = "UPDATE_SIGNING_KEY_MISSING"
			status = 503
		}
		writeAPIError(w, r, status, code, err.Error())
		return
	}
	s.recordAdminAudit(r, "agent_update.publish", "agent_update",
		manifest.Version+"-"+manifest.Architecture, nil, manifest, notes)
	writeJSON(w, 201, manifest)
}

func newerVersion(current, candidate string) bool {
	var ca, cb, cc, na, nb, nc int
	_, err1 := fmt.Sscanf(strings.TrimPrefix(current, "v"), "%d.%d.%d", &ca, &cb, &cc)
	_, err2 := fmt.Sscanf(strings.TrimPrefix(candidate, "v"), "%d.%d.%d", &na, &nb, &nc)
	return err1 == nil && err2 == nil &&
		(na > ca || na == ca && (nb > cb || nb == cb && nc > cc))
}
