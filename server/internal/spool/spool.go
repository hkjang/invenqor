package spool

import (
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/hkjang/invenqor/server/internal/durablefs"
)

type Manager struct {
	directory string
}

func Open(stateDir string) (*Manager, error) {
	directory := filepath.Join(stateDir, "spool")
	if err := os.MkdirAll(directory, 0o700); err != nil {
		return nil, fmt.Errorf("create event spool: %w", err)
	}
	if err := os.Chmod(directory, 0o700); err != nil {
		return nil, fmt.Errorf("secure event spool: %w", err)
	}
	return &Manager{directory: directory}, nil
}

// Append creates an immutable event segment. Existing names are treated as
// idempotent duplicate writes and are never overwritten.
func (m *Manager) Append(
	agentID string,
	eventID string,
	raw []byte,
) (bool, error) {
	if !safeID(agentID) || !safeID(eventID) {
		return false, errors.New("spool identity is invalid")
	}
	path := filepath.Join(m.directory, agentID+"_"+eventID+".json")
	file, err := os.OpenFile(
		path,
		os.O_WRONLY|os.O_CREATE|os.O_EXCL,
		0o600,
	)
	if errors.Is(err, fs.ErrExist) {
		return true, nil
	}
	if err != nil {
		return false, fmt.Errorf("create spool segment: %w", err)
	}
	failed := true
	defer func() {
		file.Close()
		if failed {
			_ = os.Remove(path)
		}
	}()
	if _, err := file.Write(raw); err != nil {
		return false, fmt.Errorf("write spool segment: %w", err)
	}
	if err := file.Sync(); err != nil {
		return false, fmt.Errorf("sync spool segment: %w", err)
	}
	if err := file.Close(); err != nil {
		return false, fmt.Errorf("close spool segment: %w", err)
	}
	if err := durablefs.SyncDirectory(m.directory); err != nil {
		return false, fmt.Errorf("sync spool directory: %w", err)
	}
	failed = false
	return false, nil
}

func (m *Manager) Pending() ([]string, error) {
	entries, err := os.ReadDir(m.directory)
	if err != nil {
		return nil, err
	}
	result := make([]string, 0, len(entries))
	for _, entry := range entries {
		if !entry.IsDir() && strings.HasSuffix(entry.Name(), ".json") {
			result = append(result, filepath.Join(m.directory, entry.Name()))
		}
	}
	sort.Strings(result)
	return result, nil
}

func (m *Manager) Read(path string) ([]byte, error) {
	if filepath.Dir(path) != m.directory {
		return nil, errors.New("spool path is outside the spool directory")
	}
	return os.ReadFile(path)
}

func (m *Manager) Acknowledge(path string) error {
	if filepath.Dir(path) != m.directory {
		return errors.New("spool path is outside the spool directory")
	}
	if err := os.Remove(path); err != nil && !errors.Is(err, fs.ErrNotExist) {
		return fmt.Errorf("acknowledge spool segment: %w", err)
	}
	return nil
}

func (m *Manager) Directory() string {
	return m.directory
}

func safeID(value string) bool {
	if value == "" {
		return false
	}
	for _, character := range value {
		if (character >= 'a' && character <= 'z') ||
			(character >= 'A' && character <= 'Z') ||
			(character >= '0' && character <= '9') ||
			character == '-' {
			continue
		}
		return false
	}
	return true
}
