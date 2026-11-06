package main_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A fast successful check must not attest the head ready while a workflow run
// on that same commit is still open or has completed failed without a job.
func TestSecurityOpenWorkflowIsNotAttestedReady(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "open-workflow")
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
		t.Fatalf("ready pull request attests a head whose workflow run was not green: %q\nrun err=%v\n%s", got, err, output)
	}
}
