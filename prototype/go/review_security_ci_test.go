package main_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A ready pull request is the factory's attestation that the published head
// is green. A fast success that arrives before another check registers must
// not become that attestation.
func TestSecurityPartialRegistrationIsNotAttestedReady(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "partial-register")
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	data, readErr := os.ReadFile(filepath.Join(f.root, "pr.json"))
	if readErr != nil {
		return
	}
	var pr struct {
		Draft bool `json:"isDraft"`
	}
	if jsonErr := json.Unmarshal(data, &pr); jsonErr != nil {
		t.Fatal(jsonErr)
	}
	if pr.Draft {
		return
	}
	if got := strings.TrimSpace(f.git(f.repo, "show", "origin/issue-7:widget.txt")); got == "broken" {
		t.Fatalf("ready pull request attests a head whose later check failed during registration: %q\nrun err=%v\n%s", got, err, output)
	}
}
