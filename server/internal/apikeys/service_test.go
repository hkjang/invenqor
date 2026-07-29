package apikeys_test

import (
	"context"
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/hkjang/invenqor/server/internal/apikeys"
	"github.com/hkjang/invenqor/server/internal/storage"
)

func TestAPIKeyScopeLifecycleRotationAndRevocation(t *testing.T) {
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "invenqor.db"),
	})
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	userID := uuid.NewString()
	_, err = runtime.DB().Exec(
		`INSERT INTO users(
		 id, username, normalized_username, display_name, active, super_admin
		 ) VALUES ($1,'key.owner','key.owner','Key Owner',TRUE,TRUE)`, userID,
	)
	if err != nil {
		t.Fatal(err)
	}
	service := apikeys.NewService(runtime.DB())
	created, err := service.Create(
		context.Background(), userID, "automation",
		[]string{"mcp.access", "assets.read"}, nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if created.Secret == "" || created.Key.Prefix == "" ||
		len(created.Key.Scopes) != 2 {
		t.Fatalf("created = %#v", created)
	}
	credential, err := service.Authenticate(context.Background(), created.Secret)
	if err != nil || credential.KeyID != created.Key.ID {
		t.Fatalf("Authenticate() = %#v, %v", credential, err)
	}

	updated, err := service.AddScopes(
		context.Background(), created.Key.ID, []string{"agents.read"},
	)
	if err != nil || len(updated.Scopes) != 3 {
		t.Fatalf("AddScopes() = %#v, %v", updated, err)
	}
	updated, err = service.RemoveScope(
		context.Background(), created.Key.ID, "assets.read",
	)
	if err != nil || len(updated.Scopes) != 2 {
		t.Fatalf("RemoveScope() = %#v, %v", updated, err)
	}

	rotated, err := service.Rotate(
		context.Background(), created.Key.ID, time.Hour,
	)
	if err != nil || rotated.Secret == created.Secret {
		t.Fatalf("Rotate() = %#v, %v", rotated, err)
	}
	if _, err := service.Authenticate(context.Background(), created.Secret); err != nil {
		t.Fatalf("old key rejected during grace: %v", err)
	}
	if _, err := service.Authenticate(context.Background(), rotated.Secret); err != nil {
		t.Fatalf("new key rejected: %v", err)
	}

	if err := service.Revoke(context.Background(), created.Key.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := service.Authenticate(context.Background(), rotated.Secret); !errors.Is(err, apikeys.ErrUnauthorized) {
		t.Fatalf("revoked key error = %v", err)
	}
}

func TestAPIKeyRejectsUnknownScopesAndExpiredKeys(t *testing.T) {
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "invenqor.db"),
	})
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	userID := uuid.NewString()
	_, _ = runtime.DB().Exec(
		`INSERT INTO users(
		 id, username, normalized_username, display_name, active, super_admin
		 ) VALUES ($1,'key.owner','key.owner','Key Owner',TRUE,TRUE)`, userID,
	)
	service := apikeys.NewService(runtime.DB())
	if _, err := service.Create(context.Background(), userID, "bad",
		[]string{"root.everything"}, nil); !errors.Is(err, apikeys.ErrInvalid) {
		t.Fatalf("unknown scope error = %v", err)
	}
	past := time.Now().Add(-time.Minute)
	if _, err := service.Create(context.Background(), userID, "expired",
		[]string{"assets.read"}, &past); !errors.Is(err, apikeys.ErrInvalid) {
		t.Fatalf("past expiry error = %v", err)
	}
}
