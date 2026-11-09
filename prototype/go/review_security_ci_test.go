package main_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A fast success must not attest readiness while another check has yet to
// register or a workflow on the same head remains open or completes failed.
func TestSecurityIncompleteCIIsNotAttestedReady(t *testing.T) {
	for _, mode := range []string{"partial-register", "open-workflow"} {
		t.Run(mode, func(t *testing.T) {
			f := newFixture(t)
			f.writeState("open", "OWNER", mode)
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
				t.Fatalf("ready pull request attests a head with incomplete CI: %q\nrun err=%v\n%s", got, err, output)
			}
		})
	}
}
