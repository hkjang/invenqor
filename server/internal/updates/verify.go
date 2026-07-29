package updates

import (
	"crypto/ed25519"
	"encoding/base64"
	"errors"
	"fmt"
	"strings"
)

// Publishing used to accept any 64-byte value as a signature. Nothing on the
// server could check it, because the server held no public key - so a mistyped
// or stale signature published successfully and then failed verification on
// every agent in the fleet, forever, with the only symptom a line in each
// agent's log. Publishing is the one moment where the mistake is cheap to
// catch, so the key belongs here.
//
// The key is optional: a deployment that signs elsewhere and trusts its release
// pipeline can leave it unset, and the API says so rather than pretending the
// signature was checked.

var (
	ErrSignatureUnverifiable = errors.New(
		"the update signing public key is not configured on this server",
	)
	ErrSignatureRejected = errors.New(
		"the signature does not verify against the configured signing key",
	)
)

// ParsePublicKey accepts the same base64 Ed25519 public key the agents pin.
func ParsePublicKey(value string) (ed25519.PublicKey, error) {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" {
		return nil, nil
	}
	decoded, err := base64.StdEncoding.DecodeString(trimmed)
	if err != nil {
		decoded, err = base64.RawStdEncoding.DecodeString(trimmed)
	}
	if err != nil {
		return nil, fmt.Errorf("decode update signing key: %w", err)
	}
	if len(decoded) != ed25519.PublicKeySize {
		return nil, fmt.Errorf(
			"update signing key must be %d bytes, got %d",
			ed25519.PublicKeySize, len(decoded),
		)
	}
	return ed25519.PublicKey(decoded), nil
}

// DecodeSignature accepts the base64 detached signature an operator pastes or
// uploads, tolerating the line breaks `base64` inserts by default.
func DecodeSignature(value string) ([]byte, error) {
	cleaned := strings.Map(func(char rune) rune {
		if char == '\n' || char == '\r' || char == ' ' || char == '\t' {
			return -1
		}
		return char
	}, value)
	if cleaned == "" {
		return nil, errors.New("the signature is empty")
	}
	decoded, err := base64.StdEncoding.DecodeString(cleaned)
	if err != nil {
		decoded, err = base64.RawStdEncoding.DecodeString(cleaned)
	}
	if err != nil {
		return nil, fmt.Errorf("decode signature: %w", err)
	}
	if len(decoded) != ed25519.SignatureSize {
		return nil, fmt.Errorf(
			"an Ed25519 signature is %d bytes, got %d",
			ed25519.SignatureSize, len(decoded),
		)
	}
	return decoded, nil
}

// SigningKeyConfigured reports whether publish-time verification is possible.
func (s *Store) SigningKeyConfigured() bool {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return len(s.publicKey) == ed25519.PublicKeySize
}

// SetSigningKey installs the key used to verify a publication.
func (s *Store) SetSigningKey(key ed25519.PublicKey) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.publicKey = key
}

// verifySignature checks a detached signature over the exact bytes an agent will
// download.
func (s *Store) verifySignature(signature []byte, artifact []byte) error {
	s.mu.RLock()
	key := s.publicKey
	s.mu.RUnlock()
	if len(key) != ed25519.PublicKeySize {
		return ErrSignatureUnverifiable
	}
	if !ed25519.Verify(key, artifact, signature) {
		return ErrSignatureRejected
	}
	return nil
}
