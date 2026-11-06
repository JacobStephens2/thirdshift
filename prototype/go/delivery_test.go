package main_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func (f *fixture) assertPR(draft bool) {
	f.t.Helper()
	data, err := os.ReadFile(filepath.Join(f.root, "pr.json"))
	if err != nil {
		f.t.Fatalf("Delivery did not publish a PR: %v", err)
	}
	var pr struct {
		Base  string `json:"baseRefName"`
		Head  string `json:"headRefName"`
		Draft bool   `json:"isDraft"`
	}
	if err := json.Unmarshal(data, &pr); err != nil {
		f.t.Fatal(err)
	}
	if pr.Base != "main" || pr.Head != "issue-7" || pr.Draft != draft {
		f.t.Fatalf("PR = %s; want issue-7 -> main, draft=%v", data, draft)
	}
	if f.git(f.repo, "ls-remote", "--heads", "origin", "issue-7") == "" {
		f.t.Fatal("Issue branch was not pushed")
	}
}

func TestDeliveryPublishesGreenHead(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "delivery")
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err != nil {
		t.Fatalf("Delivery: %v: %s", err, output)
	}
	f.assertPR(false)
	f.assertLabels("enhancement", "in-progress")
}

func TestDeliveryRepairsFailedCI(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "ci-repair")
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err != nil {
		t.Fatalf("CI Repair: %v: %s", err, output)
	}
	f.assertPR(false)
	if got := f.git(f.repo, "show", "origin/issue-7:widget.txt"); got != "repaired" {
		t.Fatalf("CI Repair was not delivered: %q", got)
	}
	f.assertLabels("enhancement", "in-progress")
}

func TestDeliveryRepairsBaseMergeConflict(t *testing.T) {
	f := newFixture(t)
	if err := os.WriteFile(filepath.Join(f.repo, "widget.txt"), []byte("original\n"), 0600); err != nil {
		t.Fatal(err)
	}
	f.git(f.repo, "add", "widget.txt")
	f.git(f.repo, "commit", "-m", "Original widget")
	f.git(f.repo, "push", "origin", "main")
	f.writeState("open", "OWNER", "conflict-repair")
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err != nil {
		t.Fatalf("conflict Repair: %v: %s", err, output)
	}
	f.assertPR(false)
	if got := f.git(f.repo, "show", "origin/issue-7:widget.txt"); got != "resolved" {
		t.Fatalf("conflict Repair = %q", got)
	}
	parents := strings.Fields(f.git(f.repo, "rev-list", "--parents", "-n", "1", "origin/issue-7"))
	if len(parents) != 3 {
		t.Fatalf("Repair did not commit the Base branch merge: %v", parents)
	}
	f.git(f.repo, "merge-base", "--is-ancestor", "origin/main", "origin/issue-7")
}

func TestFailedTicketRunSalvagesDraft(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "salvage")
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err == nil {
		t.Fatalf("failed Harness succeeded: %s", output)
	}
	f.assertPR(true)
	f.assertLabels("enhancement", "in-progress", "agent-failed")
	if got := f.git(f.repo, "show", "origin/issue-7:widget.txt"); got != "broken" {
		t.Fatalf("committed work lost: %q", got)
	}
}

func TestDeliveryEnforcesRepairLimit(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "ci-still-red")
	output, err := f.run("--repair-limit", "2", "https://github.com/acme/widgets/issues/7")
	if err == nil || !strings.Contains(output, "Repair limit reached (2)") {
		t.Fatalf("Repair budget: %v: %s", err, output)
	}
	f.assertPR(true)
	f.assertLabels("enhancement", "in-progress", "agent-failed")
	sessions, err := os.ReadFile(filepath.Join(f.root, "sessions.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if got := len(strings.Split(strings.TrimSpace(string(sessions)), "\n")); got != 3 {
		t.Fatalf("opening + two Repairs = %d sessions", got)
	}
	if got := f.git(f.repo, "show", "origin/issue-7:widget.txt"); got != "attempt 3" {
		t.Fatalf("last Repair was not pushed: %q", got)
	}
}

func TestDeliveryWatchesPendingChecksAndStatuses(t *testing.T) {
	for _, mode := range []string{"pending-check", "pending-status", "no-ci"} {
		t.Run(mode, func(t *testing.T) {
			f := newFixture(t)
			f.writeState("open", "OWNER", mode)
			output, err := f.run("https://github.com/acme/widgets/issues/7")
			if err != nil {
				t.Fatalf("CI watch: %v: %s", err, output)
			}
			f.assertPR(false)
		})
	}
}

func TestDeliveryDoesNotTrustDisappearedChecks(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "checks-disappear")
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err == nil || !strings.Contains(output, "CI checks disappeared") {
		t.Fatalf("disappeared CI: %v: %s", err, output)
	}
	f.assertPR(true)
}

func TestDeliveryStopsWhenPullRequestCloses(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "pr-closes")
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err == nil || !strings.Contains(output, "state=CLOSED") {
		t.Fatalf("closed PR gate: %v: %s", err, output)
	}
	calls, err := os.ReadFile(filepath.Join(f.root, "calls.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(calls), `"ready"`) {
		t.Fatalf("edited closed PR: %s", calls)
	}
}
