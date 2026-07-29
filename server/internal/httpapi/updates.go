package httpapi

import (
	"errors"
	"fmt"
	"io"
	"net/http"
	"strconv"
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
	if manifest == nil || !newerVersion(
		r.URL.Query().Get("current_version"), manifest.Version,
	) {
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
	rollout, _ := strconv.Atoi(r.FormValue("rollout_percent"))
	manifest, err := s.updateStore.Publish(updates.Manifest{
		Version: r.FormValue("version"), Channel: r.FormValue("channel"),
		OS: r.FormValue("os"), Architecture: r.FormValue("architecture"),
		Signature: strings.TrimSpace(r.FormValue("signature")), Rollout: rollout,
	}, io.LimitReader(file, 128*1024*1024+1))
	if err != nil {
		writeAPIError(w, r, 400, "INVALID_UPDATE", err.Error())
		return
	}
	s.recordAdminAudit(r, "agent_update.publish", "agent_update",
		manifest.Version+"-"+manifest.Architecture, nil, manifest, "")
	writeJSON(w, 201, manifest)
}

func newerVersion(current, candidate string) bool {
	var ca, cb, cc, na, nb, nc int
	_, err1 := fmt.Sscanf(strings.TrimPrefix(current, "v"), "%d.%d.%d", &ca, &cb, &cc)
	_, err2 := fmt.Sscanf(strings.TrimPrefix(candidate, "v"), "%d.%d.%d", &na, &nb, &nc)
	return err1 == nil && err2 == nil &&
		(na > ca || na == ca && (nb > cb || nb == cb && nc > cc))
}
