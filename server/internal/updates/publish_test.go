package updates

import (
	"crypto/ed25519"
	"encoding/base64"
	"errors"
	"strings"
	"testing"

	"github.com/google/uuid"
)

func newStore(t *testing.T) *Store {
	t.Helper()
	store, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	return store
}

func signingPair(t *testing.T) (ed25519.PrivateKey, string) {
	t.Helper()
	public, private, err := ed25519.GenerateKey(nil)
	if err != nil {
		t.Fatal(err)
	}
	return private, base64.StdEncoding.EncodeToString(public)
}

func manifest(signature string) Manifest {
	return Manifest{
		Version:      "1.2.3",
		Channel:      "stable",
		OS:           "linux",
		Architecture: "x86_64",
		Signature:    signature,
		Rollout:      100,
	}
}

// The defect this covers: publishing accepted any 64-byte value as a signature,
// so a mistyped or stale signature published fine and then failed verification
// on every agent in the fleet, forever, with the only symptom a log line on each
// host.
func TestPublishRejectsASignatureThatDoesNotVerify(t *testing.T) {
	store := newStore(t)
	private, publicKey := signingPair(t)
	parsed, err := ParsePublicKey(publicKey)
	if err != nil {
		t.Fatal(err)
	}
	store.SetSigningKey(parsed)
	artifact := []byte("agent-binary-bytes")

	// A signature over different bytes: exactly the "signed the wrong file" case.
	wrong := base64.StdEncoding.EncodeToString(
		ed25519.Sign(private, []byte("some-other-file")),
	)
	_, err = store.Publish(manifest(wrong), strings.NewReader(string(artifact)))
	if !errors.Is(err, ErrSignatureRejected) {
		t.Fatalf("Publish() error = %v, want ErrSignatureRejected", err)
	}
	// Nothing may be left behind by a rejected publication.
	releases, err := store.Releases()
	if err != nil {
		t.Fatal(err)
	}
	if len(releases) != 0 {
		t.Fatalf("a rejected publication left %d releases behind", len(releases))
	}
	if found, _ := store.Artifact("1.2.3-linux-x86_64"); found != "" {
		t.Fatal("a rejected publication left its artifact behind")
	}

	good := base64.StdEncoding.EncodeToString(ed25519.Sign(private, artifact))
	published, err := store.Publish(manifest(good), strings.NewReader(string(artifact)))
	if err != nil {
		t.Fatalf("Publish() error = %v", err)
	}
	if !published.SignatureVerified {
		t.Fatal("a verified publication did not record that it was verified")
	}
	if published.Size != int64(len(artifact)) || published.SHA256 == "" {
		t.Fatalf("published manifest = %+v", published)
	}
}

// Without a configured key the server must say so rather than implying a check.
func TestPublishReportsWhenItCannotVerify(t *testing.T) {
	store := newStore(t)
	private, _ := signingPair(t)
	artifact := "agent-binary"
	signature := base64.StdEncoding.EncodeToString(
		ed25519.Sign(private, []byte(artifact)),
	)
	published, err := store.Publish(manifest(signature), strings.NewReader(artifact))
	if err != nil {
		t.Fatalf("Publish() error = %v", err)
	}
	if published.SignatureVerified {
		t.Fatal("the server claimed to verify a signature without a key")
	}
	if store.SigningKeyConfigured() {
		t.Fatal("SigningKeyConfigured() lied")
	}
}

func TestPublishAcceptsASignatureWithLineBreaks(t *testing.T) {
	store := newStore(t)
	private, publicKey := signingPair(t)
	parsed, _ := ParsePublicKey(publicKey)
	store.SetSigningKey(parsed)
	artifact := "agent-binary"
	raw := base64.StdEncoding.EncodeToString(ed25519.Sign(private, []byte(artifact)))
	// `base64` wraps at 76 columns by default, and an operator pasting the file
	// contents should not have to know that.
	wrapped := raw[:40] + "\n" + raw[40:] + "\n"
	if _, err := store.Publish(
		manifest(wrapped), strings.NewReader(artifact),
	); err != nil {
		t.Fatalf("Publish() rejected a wrapped signature: %v", err)
	}
}

func TestPublishValidationMessagesNameTheField(t *testing.T) {
	store := newStore(t)
	private, _ := signingPair(t)
	signature := base64.StdEncoding.EncodeToString(ed25519.Sign(private, []byte("x")))
	cases := []struct {
		name     string
		mutate   func(*Manifest)
		contains string
	}{
		{"version", func(m *Manifest) { m.Version = "1.2" }, "version"},
		{"channel", func(m *Manifest) { m.Channel = "nightly" }, "channel"},
		{"architecture", func(m *Manifest) { m.Architecture = "riscv" }, "architecture"},
		{"rollout", func(m *Manifest) { m.Rollout = 140 }, "rollout"},
		{"signature", func(m *Manifest) { m.Signature = "not-base64!" }, "signature"},
	}
	for _, testCase := range cases {
		candidate := manifest(signature)
		testCase.mutate(&candidate)
		_, err := store.Publish(candidate, strings.NewReader("x"))
		if err == nil {
			t.Fatalf("%s: Publish() accepted invalid metadata", testCase.name)
		}
		if !strings.Contains(strings.ToLower(err.Error()), testCase.contains) {
			t.Fatalf("%s: error %q does not name the field", testCase.name, err)
		}
	}
	if _, err := store.Publish(manifest(signature), strings.NewReader("")); err == nil {
		t.Fatal("Publish() accepted an empty artifact")
	}
}

func TestPublishRefusesToOverwriteAnExistingRelease(t *testing.T) {
	store := newStore(t)
	private, _ := signingPair(t)
	artifact := "agent-binary"
	signature := base64.StdEncoding.EncodeToString(
		ed25519.Sign(private, []byte(artifact)),
	)
	if _, err := store.Publish(manifest(signature), strings.NewReader(artifact)); err != nil {
		t.Fatal(err)
	}
	// Republishing the same version with different bytes would change what a
	// host downloads without changing the version it reports.
	_, err := store.Publish(manifest(signature), strings.NewReader("different"))
	if err == nil || !strings.Contains(err.Error(), "already published") {
		t.Fatalf("Publish() error = %v, want an already-published refusal", err)
	}
}

// Staged rollout and an emergency stop had to be possible without re-uploading a
// multi-megabyte artifact under a new version number.
func TestRolloutCanBeWidenedAndHalted(t *testing.T) {
	store := newStore(t)
	private, _ := signingPair(t)
	artifact := "agent-binary"
	signature := base64.StdEncoding.EncodeToString(
		ed25519.Sign(private, []byte(artifact)),
	)
	published, err := store.Publish(
		Manifest{
			Version: "1.2.3", Channel: "stable", OS: "linux",
			Architecture: "x86_64", Signature: signature, Rollout: 10,
		},
		strings.NewReader(artifact),
	)
	if err != nil {
		t.Fatal(err)
	}
	base := "1.2.3-linux-x86_64"

	// A canary rollout must reach roughly its share of the fleet and no more.
	selected := 0
	const fleet = 4000
	agents := make([]string, fleet)
	for index := range agents {
		agents[index] = uuid.NewString()
	}
	for _, agent := range agents {
		found, err := store.Latest("stable", "linux", "x86_64", agent)
		if err != nil {
			t.Fatal(err)
		}
		if found != nil {
			selected++
		}
	}
	share := float64(selected) * 100 / fleet
	if share < 7 || share > 13 {
		t.Fatalf("a 10%% rollout reached %.1f%% of the fleet", share)
	}

	if _, err := store.SetRollout(base, 100); err != nil {
		t.Fatal(err)
	}
	for _, agent := range agents[:200] {
		found, err := store.Latest("stable", "linux", "x86_64", agent)
		if err != nil {
			t.Fatal(err)
		}
		if found == nil {
			t.Fatal("a full rollout skipped an agent")
		}
	}

	// The emergency stop: no agent may be offered the release again, and it must
	// take effect without deleting anything.
	halted, err := store.SetRollout(base, 0)
	if err != nil {
		t.Fatal(err)
	}
	if halted.Rollout != 0 {
		t.Fatalf("halted rollout = %d", halted.Rollout)
	}
	for _, agent := range agents[:200] {
		found, err := store.Latest("stable", "linux", "x86_64", agent)
		if err != nil {
			t.Fatal(err)
		}
		if found != nil {
			t.Fatal("a halted release was still offered")
		}
	}
	if published.Version != "1.2.3" {
		t.Fatalf("published = %+v", published)
	}
	// Retiring removes it entirely.
	if err := store.Retire(base); err != nil {
		t.Fatal(err)
	}
	releases, err := store.Releases()
	if err != nil {
		t.Fatal(err)
	}
	if len(releases) != 0 {
		t.Fatalf("retire left %d releases", len(releases))
	}
	if err := store.Retire("../escape"); err == nil {
		t.Fatal("Retire() accepted a traversal attempt")
	}
}

func TestReleasesAreListedNewestFirst(t *testing.T) {
	store := newStore(t)
	private, _ := signingPair(t)
	for _, version := range []string{"1.2.3", "1.10.0", "1.9.9"} {
		artifact := "agent-" + version
		signature := base64.StdEncoding.EncodeToString(
			ed25519.Sign(private, []byte(artifact)),
		)
		if _, err := store.Publish(
			Manifest{
				Version: version, Channel: "stable", OS: "linux",
				Architecture: "x86_64", Signature: signature, Rollout: 100,
			},
			strings.NewReader(artifact),
		); err != nil {
			t.Fatal(err)
		}
	}
	releases, err := store.Releases()
	if err != nil {
		t.Fatal(err)
	}
	// 1.10.0 must sort above 1.9.9: version ordering is numeric, not lexical.
	got := []string{releases[0].Version, releases[1].Version, releases[2].Version}
	want := []string{"1.10.0", "1.9.9", "1.2.3"}
	for index := range want {
		if got[index] != want[index] {
			t.Fatalf("releases = %v, want %v", got, want)
		}
	}
}

// The rollout bucket decides which hosts a canary reaches, so its uniformity is
// a load-bearing property rather than an implementation detail.
func TestRolloutBucketsAreUniform(t *testing.T) {
	const fleet = 20000
	counts := make([]int, 100)
	for index := 0; index < fleet; index++ {
		counts[rolloutBucket(uuid.NewString())]++
	}
	expected := fleet / 100
	for bucket, count := range counts {
		if count < expected/2 || count > expected*2 {
			t.Fatalf(
				"bucket %d holds %d agents, expected about %d",
				bucket, count, expected,
			)
		}
	}
}

func TestParsePublicKeyRejectsTheWrongLength(t *testing.T) {
	if _, err := ParsePublicKey("c2hvcnQ="); err == nil {
		t.Fatal("ParsePublicKey() accepted a short key")
	}
	key, err := ParsePublicKey("")
	if err != nil || key != nil {
		t.Fatalf("an empty key must mean 'not configured': %v %v", key, err)
	}
}
