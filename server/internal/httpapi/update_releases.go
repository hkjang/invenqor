package httpapi

import (
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"errors"
	"io"
	"net/http"
	"os"
	"strconv"
	"strings"

	"github.com/go-chi/chi/v5"
	"github.com/hkjang/invenqor/server/internal/updates"
)

// Publishing a build used to be the only update operation the API offered, which
// left an operator with no way to widen a rollout, stop a bad release, or see
// how far one had progressed - short of republishing with a new version number.
// These endpoints exist so the normal rollout workflow needs no re-upload.

type releaseView struct {
	updates.Release
	// Adopted counts agents already reporting this version, and Eligible how
	// many the current rollout percent can reach. Rollout progress is the
	// question an operator asks immediately after publishing.
	Adopted  int64 `json:"adopted_agents"`
	Eligible int64 `json:"eligible_agents"`
}

func (s *Server) listAgentUpdates(
	response http.ResponseWriter,
	request *http.Request,
) {
	if s.updateStore == nil {
		writeAPIError(
			response, request, http.StatusServiceUnavailable,
			"UPDATE_STORE_UNAVAILABLE", "The update store is unavailable.",
		)
		return
	}
	releases, err := s.updateStore.Releases()
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	versions, total, err := s.agentVersionCounts(request.Context())
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	views := make([]releaseView, 0, len(releases))
	for _, release := range releases {
		views = append(views, releaseView{
			Release:  release,
			Adopted:  versions[release.Version],
			Eligible: total * int64(release.Rollout) / 100,
		})
	}
	distribution := make([]statisticBucket, 0, len(versions))
	for version, count := range versions {
		label := version
		if label == "" {
			label = "미확인"
		}
		distribution = append(distribution, statisticBucket{
			Label: label, Count: count,
		})
	}
	sortBucketsByCount(distribution)
	writeJSON(response, http.StatusOK, map[string]any{
		"releases":            views,
		"agent_versions":      distribution,
		"agents":              total,
		"signature_verified":  s.updateStore.SigningKeyConfigured(),
		"agent_version":       "",
		"signing_key_missing": !s.updateStore.SigningKeyConfigured(),
	})
}

func (s *Server) agentVersionCounts(
	ctx context.Context,
) (map[string]int64, int64, error) {
	rows, err := s.database.DB().QueryContext(
		ctx,
		`SELECT COALESCE(NULLIF(version,''),'') AS version, COUNT(*)
		   FROM agents WHERE blocked_at IS NULL
		  GROUP BY version`,
	)
	if err != nil {
		return nil, 0, err
	}
	defer rows.Close()
	counts := map[string]int64{}
	var total int64
	for rows.Next() {
		var version string
		var count int64
		if err := rows.Scan(&version, &count); err != nil {
			return nil, 0, err
		}
		counts[version] = count
		total += count
	}
	return counts, total, rows.Err()
}

func sortBucketsByCount(buckets []statisticBucket) {
	for outer := 1; outer < len(buckets); outer++ {
		for index := outer; index > 0; index-- {
			left, right := buckets[index-1], buckets[index]
			if right.Count < left.Count ||
				(right.Count == left.Count && right.Label >= left.Label) {
				break
			}
			buckets[index-1], buckets[index] = right, left
		}
	}
}

func (s *Server) updateAgentUpdateRollout(
	response http.ResponseWriter,
	request *http.Request,
) {
	if s.updateStore == nil {
		writeAPIError(
			response, request, http.StatusServiceUnavailable,
			"UPDATE_STORE_UNAVAILABLE", "The update store is unavailable.",
		)
		return
	}
	base := chi.URLParam(request, "release")
	var input struct {
		RolloutPercent *int   `json:"rollout_percent"`
		Reason         string `json:"reason"`
	}
	if base == "" || decodeJSON(request, &input) != nil || input.RolloutPercent == nil {
		writeAPIError(
			response, request, http.StatusBadRequest,
			"INVALID_REQUEST", "rollout_percent is required.",
		)
		return
	}
	manifest, err := s.updateStore.SetRollout(base, *input.RolloutPercent)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			writeAPIError(
				response, request, http.StatusNotFound,
				"RELEASE_NOT_FOUND", "The release does not exist.",
			)
			return
		}
		writeAPIError(
			response, request, http.StatusBadRequest,
			"INVALID_ROLLOUT", err.Error(),
		)
		return
	}
	action := "agent_update.rollout"
	if *input.RolloutPercent == 0 {
		// Stopping a release is the action an operator takes under pressure, so it
		// gets its own audit verb.
		action = "agent_update.halt"
	}
	s.recordAdminAudit(
		request, action, "agent_update", base, nil,
		map[string]any{
			"version":         manifest.Version,
			"architecture":    manifest.Architecture,
			"rollout_percent": manifest.Rollout,
		},
		input.Reason,
	)
	writeJSON(response, http.StatusOK, map[string]any{"release": manifest})
}

func (s *Server) retireAgentUpdate(
	response http.ResponseWriter,
	request *http.Request,
) {
	if s.updateStore == nil {
		writeAPIError(
			response, request, http.StatusServiceUnavailable,
			"UPDATE_STORE_UNAVAILABLE", "The update store is unavailable.",
		)
		return
	}
	base := chi.URLParam(request, "release")
	if err := s.updateStore.Retire(base); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			writeAPIError(
				response, request, http.StatusNotFound,
				"RELEASE_NOT_FOUND", "The release does not exist.",
			)
			return
		}
		writeAPIError(
			response, request, http.StatusBadRequest,
			"INVALID_RELEASE", err.Error(),
		)
		return
	}
	s.recordAdminAudit(
		request, "agent_update.retire", "agent_update", base, nil,
		map[string]any{"retired": true}, "",
	)
	response.WriteHeader(http.StatusNoContent)
}

// publishOptions reads the fields that make publishing forgiving: a signature
// pasted with line breaks, a signature uploaded as a file, and an explicit
// rollback flag.
func publishOptions(request *http.Request) (string, bool, string, int, error) {
	signature := strings.TrimSpace(request.FormValue("signature"))
	if file, _, err := request.FormFile("signature_file"); err == nil {
		defer file.Close()
		contents, readErr := io.ReadAll(io.LimitReader(file, 4096))
		if readErr != nil {
			return "", false, "", 0, errors.New("the signature file could not be read")
		}
		// `openssl pkeyutl -sign` writes 64 raw bytes; asking an operator to
		// base64 it first is exactly the step that invites a mistake, so accept
		// the file the signing command actually produced.
		if len(contents) == ed25519.SignatureSize {
			signature = base64.StdEncoding.EncodeToString(contents)
		} else {
			signature = strings.TrimSpace(string(contents))
		}
	}
	rollout := 100
	if raw := strings.TrimSpace(request.FormValue("rollout_percent")); raw != "" {
		parsed, err := strconv.Atoi(raw)
		if err != nil {
			return "", false, "", 0, errors.New("rollout_percent must be a number")
		}
		rollout = parsed
	}
	allowDowngrade := strings.EqualFold(
		strings.TrimSpace(request.FormValue("allow_downgrade")), "true",
	)
	return signature, allowDowngrade, strings.TrimSpace(request.FormValue("notes")),
		rollout, nil
}
