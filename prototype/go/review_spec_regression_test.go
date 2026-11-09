package main_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSpecDelayedCIRegistration(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "delivery")
	path := filepath.Join(f.root, "tools", "gh")
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	marker := `        if mode == "no-ci" or (mode == "checks-disappear" and count > 0):`
	replacement := `        if mode == "delivery":
            checks = [] if count == 0 and widget == "broken\n" else [{"name": "test", "status": "completed", "conclusion": "failure" if widget == "broken\n" else "success"}]
` + marker
	if !strings.Contains(string(raw), marker) {
		t.Fatal("fake-gh insertion marker changed")
	}
	if err := os.WriteFile(path, []byte(strings.Replace(string(raw), marker, replacement, 1)), 0755); err != nil {
		t.Fatal(err)
	}
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err != nil {
		t.Fatalf("Delivery: %v: %s", err, output)
	}
	f.assertPR(false)
	if got := f.git(f.repo, "show", "origin/issue-7:widget.txt"); got != "repaired" {
		t.Fatalf("ready PR contains %q; delayed failing CI was never repaired", got)
	}
}

func TestSpecFailedPushStillDraftsExistingPR(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "salvage")
	f.git(f.repo, "push", "origin", "HEAD:refs/heads/issue-7")
	pr := `{"number":8,"url":"https://github.com/acme/widgets/pull/8","state":"OPEN","isDraft":false,"headRefName":"issue-7","baseRefName":"main","mergeable":"MERGEABLE"}`
	if err := os.WriteFile(filepath.Join(f.root, "pr.json"), []byte(pr), 0600); err != nil {
		t.Fatal(err)
	}
	hook := filepath.Join(f.root, "origin.git", "hooks", "pre-receive")
	if err := os.WriteFile(hook, []byte("#!/bin/sh\nexit 1\n"), 0755); err != nil {
		t.Fatal(err)
	}
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err == nil {
		t.Fatalf("failed Harness succeeded: %s", output)
	}
	if !strings.Contains(output, "pre-receive hook declined") {
		t.Fatalf("expected push rejection: %s", output)
	}
	f.assertLabels("enhancement", "in-progress", "agent-failed")
	f.assertPR(true)
}
