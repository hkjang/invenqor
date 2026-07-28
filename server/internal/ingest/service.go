package ingest

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/hkjang/invenqor/server/internal/agents"
)

var ErrInProgress = errors.New("event is already being processed")

type Result struct {
	Duplicate     bool   `json:"duplicate"`
	PolicyVersion string `json:"policy_version,omitempty"`
}

type Service struct {
	database *sql.DB
	now      func() time.Time
}

func NewService(database *sql.DB) *Service {
	return &Service{
		database: database,
		now:      func() time.Time { return time.Now().UTC() },
	}
}

func (s *Service) Process(
	ctx context.Context,
	agent agents.Agent,
	envelope Envelope,
	raw []byte,
	agentVersion string,
) (Result, error) {
	if agent.AgentID != envelope.AgentID {
		return Result{}, errors.New("authenticated agent does not match event agent_id")
	}
	tx, err := s.database.BeginTx(ctx, nil)
	if err != nil {
		return Result{}, fmt.Errorf("begin event processing: %w", err)
	}
	defer tx.Rollback()

	eventInternalID := uuid.NewString()
	insert, err := tx.ExecContext(
		ctx,
		`INSERT INTO agent_events(
			id, agent_id, event_id, schema_version, kind, snapshot_hash,
			raw_event, processing_status
		) VALUES ($1, $2, $3, $4, $5, $6, $7, 'processing')
		ON CONFLICT(agent_id, event_id) DO NOTHING`,
		eventInternalID,
		agent.ID,
		envelope.EventID,
		envelope.SchemaVersion,
		envelope.Kind,
		envelope.SnapshotHash,
		string(raw),
	)
	if err != nil {
		return Result{}, fmt.Errorf("reserve event: %w", err)
	}
	inserted, _ := insert.RowsAffected()
	if inserted == 0 {
		status := ""
		if err := tx.QueryRowContext(
			ctx,
			`SELECT id, processing_status FROM agent_events
			 WHERE agent_id = $1 AND event_id = $2`,
			agent.ID,
			envelope.EventID,
		).Scan(&eventInternalID, &status); err != nil {
			return Result{}, fmt.Errorf("read existing event: %w", err)
		}
		if status == "processed" {
			if err := tx.Commit(); err != nil {
				return Result{}, err
			}
			return Result{
				Duplicate:     true,
				PolicyVersion: agent.PolicyVersion,
			}, nil
		}
		if status == "processing" {
			return Result{}, ErrInProgress
		}
		claim, err := tx.ExecContext(
			ctx,
			`UPDATE agent_events
			 SET processing_status = 'processing', processing_error = NULL
			 WHERE id = $1 AND processing_status IN ('pending', 'failed')`,
			eventInternalID,
		)
		if err != nil {
			return Result{}, err
		}
		if count, _ := claim.RowsAffected(); count == 0 {
			return Result{}, ErrInProgress
		}
	}

	assetIDs := make([]string, 0)
	if envelope.Snapshot != nil {
		for _, record := range envelope.Snapshot.Records {
			assetID, err := s.upsertRecord(
				ctx, tx, agent, eventInternalID, record, "",
			)
			if err != nil {
				return s.failEvent(
					ctx, tx, eventInternalID, agent, envelope, raw, err,
				)
			}
			assetIDs = append(assetIDs, assetID)
		}
	}
	for _, change := range envelope.Changes {
		if change.Kind == "removed" {
			if err := s.removeRecord(
				ctx,
				tx,
				agent,
				eventInternalID,
				change.Category,
				change.AssetID,
			); err != nil {
				return s.failEvent(
					ctx, tx, eventInternalID, agent, envelope, raw, err,
				)
			}
			continue
		}
		assetID, err := s.upsertRecord(
			ctx, tx, agent, eventInternalID, *change.Record, change.Kind,
		)
		if err != nil {
			return s.failEvent(
				ctx, tx, eventInternalID, agent, envelope, raw, err,
			)
		}
		assetIDs = append(assetIDs, assetID)
	}
	errorsToStore := append(
		append([]CollectionError{}, envelope.CollectionErrors...),
		snapshotErrors(envelope.Snapshot)...,
	)
	for _, collectionError := range errorsToStore {
		if _, err := tx.ExecContext(
			ctx,
			`INSERT INTO collector_errors(
				id, agent_event_id, collector, message
			) VALUES ($1, $2, $3, $4)`,
			uuid.NewString(),
			eventInternalID,
			collectionError.Collector,
			collectionError.Message,
		); err != nil {
			return s.failEvent(
				ctx, tx, eventInternalID, agent, envelope, raw, err,
			)
		}
	}
	if err := s.linkToHost(ctx, tx, agent.ID, assetIDs); err != nil {
		return s.failEvent(
			ctx, tx, eventInternalID, agent, envelope, raw, err,
		)
	}
	now := s.now()
	inventoryAt := any(nil)
	if envelope.Kind == "inventory" {
		inventoryAt = now
	}
	hostname, osName, architecture := agentMetadata(envelope)
	_, err = tx.ExecContext(
		ctx,
		`UPDATE agents SET
			hostname = CASE WHEN $1 = '' THEN hostname ELSE $1 END,
			os_name = CASE WHEN $2 = '' THEN os_name ELSE $2 END,
			architecture = CASE WHEN $3 = '' THEN architecture ELSE $3 END,
			version = CASE WHEN $4 = '' THEN version ELSE $4 END,
			status = 'active', last_seen_at = $5,
			last_inventory_at = COALESCE($6, last_inventory_at), updated_at = $5
		 WHERE id = $7`,
		hostname,
		osName,
		architecture,
		agentVersion,
		now,
		inventoryAt,
		agent.ID,
	)
	if err != nil {
		return s.failEvent(
			ctx, tx, eventInternalID, agent, envelope, raw, err,
		)
	}
	_, err = tx.ExecContext(
		ctx,
		`UPDATE agent_events
		 SET processing_status = 'processed', processed_at = $1,
		     processing_error = NULL
		 WHERE id = $2`,
		now,
		eventInternalID,
	)
	if err != nil {
		return s.failEvent(
			ctx, tx, eventInternalID, agent, envelope, raw, err,
		)
	}
	if err := tx.Commit(); err != nil {
		return Result{}, fmt.Errorf("commit event: %w", err)
	}
	return Result{PolicyVersion: agent.PolicyVersion}, nil
}

func (s *Service) failEvent(
	ctx context.Context,
	tx *sql.Tx,
	eventID string,
	agent agents.Agent,
	envelope Envelope,
	raw []byte,
	cause error,
) (Result, error) {
	_ = tx.Rollback()
	message := cause.Error()
	if len(message) > 1000 {
		message = message[:1000]
	}
	_, _ = s.database.ExecContext(
		ctx,
		`INSERT INTO agent_events(
			id, agent_id, event_id, schema_version, kind, snapshot_hash,
			raw_event, processing_status, processing_error
		) VALUES ($1, $2, $3, $4, $5, $6, $7, 'failed', $8)
		ON CONFLICT(agent_id, event_id) DO UPDATE
		SET processing_status = 'failed', processing_error = excluded.processing_error`,
		eventID,
		agent.ID,
		envelope.EventID,
		envelope.SchemaVersion,
		envelope.Kind,
		envelope.SnapshotHash,
		string(raw),
		message,
	)
	return Result{}, cause
}

func (s *Service) upsertRecord(
	ctx context.Context,
	tx *sql.Tx,
	agent agents.Agent,
	eventID string,
	record AssetRecord,
	explicitKind string,
) (string, error) {
	payload := compactJSON(record.Payload)
	var sourceID, assetID, previous string
	var deleted any
	err := tx.QueryRowContext(
		ctx,
		`SELECT s.id, s.asset_id, s.payload_json, s.deleted_at
		 FROM asset_sources s
		 WHERE s.agent_id = $1 AND s.category = $2 AND s.source_asset_id = $3`,
		agent.ID,
		record.Category,
		record.AssetID,
	).Scan(&sourceID, &assetID, &previous, &deleted)
	newRecord := errors.Is(err, sql.ErrNoRows)
	if err != nil && !newRecord {
		return "", fmt.Errorf("read asset source: %w", err)
	}
	now := s.now()
	collectedAt := time.Unix(int64(record.CollectedAt), 0).UTC()
	assetType := assetType(record.Category)
	name := assetName(record)
	if newRecord {
		sourceID = uuid.NewString()
		assetID = uuid.NewString()
		assetKey := agent.AgentID + ":" + record.Category + ":" + record.AssetID
		_, err = tx.ExecContext(
			ctx,
			`INSERT INTO assets(
				id, asset_key, name, type, status, attributes_json, source,
				first_seen_at, last_seen_at, created_at, updated_at
			) VALUES (
				$1, $2, $3, $4, 'active', $5, 'agent',
				$6, $6, $6, $6
			)`,
			assetID,
			assetKey,
			name,
			assetType,
			payload,
			collectedAt,
		)
		if err != nil {
			return "", fmt.Errorf("insert representative asset: %w", err)
		}
		_, err = tx.ExecContext(
			ctx,
			`INSERT INTO asset_sources(
				id, asset_id, agent_id, category, source_asset_id,
				source_name, payload_json, collected_at, first_seen_at,
				last_seen_at
			) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, $8)`,
			sourceID,
			assetID,
			agent.ID,
			record.Category,
			record.AssetID,
			record.Source,
			payload,
			collectedAt,
		)
		if err != nil {
			return "", fmt.Errorf("insert asset source: %w", err)
		}
	} else {
		_, err = tx.ExecContext(
			ctx,
			`UPDATE asset_sources SET source_name = $1, payload_json = $2,
			 collected_at = $3, last_seen_at = $3, deleted_at = NULL
			 WHERE id = $4`,
			record.Source,
			payload,
			collectedAt,
			sourceID,
		)
		if err != nil {
			return "", fmt.Errorf("update asset source: %w", err)
		}
		_, err = tx.ExecContext(
			ctx,
			`UPDATE assets SET name = $1, attributes_json = $2,
			 status = 'active', last_seen_at = $3, updated_at = $4,
			 deleted_at = NULL WHERE id = $5`,
			name,
			payload,
			collectedAt,
			now,
			assetID,
		)
		if err != nil {
			return "", fmt.Errorf("update representative asset: %w", err)
		}
	}
	hash := sha256.Sum256([]byte(payload))
	_, err = tx.ExecContext(
		ctx,
		`INSERT INTO asset_snapshots(
			id, asset_id, snapshot_json, snapshot_hash, captured_at
		) VALUES ($1, $2, $3, $4, $5)`,
		uuid.NewString(),
		assetID,
		payload,
		hex.EncodeToString(hash[:]),
		collectedAt,
	)
	if err != nil {
		return "", fmt.Errorf("store asset snapshot: %w", err)
	}
	changeType := explicitKind
	if changeType == "" {
		if newRecord || deleted != nil {
			changeType = "added"
		} else if !sameJSON(previous, payload) {
			changeType = "updated"
		}
	}
	if changeType != "" {
		var before any
		if !newRecord {
			before = previous
		}
		_, err = tx.ExecContext(
			ctx,
			`INSERT INTO asset_changes(
				id, asset_id, source_event_id, change_type, before_json,
				after_json, actor_type, actor_id
			) VALUES ($1, $2, $3, $4, $5, $6, 'agent', $7)`,
			uuid.NewString(),
			assetID,
			eventID,
			changeType,
			before,
			payload,
			agent.ID,
		)
		if err != nil {
			return "", fmt.Errorf("store asset change: %w", err)
		}
	}
	return assetID, nil
}

func (s *Service) removeRecord(
	ctx context.Context,
	tx *sql.Tx,
	agent agents.Agent,
	eventID string,
	category string,
	sourceAssetID string,
) error {
	var sourceID, assetID, previous string
	err := tx.QueryRowContext(
		ctx,
		`SELECT id, asset_id, payload_json FROM asset_sources
		 WHERE agent_id = $1 AND category = $2 AND source_asset_id = $3
		   AND deleted_at IS NULL`,
		agent.ID,
		category,
		sourceAssetID,
	).Scan(&sourceID, &assetID, &previous)
	if errors.Is(err, sql.ErrNoRows) {
		return nil
	}
	if err != nil {
		return err
	}
	now := s.now()
	if _, err := tx.ExecContext(
		ctx,
		`UPDATE asset_sources SET deleted_at = $1 WHERE id = $2`,
		now,
		sourceID,
	); err != nil {
		return err
	}
	var active int
	if err := tx.QueryRowContext(
		ctx,
		`SELECT COUNT(*) FROM asset_sources
		 WHERE asset_id = $1 AND deleted_at IS NULL`,
		assetID,
	).Scan(&active); err != nil {
		return err
	}
	if active > 0 {
		return nil
	}
	if _, err := tx.ExecContext(
		ctx,
		`UPDATE assets SET status = 'removed', deleted_at = $1,
		 updated_at = $1 WHERE id = $2`,
		now,
		assetID,
	); err != nil {
		return err
	}
	if _, err := tx.ExecContext(
		ctx,
		`UPDATE asset_relations SET valid_to = $1
		 WHERE (source_asset_id = $2 OR target_asset_id = $2)
		   AND valid_to IS NULL`,
		now,
		assetID,
	); err != nil {
		return err
	}
	_, err = tx.ExecContext(
		ctx,
		`INSERT INTO asset_changes(
			id, asset_id, source_event_id, change_type, before_json,
			actor_type, actor_id, reason
		) VALUES ($1, $2, $3, 'removed', $4, 'agent', $5, 'explicit_agent_change')`,
		uuid.NewString(),
		assetID,
		eventID,
		previous,
		agent.ID,
	)
	return err
}

func (s *Service) linkToHost(
	ctx context.Context,
	tx *sql.Tx,
	agentID string,
	assetIDs []string,
) error {
	var hostID string
	err := tx.QueryRowContext(
		ctx,
		`SELECT asset_id FROM asset_sources
		 WHERE agent_id = $1 AND category = 'system' AND deleted_at IS NULL
		 ORDER BY first_seen_at LIMIT 1`,
		agentID,
	).Scan(&hostID)
	if errors.Is(err, sql.ErrNoRows) {
		return nil
	}
	if err != nil {
		return err
	}
	for _, assetID := range assetIDs {
		if assetID == hostID {
			continue
		}
		_, err := tx.ExecContext(
			ctx,
			`INSERT INTO asset_relations(
				id, source_asset_id, relation_type, target_asset_id,
				source, confidence
			) VALUES ($1, $2, 'belongs_to_host', $3, 'agent', 1.0)
			ON CONFLICT(source_asset_id, relation_type, target_asset_id)
			DO UPDATE SET valid_to = NULL, source = 'agent', confidence = 1.0`,
			uuid.NewString(),
			assetID,
			hostID,
		)
		if err != nil {
			return err
		}
	}
	return nil
}

func agentMetadata(envelope Envelope) (string, string, string) {
	if envelope.Snapshot == nil {
		return "", "", ""
	}
	for _, record := range envelope.Snapshot.Records {
		if record.Category != "system" {
			continue
		}
		var payload map[string]any
		if json.Unmarshal(record.Payload, &payload) != nil {
			continue
		}
		hostname, _ := payload["hostname"].(string)
		architecture, _ := payload["architecture"].(string)
		osName := ""
		if release, ok := payload["os_release"].(map[string]any); ok {
			osName, _ = release["pretty_name"].(string)
			if osName == "" {
				osName, _ = release["name"].(string)
			}
		}
		return hostname, osName, architecture
	}
	return "", "", ""
}

func assetType(category string) string {
	switch category {
	case "system":
		return "host"
	case "software.package":
		return "software_package"
	case "process":
		return "process"
	case "service":
		return "service"
	case "account":
		return "account"
	case "container":
		return "container"
	case "network.interface":
		return "network_interface"
	default:
		return strings.ReplaceAll(category, ".", "_")
	}
}

func assetName(record AssetRecord) string {
	var payload map[string]any
	_ = json.Unmarshal(record.Payload, &payload)
	for _, key := range []string{
		"hostname", "name", "service", "username", "device", "mount_point",
		"interface", "id",
	} {
		if value, ok := payload[key].(string); ok && strings.TrimSpace(value) != "" {
			return value
		}
	}
	return record.AssetID
}

func compactJSON(raw json.RawMessage) string {
	var buffer bytes.Buffer
	if err := json.Compact(&buffer, raw); err != nil {
		return string(raw)
	}
	return buffer.String()
}

func sameJSON(left string, right string) bool {
	var leftValue, rightValue any
	if json.Unmarshal([]byte(left), &leftValue) != nil ||
		json.Unmarshal([]byte(right), &rightValue) != nil {
		return left == right
	}
	return reflect.DeepEqual(leftValue, rightValue)
}
