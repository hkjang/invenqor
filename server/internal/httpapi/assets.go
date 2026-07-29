package httpapi

import (
	"database/sql"
	"encoding/csv"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/go-chi/chi/v5"
	"github.com/go-chi/chi/v5/middleware"
	"github.com/google/uuid"
	"github.com/hkjang/invenqor/server/internal/apitime"
	"github.com/hkjang/invenqor/server/internal/audit"
	"github.com/hkjang/invenqor/server/internal/classify"
)

type assetView struct {
	ID              string          `json:"id"`
	AssetKey        string          `json:"asset_key"`
	Name            string          `json:"name"`
	Type            string          `json:"type"`
	Status          string          `json:"status"`
	Criticality     string          `json:"criticality"`
	Environment     string          `json:"environment"`
	OwnerDepartment string          `json:"owner_department"`
	Location        string          `json:"location"`
	Confidence      float64         `json:"confidence"`
	Attributes      json.RawMessage `json:"attributes"`
	CustomFields    json.RawMessage `json:"custom_fields"`
	Source          string          `json:"source"`
	FirstSeenAt     apitime.Time    `json:"first_seen_at"`
	LastSeenAt      apitime.Time    `json:"last_seen_at"`
	DeletedAt       apitime.Time    `json:"deleted_at,omitempty"`
}

const assetColumns = `id, asset_key, name, type, status, criticality,
	environment, owner_department, location, confidence, attributes_json,
	custom_fields_json, source, first_seen_at, last_seen_at, deleted_at`

func scanAsset(scanner interface{ Scan(...any) error }) (assetView, error) {
	var asset assetView
	var attributes, customFields string
	err := scanner.Scan(
		&asset.ID, &asset.AssetKey, &asset.Name, &asset.Type, &asset.Status,
		&asset.Criticality, &asset.Environment, &asset.OwnerDepartment,
		&asset.Location, &asset.Confidence, &attributes, &customFields,
		&asset.Source, &asset.FirstSeenAt, &asset.LastSeenAt, &asset.DeletedAt,
	)
	asset.Attributes = json.RawMessage(attributes)
	asset.CustomFields = json.RawMessage(customFields)
	return asset, err
}

// assetListFilter is shared by the listing and its CSV export so a download
// always contains exactly the rows the operator was looking at.
type assetListFilter struct {
	Search         string
	Type           string
	Status         string
	Environment    string
	Criticality    string
	Owner          string
	IncludeDeleted bool
	Sort           string
	Limit          int
	Offset         int
}

// Environment and criticality became meaningful once classification started
// setting them, but the listing could not filter on either, so "show me every
// critical production asset" - the question the classification work exists to
// answer - had to be asked through the query DSL instead.
func parseAssetListFilter(request *http.Request) assetListFilter {
	values := request.URL.Query()
	return assetListFilter{
		Search:         strings.TrimSpace(values.Get("q")),
		Type:           strings.TrimSpace(values.Get("type")),
		Status:         strings.TrimSpace(values.Get("status")),
		Environment:    strings.TrimSpace(values.Get("environment")),
		Criticality:    strings.TrimSpace(values.Get("criticality")),
		Owner:          strings.TrimSpace(values.Get("owner_department")),
		IncludeDeleted: values.Get("include_deleted") == "true",
		Sort:           strings.TrimSpace(values.Get("sort")),
		Limit:          queryInt(request, "limit", 50, 1, 200),
		Offset:         queryInt(request, "offset", 0, 0, 1_000_000),
	}
}

func (filter assetListFilter) where() (string, []any) {
	statement := " WHERE 1=1"
	arguments := make([]any, 0, 8)
	add := func(clause string, value any) {
		arguments = append(arguments, value)
		statement += fmt.Sprintf(clause, len(arguments))
	}
	if filter.Search != "" {
		add(
			" AND (LOWER(name) LIKE $%[1]d OR LOWER(asset_key) LIKE $%[1]d)",
			"%"+strings.ToLower(filter.Search)+"%",
		)
	}
	if filter.Type != "" {
		add(" AND type = $%d", filter.Type)
	}
	if filter.Status != "" {
		add(" AND status = $%d", filter.Status)
	}
	if filter.Environment != "" {
		add(" AND environment = $%d", filter.Environment)
	}
	if filter.Criticality != "" {
		add(" AND criticality = $%d", filter.Criticality)
	}
	if filter.Owner != "" {
		add(
			" AND LOWER(owner_department) LIKE $%d",
			"%"+strings.ToLower(filter.Owner)+"%",
		)
	}
	if !filter.IncludeDeleted {
		statement += " AND deleted_at IS NULL"
	}
	return statement, arguments
}

// assetSortOrders is a fixed allowlist: the value reaches SQL directly, so it
// can never be caller-supplied text.
var assetSortOrders = map[string]string{
	"":            "last_seen_at DESC, id",
	"recent":      "last_seen_at DESC, id",
	"oldest":      "last_seen_at, id",
	"name":        "LOWER(name), id",
	"type":        "type, LOWER(name), id",
	"criticality": "criticality, LOWER(name), id",
	"discovered":  "first_seen_at DESC, id",
}

func (filter assetListFilter) orderBy() string {
	if order, found := assetSortOrders[filter.Sort]; found {
		return order
	}
	return assetSortOrders[""]
}

func (s *Server) assetRows(
	request *http.Request,
	filter assetListFilter,
) ([]assetView, error) {
	where, arguments := filter.where()
	arguments = append(arguments, filter.Limit, filter.Offset)
	statement := "SELECT " + assetColumns + " FROM assets" + where +
		fmt.Sprintf(
			" ORDER BY %s LIMIT $%d OFFSET $%d",
			filter.orderBy(), len(arguments)-1, len(arguments),
		)
	rows, err := s.database.DB().QueryContext(
		request.Context(), statement, arguments...,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]assetView, 0, filter.Limit)
	for rows.Next() {
		asset, err := scanAsset(rows)
		if err != nil {
			return nil, err
		}
		items = append(items, asset)
	}
	return items, rows.Err()
}

func (s *Server) listAssets(response http.ResponseWriter, request *http.Request) {
	filter := parseAssetListFilter(request)
	items, err := s.assetRows(request, filter)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	where, arguments := filter.where()
	// The console showed "50개 자산 · offset 0", which says nothing about how
	// large the result actually is. The count is one cheap query away.
	var total int64
	if err := s.database.DB().QueryRowContext(
		request.Context(),
		"SELECT COUNT(*) FROM assets"+where,
		arguments...,
	).Scan(&total); err != nil {
		s.internalError(response, request, err)
		return
	}
	writeJSON(response, 200, map[string]any{
		"items": items, "limit": filter.Limit, "offset": filter.Offset,
		"total":    total,
		"has_more": int64(filter.Offset+len(items)) < total,
	})
}

// exportAssets writes the current filter as CSV. Inventory extracts are asked
// for constantly - for a review, a ticket, a hand-off - and the alternative is
// an operator copying a paged table by hand.
func (s *Server) exportAssets(response http.ResponseWriter, request *http.Request) {
	filter := parseAssetListFilter(request)
	filter.Offset = 0
	filter.Limit = queryInt(request, "limit", 10_000, 1, 100_000)
	items, err := s.assetRows(request, filter)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	response.Header().Set("Content-Type", "text/csv; charset=utf-8")
	response.Header().Set(
		"Content-Disposition",
		`attachment; filename="invenqor-assets.csv"`,
	)
	_, _ = response.Write([]byte{0xEF, 0xBB, 0xBF})
	writer := csv.NewWriter(response)
	_ = writer.Write([]string{
		"asset_key", "name", "type", "status", "environment", "criticality",
		"owner_department", "location", "source", "confidence",
		"first_seen_at", "last_seen_at",
	})
	for _, item := range items {
		_ = writer.Write([]string{
			item.AssetKey, item.Name, item.Type, item.Status, item.Environment,
			item.Criticality, item.OwnerDepartment, item.Location, item.Source,
			strconv.FormatFloat(item.Confidence, 'f', 2, 64),
			item.FirstSeenAt.String(), item.LastSeenAt.String(),
		})
	}
	writer.Flush()
}

func (s *Server) getAsset(response http.ResponseWriter, request *http.Request) {
	asset, err := scanAsset(s.database.DB().QueryRowContext(
		request.Context(),
		`SELECT `+assetColumns+` FROM assets WHERE id = $1`,
		chi.URLParam(request, "assetID"),
	))
	if errors.Is(err, sql.ErrNoRows) {
		writeAPIError(response, request, 404, "ASSET_NOT_FOUND", "The asset does not exist.")
		return
	}
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	rows, err := s.database.DB().QueryContext(
		request.Context(),
		`SELECT id, agent_id, category, source_asset_id, source_name,
		 payload_json, collected_at, first_seen_at, last_seen_at, deleted_at
		 FROM asset_sources WHERE asset_id = $1 ORDER BY first_seen_at`,
		asset.ID,
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	defer rows.Close()
	sources := make([]map[string]any, 0)
	for rows.Next() {
		var id, agentID, category, sourceID, sourceName, payload string
		var collected, firstSeen, lastSeen, deleted any
		if err := rows.Scan(
			&id, &agentID, &category, &sourceID, &sourceName, &payload,
			&collected, &firstSeen, &lastSeen, &deleted,
		); err != nil {
			s.internalError(response, request, err)
			return
		}
		sources = append(sources, map[string]any{
			"id": id, "agent_id": agentID, "category": category,
			"source_asset_id": sourceID, "source_name": sourceName,
			"payload": json.RawMessage(payload), "collected_at": apiTime(collected),
			"first_seen_at": apiTime(firstSeen), "last_seen_at": apiTime(lastSeen),
			"deleted_at": apiTime(deleted),
		})
	}
	writeJSON(response, 200, map[string]any{"asset": asset, "sources": sources})
}

type assetMutation struct {
	AssetKey        *string         `json:"asset_key,omitempty"`
	Name            *string         `json:"name,omitempty"`
	Type            *string         `json:"type,omitempty"`
	Status          *string         `json:"status,omitempty"`
	Criticality     *string         `json:"criticality,omitempty"`
	Environment     *string         `json:"environment,omitempty"`
	OwnerDepartment *string         `json:"owner_department,omitempty"`
	Location        *string         `json:"location,omitempty"`
	Attributes      json.RawMessage `json:"attributes,omitempty"`
	CustomFields    json.RawMessage `json:"custom_fields,omitempty"`
	Reason          string          `json:"reason,omitempty"`
}

func (s *Server) createAsset(response http.ResponseWriter, request *http.Request) {
	var input assetMutation
	if err := decodeJSON(request, &input); err != nil ||
		input.Name == nil || input.Type == nil {
		writeAPIError(response, request, 400, "INVALID_ASSET", "name and type are required.")
		return
	}
	id := uuid.NewString()
	key := id
	if input.AssetKey != nil && strings.TrimSpace(*input.AssetKey) != "" {
		key = strings.TrimSpace(*input.AssetKey)
	}
	attributes := jsonValueOr(input.Attributes, "{}")
	customFields := jsonValueOr(input.CustomFields, "{}")
	now := time.Now().UTC()
	_, err := s.database.DB().ExecContext(
		request.Context(),
		`INSERT INTO assets(
			id, asset_key, name, type, status, criticality, environment,
			owner_department, location, attributes_json, custom_fields_json,
			source, first_seen_at, last_seen_at, created_at, updated_at
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
		 'manual', $12, $12, $12, $12)`,
		id, key, strings.TrimSpace(*input.Name), strings.TrimSpace(*input.Type),
		stringOr(input.Status, "active"), stringOr(input.Criticality, "normal"),
		stringOr(input.Environment, "other"), stringOr(input.OwnerDepartment, ""),
		stringOr(input.Location, ""), attributes, customFields, now,
	)
	if err != nil {
		writeAPIError(response, request, 409, "ASSET_CONFLICT", "The asset key already exists.")
		return
	}
	_, _ = s.database.DB().ExecContext(
		request.Context(),
		`INSERT INTO asset_changes(
			id, asset_id, change_type, after_json, actor_type, actor_id, reason
		) VALUES ($1, $2, 'created', $3, 'user', $4, $5)`,
		uuid.NewString(), id, attributes,
		principalFromContext(request.Context()).User.ID, input.Reason,
	)
	s.recordAdminAudit(request, "asset.create", "asset", id, nil, input, input.Reason)
	asset, _ := scanAsset(s.database.DB().QueryRow(
		`SELECT `+assetColumns+` FROM assets WHERE id = $1`, id,
	))
	writeJSON(response, 201, map[string]any{"asset": asset})
}

func (s *Server) updateAsset(response http.ResponseWriter, request *http.Request) {
	id := chi.URLParam(request, "assetID")
	before, err := scanAsset(s.database.DB().QueryRowContext(
		request.Context(), `SELECT `+assetColumns+` FROM assets WHERE id = $1`, id,
	))
	if errors.Is(err, sql.ErrNoRows) {
		writeAPIError(response, request, 404, "ASSET_NOT_FOUND", "The asset does not exist.")
		return
	}
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	var input assetMutation
	if err := decodeJSON(request, &input); err != nil {
		writeAPIError(response, request, 400, "INVALID_ASSET", "The request body is invalid.")
		return
	}
	name := before.Name
	if input.Name != nil {
		name = strings.TrimSpace(*input.Name)
	}
	status := before.Status
	if input.Status != nil {
		status = *input.Status
	}
	criticality := before.Criticality
	if input.Criticality != nil {
		criticality = *input.Criticality
	}
	environment := before.Environment
	if input.Environment != nil {
		environment = *input.Environment
	}
	department := before.OwnerDepartment
	if input.OwnerDepartment != nil {
		department = *input.OwnerDepartment
	}
	location := before.Location
	if input.Location != nil {
		location = *input.Location
	}
	attributes := string(before.Attributes)
	if len(input.Attributes) > 0 {
		attributes = jsonValueOr(input.Attributes, "{}")
	}
	customFields := string(before.CustomFields)
	if len(input.CustomFields) > 0 {
		customFields = jsonValueOr(input.CustomFields, "{}")
	}
	// Whatever a person edits here becomes theirs: the classification rules must
	// never move it back on the next inventory event.
	claimed := make([]string, 0, 5)
	for field, changed := range map[string]bool{
		"type":             input.Type != nil,
		"criticality":      input.Criticality != nil,
		"environment":      input.Environment != nil,
		"owner_department": input.OwnerDepartment != nil,
		"location":         input.Location != nil,
	} {
		if changed {
			claimed = append(claimed, field)
		}
	}
	sort.Strings(claimed)
	now := time.Now().UTC()
	transaction, err := s.database.DB().BeginTx(request.Context(), nil)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	defer transaction.Rollback()
	if err := classify.MarkManual(request.Context(), transaction, id, claimed); err != nil {
		s.internalError(response, request, err)
		return
	}
	_, err = transaction.ExecContext(
		request.Context(),
		`UPDATE assets SET name=$1, status=$2, criticality=$3,
		 environment=$4, owner_department=$5, location=$6,
		 attributes_json=$7, custom_fields_json=$8, updated_at=$9
		 WHERE id=$10`,
		name, status, criticality, environment, department, location,
		attributes, customFields, now, id,
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	if err := transaction.Commit(); err != nil {
		s.internalError(response, request, err)
		return
	}
	after, _ := scanAsset(s.database.DB().QueryRow(
		`SELECT `+assetColumns+` FROM assets WHERE id = $1`, id,
	))
	beforeJSON, _ := json.Marshal(before)
	afterJSON, _ := json.Marshal(after)
	_, _ = s.database.DB().ExecContext(
		request.Context(),
		`INSERT INTO asset_changes(
			id, asset_id, change_type, before_json, after_json,
			actor_type, actor_id, reason
		) VALUES ($1,$2,'updated',$3,$4,'user',$5,$6)`,
		uuid.NewString(), id, string(beforeJSON), string(afterJSON),
		principalFromContext(request.Context()).User.ID, input.Reason,
	)
	s.recordAdminAudit(request, "asset.update", "asset", id, before, after, input.Reason)
	writeJSON(response, 200, map[string]any{"asset": after})
}

func (s *Server) deleteAsset(response http.ResponseWriter, request *http.Request) {
	s.setAssetDeleted(response, request, true)
}

func (s *Server) restoreAsset(response http.ResponseWriter, request *http.Request) {
	s.setAssetDeleted(response, request, false)
}

func (s *Server) setAssetDeleted(
	response http.ResponseWriter, request *http.Request, deleted bool,
) {
	id := chi.URLParam(request, "assetID")
	now := time.Now().UTC()
	var deletedAt any
	status, action, change := "active", "asset.restore", "restored"
	if deleted {
		deletedAt, status, action, change = now, "deleted", "asset.delete", "deleted"
	}
	result, err := s.database.DB().ExecContext(
		request.Context(),
		`UPDATE assets SET deleted_at=$1, status=$2, updated_at=$3 WHERE id=$4`,
		deletedAt, status, now, id,
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	if count, _ := result.RowsAffected(); count == 0 {
		writeAPIError(response, request, 404, "ASSET_NOT_FOUND", "The asset does not exist.")
		return
	}
	_, _ = s.database.DB().ExecContext(
		request.Context(),
		`INSERT INTO asset_changes(
			id,asset_id,change_type,actor_type,actor_id
		) VALUES($1,$2,$3,'user',$4)`,
		uuid.NewString(), id, change, principalFromContext(request.Context()).User.ID,
	)
	s.recordAdminAudit(request, action, "asset", id, nil, map[string]any{"deleted": deleted}, "")
	writeJSON(response, 200, map[string]any{"deleted": deleted})
}

func (s *Server) assetHistory(response http.ResponseWriter, request *http.Request) {
	rows, err := s.database.DB().QueryContext(
		request.Context(),
		`SELECT id, change_type, before_json, after_json, actor_type,
		 actor_id, reason, occurred_at FROM asset_changes
		 WHERE asset_id=$1 ORDER BY occurred_at DESC`,
		chi.URLParam(request, "assetID"),
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	defer rows.Close()
	items := make([]map[string]any, 0)
	for rows.Next() {
		var id, changeType, actorType, reason string
		var before, after, actorID, occurred any
		if err := rows.Scan(&id, &changeType, &before, &after, &actorType, &actorID, &reason, &occurred); err != nil {
			s.internalError(response, request, err)
			return
		}
		items = append(items, map[string]any{
			"id": id, "change_type": changeType, "before": rawJSON(before),
			"after": rawJSON(after), "actor_type": actorType,
			"actor_id": actorID, "reason": reason, "occurred_at": apiTime(occurred),
		})
	}
	writeJSON(response, 200, map[string]any{"items": items})
}

func (s *Server) assetRelations(response http.ResponseWriter, request *http.Request) {
	id := chi.URLParam(request, "assetID")
	rows, err := s.database.DB().QueryContext(
		request.Context(),
		`SELECT r.id,r.source_asset_id,r.relation_type,r.target_asset_id,
		 r.valid_from,r.valid_to,r.source,r.confidence,
		 sa.name,sa.type,ta.name,ta.type
		 FROM asset_relations r
		 JOIN assets sa ON sa.id=r.source_asset_id
		 JOIN assets ta ON ta.id=r.target_asset_id
		 WHERE (r.source_asset_id=$1 OR r.target_asset_id=$1)
		   AND r.valid_to IS NULL`,
		id,
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	defer rows.Close()
	items := make([]map[string]any, 0)
	for rows.Next() {
		var relationID, sourceID, relationType, targetID, source, sourceName, sourceType, targetName, targetType string
		var validFrom, validTo any
		var confidence float64
		if err := rows.Scan(
			&relationID, &sourceID, &relationType, &targetID, &validFrom,
			&validTo, &source, &confidence, &sourceName, &sourceType,
			&targetName, &targetType,
		); err != nil {
			s.internalError(response, request, err)
			return
		}
		items = append(items, map[string]any{
			"id": relationID, "source_asset_id": sourceID,
			"relation_type": relationType, "target_asset_id": targetID,
			"valid_from": validFrom, "source": source, "confidence": confidence,
			"source_asset": map[string]any{"name": sourceName, "type": sourceType},
			"target_asset": map[string]any{"name": targetName, "type": targetType},
		})
	}
	writeJSON(response, 200, map[string]any{"items": items})
}

func (s *Server) createAssetRelation(response http.ResponseWriter, request *http.Request) {
	var input struct {
		TargetID     string  `json:"target_asset_id"`
		RelationType string  `json:"relation_type"`
		Confidence   float64 `json:"confidence"`
		Reason       string  `json:"reason"`
	}
	if err := decodeJSON(request, &input); err != nil ||
		input.TargetID == "" || input.RelationType == "" {
		writeAPIError(response, request, 400, "INVALID_RELATION", "target_asset_id and relation_type are required.")
		return
	}
	if input.Confidence == 0 {
		input.Confidence = 1
	}
	id := uuid.NewString()
	_, err := s.database.DB().ExecContext(
		request.Context(),
		`INSERT INTO asset_relations(
			id,source_asset_id,relation_type,target_asset_id,source,confidence
		) VALUES($1,$2,$3,$4,'manual',$5)`,
		id, chi.URLParam(request, "assetID"), input.RelationType,
		input.TargetID, input.Confidence,
	)
	if err != nil {
		writeAPIError(response, request, 409, "RELATION_CONFLICT", "The relation could not be created.")
		return
	}
	s.recordAdminAudit(request, "relation.create", "asset_relation", id, nil, input, input.Reason)
	writeJSON(response, 201, map[string]any{"id": id})
}

func (s *Server) deleteAssetRelation(response http.ResponseWriter, request *http.Request) {
	id := chi.URLParam(request, "relationID")
	result, err := s.database.DB().ExecContext(
		request.Context(),
		`UPDATE asset_relations SET valid_to=$1
		 WHERE id=$2 AND valid_to IS NULL`,
		time.Now().UTC(), id,
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	if count, _ := result.RowsAffected(); count == 0 {
		writeAPIError(response, request, 404, "RELATION_NOT_FOUND", "The relation does not exist.")
		return
	}
	s.recordAdminAudit(request, "relation.delete", "asset_relation", id, nil, nil, "")
	writeJSON(response, 200, map[string]any{"deleted": true})
}

func (s *Server) mergeAssets(response http.ResponseWriter, request *http.Request) {
	var input struct {
		PrimaryID    string   `json:"primary_id"`
		SecondaryIDs []string `json:"secondary_ids"`
		Reason       string   `json:"reason"`
	}
	if err := decodeJSON(request, &input); err != nil ||
		input.PrimaryID == "" || len(input.SecondaryIDs) == 0 {
		writeAPIError(response, request, 400, "INVALID_MERGE", "primary_id and secondary_ids are required.")
		return
	}
	tx, err := s.database.DB().BeginTx(request.Context(), nil)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	defer tx.Rollback()
	moved := make([]string, 0)
	for _, secondaryID := range input.SecondaryIDs {
		if secondaryID == input.PrimaryID {
			continue
		}
		rows, err := tx.QueryContext(
			request.Context(), `SELECT id FROM asset_sources WHERE asset_id=$1`, secondaryID,
		)
		if err != nil {
			s.internalError(response, request, err)
			return
		}
		for rows.Next() {
			var sourceID string
			_ = rows.Scan(&sourceID)
			moved = append(moved, sourceID)
		}
		rows.Close()
		if _, err := tx.ExecContext(
			request.Context(), `UPDATE asset_sources SET asset_id=$1 WHERE asset_id=$2`,
			input.PrimaryID, secondaryID,
		); err != nil {
			s.internalError(response, request, err)
			return
		}
		if _, err := tx.ExecContext(
			request.Context(),
			`UPDATE assets SET status='merged',deleted_at=$1,updated_at=$1 WHERE id=$2`,
			time.Now().UTC(), secondaryID,
		); err != nil {
			s.internalError(response, request, err)
			return
		}
	}
	metadata, _ := json.Marshal(map[string]any{
		"secondary_ids": input.SecondaryIDs, "source_ids": moved,
	})
	_, err = tx.ExecContext(
		request.Context(),
		`INSERT INTO asset_changes(
			id,asset_id,change_type,after_json,actor_type,actor_id,reason
		) VALUES($1,$2,'merged',$3,'user',$4,$5)`,
		uuid.NewString(), input.PrimaryID, string(metadata),
		principalFromContext(request.Context()).User.ID, input.Reason,
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	if err := (audit.Recorder{}).Record(request.Context(), tx, audit.Entry{
		ActorType: "user", ActorID: principalFromContext(request.Context()).User.ID,
		Action: "asset.merge", ResourceType: "asset", ResourceID: input.PrimaryID,
		Result: "success", Reason: input.Reason, After: json.RawMessage(metadata),
	}); err != nil {
		s.internalError(response, request, err)
		return
	}
	if err := tx.Commit(); err != nil {
		s.internalError(response, request, err)
		return
	}
	writeJSON(response, 200, map[string]any{"merged": true, "source_ids": moved})
}

func (s *Server) splitAsset(response http.ResponseWriter, request *http.Request) {
	var input struct {
		SourceIDs []string `json:"source_ids"`
		Name      string   `json:"name"`
		Type      string   `json:"type"`
		Reason    string   `json:"reason"`
	}
	if err := decodeJSON(request, &input); err != nil ||
		len(input.SourceIDs) == 0 || input.Name == "" || input.Type == "" {
		writeAPIError(response, request, 400, "INVALID_SPLIT", "source_ids, name and type are required.")
		return
	}
	originalID := chi.URLParam(request, "assetID")
	newID := uuid.NewString()
	now := time.Now().UTC()
	tx, err := s.database.DB().BeginTx(request.Context(), nil)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	defer tx.Rollback()
	_, err = tx.ExecContext(
		request.Context(),
		`INSERT INTO assets(
			id,asset_key,name,type,status,source,first_seen_at,last_seen_at,
		 created_at,updated_at
		) VALUES($1,$2,$3,$4,'active','manual',$5,$5,$5,$5)`,
		newID, "split:"+newID, input.Name, input.Type, now,
	)
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	for _, sourceID := range input.SourceIDs {
		result, err := tx.ExecContext(
			request.Context(),
			`UPDATE asset_sources SET asset_id=$1
			 WHERE id=$2 AND asset_id=$3`,
			newID, sourceID, originalID,
		)
		if err != nil {
			s.internalError(response, request, err)
			return
		}
		if count, _ := result.RowsAffected(); count == 0 {
			writeAPIError(response, request, 400, "INVALID_SOURCE", "A source does not belong to the original asset.")
			return
		}
	}
	metadata, _ := json.Marshal(input.SourceIDs)
	_, err = tx.ExecContext(
		request.Context(),
		`INSERT INTO asset_changes(
			id,asset_id,change_type,after_json,actor_type,actor_id,reason
		) VALUES($1,$2,'split',$3,'user',$4,$5)`,
		uuid.NewString(), originalID, string(metadata),
		principalFromContext(request.Context()).User.ID, input.Reason,
	)
	if err == nil {
		err = (audit.Recorder{}).Record(request.Context(), tx, audit.Entry{
			ActorType: "user", ActorID: principalFromContext(request.Context()).User.ID,
			Action: "asset.split", ResourceType: "asset", ResourceID: originalID,
			Result: "success", Reason: input.Reason,
			After: map[string]any{"new_asset_id": newID, "source_ids": input.SourceIDs},
		})
	}
	if err != nil {
		s.internalError(response, request, err)
		return
	}
	if err := tx.Commit(); err != nil {
		s.internalError(response, request, err)
		return
	}
	writeJSON(response, 201, map[string]any{"asset_id": newID})
}

func (s *Server) recordAdminAudit(
	request *http.Request,
	action string,
	resourceType string,
	resourceID string,
	before any,
	after any,
	reason string,
) {
	_ = (audit.Recorder{}).Record(request.Context(), s.database.DB(), audit.Entry{
		ActorType: "user", ActorID: principalFromContext(request.Context()).User.ID,
		ActorName: principalFromContext(request.Context()).User.Username,
		Action:    action, ResourceType: resourceType, ResourceID: resourceID,
		RequestID: middleware.GetReqID(request.Context()), SourceIP: clientIP(request),
		UserAgent: request.UserAgent(), Result: "success", Reason: reason,
		Before: before, After: after,
	})
}

func queryInt(
	request *http.Request, name string, fallback int, minimum int, maximum int,
) int {
	value, err := strconv.Atoi(request.URL.Query().Get(name))
	if err != nil {
		return fallback
	}
	if value < minimum {
		return minimum
	}
	if value > maximum {
		return maximum
	}
	return value
}

func boolInt(value bool) int {
	if value {
		return 1
	}
	return 0
}

func stringOr(value *string, fallback string) string {
	if value == nil {
		return fallback
	}
	return strings.TrimSpace(*value)
}

func jsonValueOr(value json.RawMessage, fallback string) string {
	if len(value) == 0 {
		return fallback
	}
	if !json.Valid(value) {
		return fallback
	}
	return string(value)
}

func rawJSON(value any) any {
	if value == nil {
		return nil
	}
	switch typed := value.(type) {
	case []byte:
		return json.RawMessage(typed)
	case string:
		return json.RawMessage(typed)
	default:
		return fmt.Sprint(value)
	}
}
