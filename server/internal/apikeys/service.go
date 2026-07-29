package apikeys

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
	"sort"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/hkjang/invenqor/server/internal/apitime"
)

var (
	ErrInvalid      = errors.New("API key input is invalid")
	ErrUnauthorized = errors.New("API key is invalid or inactive")
	ErrNotFound     = errors.New("API key does not exist")
)

type Scope struct {
	Name        string `json:"name"`
	Description string `json:"description"`
}

var scopeCatalog = []Scope{
	{Name: "agents.read", Description: "Read agent status"},
	{Name: "assets.delete", Description: "Logically delete and restore assets"},
	{Name: "assets.read", Description: "Read and search assets"},
	{Name: "assets.write", Description: "Create and update assets"},
	{Name: "mcp.access", Description: "Connect to the Invenqor MCP endpoint"},
	{Name: "queries.execute", Description: "Execute the validated asset Query DSL"},
	{Name: "relations.read", Description: "Read asset relationships"},
	{Name: "relations.write", Description: "Create and remove asset relationships"},
}

type Key struct {
	ID         string       `json:"id"`
	UserID     string       `json:"user_id,omitempty"`
	Name       string       `json:"name"`
	Prefix     string       `json:"prefix"`
	Scopes     []string     `json:"scopes"`
	ExpiresAt  apitime.Time `json:"expires_at,omitempty"`
	LastUsedAt apitime.Time `json:"last_used_at,omitempty"`
	CreatedAt  apitime.Time `json:"created_at"`
	UpdatedAt  apitime.Time `json:"updated_at"`
	RevokedAt  apitime.Time `json:"revoked_at,omitempty"`
}

type Credential struct {
	KeyID  string
	UserID string
	Name   string
	Scopes []string
}

type Created struct {
	Key    Key    `json:"api_key"`
	Secret string `json:"secret"`
}

type Service struct {
	db *sql.DB
}

func NewService(db *sql.DB) *Service {
	return &Service{db: db}
}

func Scopes() []Scope {
	result := make([]Scope, len(scopeCatalog))
	copy(result, scopeCatalog)
	return result
}

func ValidScopes(scopes []string) ([]string, error) {
	allowed := make(map[string]struct{}, len(scopeCatalog))
	for _, scope := range scopeCatalog {
		allowed[scope.Name] = struct{}{}
	}
	unique := make(map[string]struct{}, len(scopes))
	for _, scope := range scopes {
		scope = strings.TrimSpace(scope)
		if _, ok := allowed[scope]; !ok {
			return nil, fmt.Errorf("%w: unknown scope %q", ErrInvalid, scope)
		}
		unique[scope] = struct{}{}
	}
	result := make([]string, 0, len(unique))
	for scope := range unique {
		result = append(result, scope)
	}
	sort.Strings(result)
	return result, nil
}

func (s *Service) Create(
	ctx context.Context,
	userID string,
	name string,
	scopes []string,
	expiresAt *time.Time,
) (Created, error) {
	name = strings.TrimSpace(name)
	if name == "" || len(name) > 120 || userID == "" {
		return Created{}, ErrInvalid
	}
	scopes, err := ValidScopes(scopes)
	if err != nil {
		return Created{}, err
	}
	if len(scopes) == 0 {
		return Created{}, fmt.Errorf("%w: at least one scope is required", ErrInvalid)
	}
	if expiresAt != nil && !expiresAt.After(time.Now().UTC()) {
		return Created{}, fmt.Errorf("%w: expiry must be in the future", ErrInvalid)
	}
	secret, prefix, hash, err := newSecret()
	if err != nil {
		return Created{}, err
	}
	id := uuid.NewString()
	scopeJSON, _ := json.Marshal(scopes)
	now := time.Now().UTC()
	_, err = s.db.ExecContext(ctx,
		`INSERT INTO api_keys(
			id, user_id, name, key_hash, key_prefix, scopes_json,
			expires_at, created_at, updated_at
		) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$8)`,
		id, userID, name, hash, prefix, string(scopeJSON), expiresAt, now,
	)
	if err != nil {
		return Created{}, fmt.Errorf("create API key: %w", err)
	}
	key, err := s.Get(ctx, id)
	return Created{Key: key, Secret: secret}, err
}

func (s *Service) List(ctx context.Context) ([]Key, error) {
	rows, err := s.db.QueryContext(ctx,
		`SELECT id, user_id, name, key_prefix, scopes_json, expires_at,
		 last_used_at, created_at, updated_at, revoked_at
		 FROM api_keys ORDER BY created_at DESC, id`,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := make([]Key, 0)
	for rows.Next() {
		key, err := scanKey(rows)
		if err != nil {
			return nil, err
		}
		result = append(result, key)
	}
	return result, rows.Err()
}

func (s *Service) Get(ctx context.Context, id string) (Key, error) {
	key, err := scanKey(s.db.QueryRowContext(ctx,
		`SELECT id, user_id, name, key_prefix, scopes_json, expires_at,
		 last_used_at, created_at, updated_at, revoked_at
		 FROM api_keys WHERE id=$1`, id,
	))
	if errors.Is(err, sql.ErrNoRows) {
		return Key{}, ErrNotFound
	}
	return key, err
}

func (s *Service) ReplaceScopes(
	ctx context.Context,
	id string,
	scopes []string,
) (Key, error) {
	scopes, err := ValidScopes(scopes)
	if err != nil {
		return Key{}, err
	}
	bytes, _ := json.Marshal(scopes)
	result, err := s.db.ExecContext(ctx,
		`UPDATE api_keys SET scopes_json=$1, updated_at=CURRENT_TIMESTAMP
		 WHERE id=$2 AND revoked_at IS NULL`, string(bytes), id,
	)
	if err != nil {
		return Key{}, err
	}
	if rows, _ := result.RowsAffected(); rows == 0 {
		return Key{}, ErrNotFound
	}
	return s.Get(ctx, id)
}

func (s *Service) AddScopes(
	ctx context.Context,
	id string,
	additions []string,
) (Key, error) {
	key, err := s.Get(ctx, id)
	if err != nil {
		return Key{}, err
	}
	return s.ReplaceScopes(ctx, id, append(key.Scopes, additions...))
}

func (s *Service) RemoveScope(
	ctx context.Context,
	id string,
	remove string,
) (Key, error) {
	key, err := s.Get(ctx, id)
	if err != nil {
		return Key{}, err
	}
	next := make([]string, 0, len(key.Scopes))
	for _, scope := range key.Scopes {
		if scope != remove {
			next = append(next, scope)
		}
	}
	if len(next) == len(key.Scopes) {
		return Key{}, fmt.Errorf("%w: scope is not assigned", ErrInvalid)
	}
	return s.ReplaceScopes(ctx, id, next)
}

func (s *Service) UpdateName(
	ctx context.Context,
	id string,
	name string,
) (Key, error) {
	name = strings.TrimSpace(name)
	if name == "" || len(name) > 120 {
		return Key{}, ErrInvalid
	}
	result, err := s.db.ExecContext(ctx,
		`UPDATE api_keys SET name=$1, updated_at=CURRENT_TIMESTAMP
		 WHERE id=$2 AND revoked_at IS NULL`, name, id,
	)
	if err != nil {
		return Key{}, err
	}
	if rows, _ := result.RowsAffected(); rows == 0 {
		return Key{}, ErrNotFound
	}
	return s.Get(ctx, id)
}

func (s *Service) Rotate(
	ctx context.Context,
	id string,
	grace time.Duration,
) (Created, error) {
	if grace < 0 || grace > 7*24*time.Hour {
		return Created{}, ErrInvalid
	}
	secret, prefix, hash, err := newSecret()
	if err != nil {
		return Created{}, err
	}
	now := time.Now().UTC()
	result, err := s.db.ExecContext(ctx,
		`UPDATE api_keys SET
		 previous_key_hash=key_hash, previous_valid_until=$1,
		 key_hash=$2, key_prefix=$3, updated_at=$4
		 WHERE id=$5 AND revoked_at IS NULL
		 AND (expires_at IS NULL OR expires_at > CURRENT_TIMESTAMP)`,
		now.Add(grace), hash, prefix, now, id,
	)
	if err != nil {
		return Created{}, err
	}
	if rows, _ := result.RowsAffected(); rows == 0 {
		return Created{}, ErrNotFound
	}
	key, err := s.Get(ctx, id)
	return Created{Key: key, Secret: secret}, err
}

func (s *Service) Revoke(ctx context.Context, id string) error {
	result, err := s.db.ExecContext(ctx,
		`UPDATE api_keys SET revoked_at=CURRENT_TIMESTAMP,
		 previous_key_hash=NULL, previous_valid_until=NULL,
		 updated_at=CURRENT_TIMESTAMP
		 WHERE id=$1 AND revoked_at IS NULL`, id,
	)
	if err != nil {
		return err
	}
	if rows, _ := result.RowsAffected(); rows == 0 {
		return ErrNotFound
	}
	return nil
}

func (s *Service) Authenticate(ctx context.Context, secret string) (Credential, error) {
	if !strings.HasPrefix(secret, "ivq_sk_") || len(secret) < 40 {
		return Credential{}, ErrUnauthorized
	}
	hash := sha256.Sum256([]byte(secret))
	var credential Credential
	var userID sql.NullString
	var scopesJSON string
	err := s.db.QueryRowContext(ctx,
		`SELECT k.id, k.user_id, k.name, k.scopes_json
		 FROM api_keys k JOIN users u ON u.id=k.user_id
		 WHERE k.revoked_at IS NULL AND u.active=TRUE AND u.deleted_at IS NULL
		 AND (k.expires_at IS NULL OR k.expires_at > CURRENT_TIMESTAMP)
		 AND (k.key_hash=$1 OR (
		   k.previous_key_hash=$1 AND k.previous_valid_until > CURRENT_TIMESTAMP
		 ))`, hex.EncodeToString(hash[:]),
	).Scan(&credential.KeyID, &userID, &credential.Name, &scopesJSON)
	if errors.Is(err, sql.ErrNoRows) {
		return Credential{}, ErrUnauthorized
	}
	if err != nil {
		return Credential{}, err
	}
	if err := json.Unmarshal([]byte(scopesJSON), &credential.Scopes); err != nil {
		return Credential{}, ErrUnauthorized
	}
	credential.UserID = userID.String
	_, _ = s.db.ExecContext(ctx,
		`UPDATE api_keys SET last_used_at=CURRENT_TIMESTAMP WHERE id=$1`,
		credential.KeyID,
	)
	return credential, nil
}

func newSecret() (secret string, prefix string, hash string, err error) {
	random := make([]byte, 32)
	if _, err = rand.Read(random); err != nil {
		return "", "", "", err
	}
	encoded := base64.RawURLEncoding.EncodeToString(random)
	prefix = encoded[:8]
	secret = "ivq_sk_" + prefix + "_" + encoded
	sum := sha256.Sum256([]byte(secret))
	hash = hex.EncodeToString(sum[:])
	return
}

type rowScanner interface {
	Scan(...any) error
}

func scanKey(scanner rowScanner) (Key, error) {
	var key Key
	var userID sql.NullString
	var scopesJSON string
	err := scanner.Scan(
		&key.ID, &userID, &key.Name, &key.Prefix, &scopesJSON,
		&key.ExpiresAt, &key.LastUsedAt, &key.CreatedAt, &key.UpdatedAt,
		&key.RevokedAt,
	)
	if err != nil {
		return Key{}, err
	}
	key.UserID = userID.String
	if err := json.Unmarshal([]byte(scopesJSON), &key.Scopes); err != nil {
		return Key{}, err
	}
	return key, nil
}
