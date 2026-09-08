package httpapi

import (
	"net/http"

	"github.com/hkjang/invenqor/server/internal/querydsl"
	"github.com/hkjang/invenqor/server/internal/storage"
)

type queryInput struct {
	Query string `json:"query"`
	Limit int    `json:"limit,omitempty"`
}

// queryGrammar publishes the field and operator list so the console can show
// what is writable instead of leaving an operator to guess and read rejections.
func (s *Server) queryGrammar(response http.ResponseWriter, request *http.Request) {
	writeJSON(response, http.StatusOK, querydsl.Describe())
}

func (s *Server) validateQuery(response http.ResponseWriter, request *http.Request) {
	var input queryInput
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, 400, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	query, err := querydsl.Parse(input.Query)
	if err != nil {
		writeJSON(response, 200, map[string]any{
			"valid": false, "error": err.Error(),
		})
		return
	}
	// Compiling is part of the answer. Parsing alone accepts a clause whose
	// value the compiler still refuses - an unreadable time, a confidence that
	// is not a number - and calling that valid sends the operator to execute
	// with an expression that is about to be rejected there.
	if _, _, err := query.SQL(
		s.database.Mode() != storage.ModeSQLiteFallback,
	); err != nil {
		writeJSON(response, 200, map[string]any{
			"valid": false, "error": err.Error(),
		})
		return
	}
	writeJSON(response, 200, map[string]any{"valid": true, "ast": query})
}

func (s *Server) executeQuery(response http.ResponseWriter, request *http.Request) {
	var input queryInput
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, 400, "INVALID_REQUEST", "The request body is invalid.")
		return
	}
	query, err := querydsl.Parse(input.Query)
	if err != nil {
		writeAPIError(response, request, 400, "INVALID_QUERY", err.Error())
		return
	}
	where, args, err := query.SQL(
		s.database.Mode() != storage.ModeSQLiteFallback,
	)
	if err != nil {
		writeAPIError(response, request, 400, "INVALID_QUERY", err.Error())
		return
	}
	limit := input.Limit
	if limit <= 0 {
		limit = 100
	}
	if limit > 500 {
		limit = 500
	}
	// One row past the limit, so a result that ends exactly on it is not
	// reported as cut short. See the truncation note below.
	args = append(args, limit+1)
	rows, err := s.database.DB().QueryContext(
		request.Context(),
		`SELECT `+assetColumns+` FROM assets WHERE `+where+
			` ORDER BY last_seen_at DESC LIMIT $`+itoa(len(args)),
		args...,
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	defer rows.Close()
	items := make([]assetView, 0)
	for rows.Next() {
		asset, err := scanAsset(rows)
		if err != nil {
			s.internalError(response, request, err)
			return
		}
		items = append(items, asset)
	}
	// The result is bounded, and whatever fitted used to be returned as though
	// it were the whole answer: "last_seen_at < \"now - 720h\"" over 1,200
	// stale hosts answered with 100 items and count 100, and neither the
	// response nor the audit entry said the other 1,100 existed. There is no
	// offset here either, so the flag is the only way a caller can learn the
	// question has more answers than it was given - and an API key reading
	// this endpoint from a script has no console to notice it in.
	truncated := len(items) > limit
	if truncated {
		items = items[:limit]
	}
	s.recordAdminAudit(
		request, "query.execute", "query", "", nil,
		map[string]any{
			"dsl": input.Query, "result_count": len(items),
			"truncated": truncated,
		}, "",
	)
	writeJSON(response, 200, map[string]any{
		"items": items, "count": len(items), "ast": query,
		"limit": limit, "truncated": truncated,
	})
}

func itoa(value int) string {
	if value == 0 {
		return "0"
	}
	buffer := [20]byte{}
	index := len(buffer)
	for value > 0 {
		index--
		buffer[index] = byte('0' + value%10)
		value /= 10
	}
	return string(buffer[index:])
}
