package agents

import (
	"context"
	"errors"
	"path/filepath"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/hkjang/invenqor/server/internal/storage"
)

func TestBearerProvisionAuthenticationRotationAndBlocking(t *testing.T) {
	t.Parallel()
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "test.db"),
	})
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	service := NewService(runtime.DB())
	externalID := uuid.NewString()
	result, err := service.ProvisionBearer(
		context.Background(), externalID, "host-01", "",
	)
	if err != nil {
		t.Fatalf("ProvisionBearer() error = %v", err)
	}
	if result.Token == "" {
		t.Fatal("ProvisionBearer() returned an empty token")
	}
	var stored string
	if err := runtime.DB().QueryRow(
		"SELECT secret_hash FROM agent_credentials WHERE agent_id = $1",
		result.Agent.ID,
	).Scan(&stored); err != nil {
		t.Fatal(err)
	}
	if stored == result.Token || len(stored) != 64 {
		t.Fatal("credential was not stored as a SHA-256 digest")
	}
	authenticated, err := service.AuthenticateBearer(
		context.Background(), result.Token,
	)
	if err != nil {
		t.Fatalf("AuthenticateBearer() error = %v", err)
	}
	if authenticated.AgentID != externalID {
		t.Fatalf("agent_id = %q, want %q", authenticated.AgentID, externalID)
	}

	replacement, err := service.RotateBearer(
		context.Background(), result.Agent.ID, time.Hour, "",
	)
	if err != nil {
		t.Fatalf("RotateBearer() error = %v", err)
	}
	if _, err := service.AuthenticateBearer(
		context.Background(), result.Token,
	); err != nil {
		t.Fatalf("old token rejected during grace period: %v", err)
	}
	if _, err := service.AuthenticateBearer(
		context.Background(), replacement,
	); err != nil {
		t.Fatalf("replacement token rejected: %v", err)
	}

	if err := service.SetBlocked(
		context.Background(), result.Agent.ID, true, "",
	); err != nil {
		t.Fatalf("SetBlocked() error = %v", err)
	}
	if _, err := service.AuthenticateBearer(
		context.Background(), replacement,
	); !errors.Is(err, ErrBlocked) {
		t.Fatalf("blocked authentication error = %v, want ErrBlocked", err)
	}
}

func TestCertificateAuthentication(t *testing.T) {
	t.Parallel()
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "test.db"),
	})
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	service := NewService(runtime.DB())
	result, err := service.ProvisionBearer(
		context.Background(), uuid.NewString(), "host-02", "",
	)
	if err != nil {
		t.Fatal(err)
	}
	fingerprint := "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	expiresAt := time.Now().Add(time.Hour)
	if err := service.RegisterCertificate(
		context.Background(),
		result.Agent.ID,
		fingerprint,
		&expiresAt,
		"",
	); err != nil {
		t.Fatalf("RegisterCertificate() error = %v", err)
	}
	agent, err := service.AuthenticateCertificate(
		context.Background(),
		"AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:"+
			"AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA",
	)
	if err != nil {
		t.Fatalf("AuthenticateCertificate() error = %v", err)
	}
	if agent.ID != result.Agent.ID {
		t.Fatalf("agent ID = %q, want %q", agent.ID, result.Agent.ID)
	}
}
