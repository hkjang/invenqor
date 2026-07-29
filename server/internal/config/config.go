package config

import (
	"errors"
	"fmt"
	"net"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

const (
	defaultListenAddress = "127.0.0.1:7070"
	defaultStateDir      = "/var/lib/invenqor-server"
)

// Config contains only process bootstrap settings. Runtime settings live in the
// metadata database and are edited through the administrative API.
type Config struct {
	ListenAddress   string
	BaseURL         string
	StateDir        string
	SQLitePath      string
	PostgresDSN     string
	DatabaseSchema  string
	DatabaseTimeout time.Duration
	ShutdownTimeout time.Duration
	MasterKeyPath   string
	UpdateDir       string
}

func Load() (Config, error) {
	stateDir := envOrDefault("INVENQOR_STATE_DIR", defaultStateDir)
	config := Config{
		ListenAddress:   envOrDefault("INVENQOR_LISTEN_ADDRESS", defaultListenAddress),
		BaseURL:         strings.TrimRight(os.Getenv("INVENQOR_BASE_URL"), "/"),
		StateDir:        stateDir,
		SQLitePath:      envOrDefault("INVENQOR_SQLITE_PATH", filepath.Join(stateDir, "invenqor.db")),
		PostgresDSN:     strings.TrimSpace(os.Getenv("INVENQOR_POSTGRES_DSN")),
		DatabaseSchema:  envOrDefault("INVENQOR_POSTGRES_SCHEMA", "public"),
		DatabaseTimeout: durationEnv("INVENQOR_DATABASE_TIMEOUT", 5*time.Second),
		ShutdownTimeout: durationEnv("INVENQOR_SHUTDOWN_TIMEOUT", 15*time.Second),
		MasterKeyPath:   strings.TrimSpace(os.Getenv("INVENQOR_MASTER_KEY_FILE")),
		UpdateDir:       envOrDefault("INVENQOR_UPDATE_DIR", filepath.Join(stateDir, "updates")),
	}
	if err := config.Validate(); err != nil {
		return Config{}, err
	}
	return config, nil
}

func (c Config) Validate() error {
	if strings.TrimSpace(c.ListenAddress) == "" {
		return errors.New("listen address is required")
	}
	if _, _, err := net.SplitHostPort(c.ListenAddress); err != nil {
		return fmt.Errorf("invalid listen address: %w", err)
	}
	if !filepath.IsAbs(c.StateDir) {
		return errors.New("state directory must be absolute")
	}
	if !filepath.IsAbs(c.SQLitePath) {
		return errors.New("SQLite path must be absolute")
	}
	if c.DatabaseTimeout <= 0 {
		return errors.New("database timeout must be positive")
	}
	if c.ShutdownTimeout <= 0 {
		return errors.New("shutdown timeout must be positive")
	}
	if c.MasterKeyPath != "" && !filepath.IsAbs(c.MasterKeyPath) {
		return errors.New("master key file path must be absolute")
	}
	if c.UpdateDir != "" && !filepath.IsAbs(c.UpdateDir) {
		return errors.New("update directory must be absolute")
	}
	if c.BaseURL != "" {
		parsed, err := url.Parse(c.BaseURL)
		if err != nil || parsed.Host == "" {
			return errors.New("base URL is invalid")
		}
		if parsed.Scheme != "https" && parsed.Scheme != "http" {
			return errors.New("base URL must use HTTP or HTTPS")
		}
	}
	if !validSchemaName(c.DatabaseSchema) {
		return errors.New("PostgreSQL schema contains invalid characters")
	}
	return nil
}

func envOrDefault(name, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(name)); value != "" {
		return value
	}
	return fallback
}

func durationEnv(name string, fallback time.Duration) time.Duration {
	value := strings.TrimSpace(os.Getenv(name))
	if value == "" {
		return fallback
	}
	if parsed, err := time.ParseDuration(value); err == nil {
		return parsed
	}
	if seconds, err := strconv.ParseUint(value, 10, 64); err == nil {
		return time.Duration(seconds) * time.Second
	}
	return fallback
}

func validSchemaName(value string) bool {
	if value == "" {
		return false
	}
	for index, char := range value {
		if (char >= 'a' && char <= 'z') ||
			(char >= 'A' && char <= 'Z') ||
			char == '_' ||
			(index > 0 && char >= '0' && char <= '9') {
			continue
		}
		return false
	}
	return true
}
