package main_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSpecSalvageStillPushesWhenDraftFails(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "salvage")
	f.git(f.repo, "push", "origin", "HEAD:refs/heads/issue-7")
	pr := `{"number":8,"url":"https://github.com/acme/widgets/pull/8","state":"OPEN","isDraft":false,"headRefName":"issue-7","baseRefName":"main","mergeable":"MERGEABLE"}`
	if err := os.WriteFile(filepath.Join(f.root, "pr.json"), []byte(pr), 0600); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(f.root, "tools", "gh")
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	marker := `elif args[:2] == ["pr", "ready"]:`
	replacement := marker + `
    if "--undo" in args:
        sys.exit("draft temporarily unavailable")`
	if !strings.Contains(string(raw), marker) {
		t.Fatal("fake-gh insertion marker changed")
	}
	if err := os.WriteFile(path, []byte(strings.Replace(string(raw), marker, replacement, 1)), 0755); err != nil {
		t.Fatal(err)
	}
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err == nil || !strings.Contains(output, "draft temporarily unavailable") {
		t.Fatalf("expected retained draft failure: %v: %s", err, output)
	}
	want := f.git(f.worktree, "rev-parse", "HEAD")
	got := f.git(f.repo, "rev-parse", "refs/remotes/origin/issue-7")
	if got != want {
		t.Fatalf("salvage did not push committed work after draft failed: origin=%s worktree=%s", got, want)
	}
}
