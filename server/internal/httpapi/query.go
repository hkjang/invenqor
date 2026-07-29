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
	args = append(args, limit)
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
	s.recordAdminAudit(
		request, "query.execute", "query", "", nil,
		map[string]any{"dsl": input.Query, "result_count": len(items)}, "",
	)
	writeJSON(response, 200, map[string]any{
		"items": items, "count": len(items), "ast": query,
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
