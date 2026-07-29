package updates

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
)

type Manifest struct {
	Version      string `json:"version"`
	Channel      string `json:"channel"`
	OS           string `json:"os"`
	Architecture string `json:"architecture"`
	SHA256       string `json:"sha256"`
	Signature    string `json:"signature"`
	DownloadURL  string `json:"download_url"`
	Size         int64  `json:"size"`
	Rollout      int    `json:"rollout_percent"`
}

type Store struct {
	root string
	mu   sync.RWMutex
}

func Open(root string) (*Store, error) {
	if err := os.MkdirAll(root, 0o700); err != nil {
		return nil, fmt.Errorf("create update store: %w", err)
	}
	if err := os.Chmod(root, 0o700); err != nil {
		return nil, err
	}
	return &Store{root: root}, nil
}

func (s *Store) Publish(manifest Manifest, source io.Reader) (Manifest, error) {
	if !safeVersion(manifest.Version) ||
		(manifest.Channel != "stable" && manifest.Channel != "beta") ||
		manifest.OS != "linux" ||
		(manifest.Architecture != "x86_64" && manifest.Architecture != "aarch64") ||
		!validSignature(manifest.Signature) ||
		manifest.Rollout < 0 || manifest.Rollout > 100 {
		return Manifest{}, errors.New("update metadata is invalid")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	base := manifest.Version + "-" + manifest.OS + "-" + manifest.Architecture
	artifact := filepath.Join(s.root, base+".bin")
	hash, size, err := writeArtifact(artifact, source)
	if err != nil {
		return Manifest{}, err
	}
	manifest.SHA256 = hash
	manifest.Size = size
	manifest.DownloadURL = "/v1/agent/updates/" + base + "/artifact"
	bytes, _ := json.MarshalIndent(manifest, "", "  ")
	if err := atomicWrite(filepath.Join(s.root, base+".json"), append(bytes, '\n'), 0o600); err != nil {
		_ = os.Remove(artifact)
		return Manifest{}, err
	}
	return manifest, nil
}

func validSignature(value string) bool {
	decoded, err := base64.StdEncoding.DecodeString(strings.TrimSpace(value))
	return err == nil && len(decoded) == 64
}

func (s *Store) Latest(channel, osName, architecture, agentID string) (*Manifest, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	entries, err := os.ReadDir(s.root)
	if err != nil {
		return nil, err
	}
	var matches []Manifest
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".json") {
			continue
		}
		var manifest Manifest
		bytes, err := os.ReadFile(filepath.Join(s.root, entry.Name()))
		if err != nil || json.Unmarshal(bytes, &manifest) != nil {
			continue
		}
		if manifest.Channel == channel && manifest.OS == osName &&
			manifest.Architecture == architecture &&
			rolloutBucket(agentID) < manifest.Rollout {
			matches = append(matches, manifest)
		}
	}
	sort.Slice(matches, func(i, j int) bool {
		return compareVersion(matches[i].Version, matches[j].Version) > 0
	})
	if len(matches) == 0 {
		return nil, nil
	}
	return &matches[0], nil
}

func (s *Store) Artifact(base string) (string, error) {
	if strings.Contains(base, "/") || strings.Contains(base, "..") {
		return "", errors.New("invalid update artifact")
	}
	path := filepath.Join(s.root, base+".bin")
	if _, err := os.Stat(path); err != nil {
		return "", err
	}
	return path, nil
}

func rolloutBucket(agentID string) int {
	value := 0
	for _, char := range agentID {
		value = (value*31 + int(char)) % 100
	}
	return value
}

func compareVersion(left, right string) int {
	var l1, l2, l3, r1, r2, r3 int
	fmt.Sscanf(strings.TrimPrefix(left, "v"), "%d.%d.%d", &l1, &l2, &l3)
	fmt.Sscanf(strings.TrimPrefix(right, "v"), "%d.%d.%d", &r1, &r2, &r3)
	switch {
	case l1 != r1:
		return l1 - r1
	case l2 != r2:
		return l2 - r2
	default:
		return l3 - r3
	}
}

func safeVersion(value string) bool {
	var a, b, c int
	n, err := fmt.Sscanf(strings.TrimPrefix(value, "v"), "%d.%d.%d", &a, &b, &c)
	return err == nil && n == 3 && value == fmt.Sprintf("%d.%d.%d", a, b, c)
}

func writeArtifact(path string, source io.Reader) (string, int64, error) {
	file, err := os.CreateTemp(filepath.Dir(path), "."+filepath.Base(path)+".tmp-*")
	if err != nil {
		return "", 0, err
	}
	temporary := file.Name()
	if err := file.Chmod(0o600); err != nil {
		file.Close()
		os.Remove(temporary)
		return "", 0, err
	}
	defer os.Remove(temporary)
	hash := sha256.New()
	size, err := io.Copy(io.MultiWriter(file, hash), io.LimitReader(source, 128*1024*1024+1))
	if err != nil || size > 128*1024*1024 {
		file.Close()
		return "", 0, errors.New("update artifact exceeds 128 MiB or could not be read")
	}
	if err := file.Sync(); err != nil {
		file.Close()
		return "", 0, err
	}
	if err := file.Close(); err != nil {
		return "", 0, err
	}
	// Link creates the immutable final name without replacing a release that
	// another server pod published concurrently.
	if err := os.Link(temporary, path); err != nil {
		return "", 0, err
	}
	if err := syncDirectory(filepath.Dir(path)); err != nil {
		return "", 0, err
	}
	return hex.EncodeToString(hash.Sum(nil)), size, nil
}

func atomicWrite(path string, bytes []byte, mode os.FileMode) error {
	file, err := os.CreateTemp(filepath.Dir(path), "."+filepath.Base(path)+".tmp-*")
	if err != nil {
		return err
	}
	temporary := file.Name()
	defer os.Remove(temporary)
	if err := file.Chmod(mode); err != nil {
		file.Close()
		return err
	}
	if _, err := file.Write(bytes); err != nil {
		file.Close()
		return err
	}
	if err := file.Sync(); err != nil {
		file.Close()
		return err
	}
	if err := file.Close(); err != nil {
		return err
	}
	if err := os.Link(temporary, path); err != nil {
		return err
	}
	return syncDirectory(filepath.Dir(path))
}

func syncDirectory(path string) error {
	directory, err := os.Open(path)
	if err != nil {
		return err
	}
	defer directory.Close()
	return directory.Sync()
}
