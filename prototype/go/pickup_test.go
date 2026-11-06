package main_test

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"
)

func (f *fixture) pickupConfig(config map[string]any) {
	f.t.Helper()
	data, err := json.Marshal(config)
	if err != nil {
		f.t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(f.root, "pickup.json"), data, 0600); err != nil {
		f.t.Fatal(err)
	}
}

func labelEvent(event, label, actor, at string) map[string]any {
	return map[string]any{"event": event, "label": map[string]string{"name": label},
		"actor": map[string]string{"login": actor}, "created_at": at}
}

func readyEvents() [][]map[string]any {
	return [][]map[string]any{{labelEvent("labeled", "ready-for-agent", "acme", "2020-01-01T00:00:00Z")}}
}

func (f *fixture) assertNoSession() {
	f.t.Helper()
	if data, err := os.ReadFile(filepath.Join(f.root, "sessions.jsonl")); !os.IsNotExist(err) {
		f.t.Fatalf("unexpected session: %s (%v)", data, err)
	}
}

func TestPickupContentionMovesToNextIssue(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "reservation")
	f.pickupConfig(map[string]any{"pages": [][]map[string]int{{{"number": 8}, {"number": 7}}}})
	token := f.git(f.repo, "rev-parse", "HEAD")
	f.git(f.repo, "push", "origin", token+":refs/heads/thirdshift/pickup/7")
	output, err := f.run("pickup")
	if err != nil || !strings.Contains(output, `"result":"done"`) {
		t.Fatalf("Pickup run: %v: %s", err, output)
	}
	if got := f.git(filepath.Join(f.root, "repo-issue-8"), "branch", "--show-current"); got != "issue-8" {
		t.Fatalf("selected %s, want issue-8", got)
	}
	if got := f.git(f.repo, "ls-remote", "origin", "refs/heads/thirdshift/pickup/7"); !strings.HasPrefix(got, token+"\t") {
		t.Fatalf("changed competing reservation: %s", got)
	}
}

func TestPickupRechecksIssueAndCompleteHistory(t *testing.T) {
	for _, change := range []string{"closed", "unready", "active-claim", "intervening-claim", "untrusted-label", "edited"} {
		t.Run(change, func(t *testing.T) {
			f := newFixture(t)
			refresh := map[string]any{}
			switch change {
			case "closed":
				refresh["data"] = map[string]any{"state": "closed"}
			case "unready":
				refresh["data"] = map[string]any{"labels": []map[string]string{{"name": "needs-info"}}}
			case "active-claim", "intervening-claim":
				events := readyEvents()
				events = append(events, []map[string]any{labelEvent("labeled", "in-progress", "acme", "2020-01-02T00:00:00Z")})
				if change == "intervening-claim" {
					events = append(events, []map[string]any{
						labelEvent("unlabeled", "in-progress", "acme", "2020-01-02T00:00:00Z"),
						labelEvent("labeled", "ready-for-agent", "acme", "2020-01-02T00:00:00Z"),
					})
				}
				refresh["events"] = events
			case "untrusted-label":
				refresh["events"] = [][]map[string]any{{labelEvent("labeled", "ready-for-agent", "outsider", "2020-01-01T00:00:00Z")}}
			case "edited":
				refresh["edit_pages"] = [][]map[string]any{{}, {{"editedAt": "2020-01-02T00:00:00Z", "editor": map[string]string{"login": "outsider"}}}}
			}
			f.pickupConfig(map[string]any{"pages": [][]map[string]int{{{"number": 7}, {"number": 8}}},
				"issues": map[string]any{"7": map[string]any{"refresh": refresh}}})
			output, err := f.run("pickup")
			if err != nil || !strings.Contains(output, `"result":"done"`) {
				t.Fatalf("Pickup run: %v: %s", err, output)
			}
			if _, err := os.Stat(f.worktree); !os.IsNotExist(err) {
				t.Fatalf("stale issue dispatched: %v", err)
			}
			if got := f.git(filepath.Join(f.root, "repo-issue-8"), "branch", "--show-current"); got != "issue-8" {
				t.Fatalf("selected %s", got)
			}
			if got := f.git(f.repo, "ls-remote", "origin", "refs/heads/thirdshift/pickup/*"); got != "" {
				t.Fatalf("stale reservation retained: %s", got)
			}
		})
	}
}

func TestPickupRejectsUnreadyOrUntrustedCandidates(t *testing.T) {
	for _, gate := range []string{"author", "latest-label", "no-label-history", "title-edit", "body-edit", "active-history", "released-without-handback", "unsettled", "parent", "blocker", "spec", "triage", "started", "pr"} {
		t.Run(gate, func(t *testing.T) {
			f := newFixture(t)
			config := map[string]any{}
			pickup := map[string]any{"issues": map[string]any{"7": config}}
			switch gate {
			case "author":
				config["data"] = map[string]any{"author_association": "NONE"}
			case "latest-label":
				config["events"] = append(readyEvents(), []map[string]any{labelEvent("labeled", "ready-for-agent", "outsider", "2020-01-02T00:00:00Z")})
			case "no-label-history":
				config["events"] = [][]map[string]any{}
			case "title-edit":
				config["events"] = append(readyEvents(), []map[string]any{{"event": "renamed", "created_at": "2020-01-02T00:00:00Z", "actor": map[string]string{"login": "outsider"}}})
			case "body-edit":
				config["edit_pages"] = [][]map[string]any{{}, {{"editedAt": "2020-01-02T00:00:00Z", "editor": nil}}}
			case "active-history", "released-without-handback":
				events := append(readyEvents(), []map[string]any{labelEvent("labeled", "in-progress", "acme", "2020-01-02T00:00:00Z")})
				if gate == "released-without-handback" {
					events = append(events, []map[string]any{labelEvent("unlabeled", "in-progress", "acme", "2020-01-03T00:00:00Z")})
				}
				config["events"] = events
			case "unsettled":
				config["events"] = [][]map[string]any{{labelEvent("labeled", "ready-for-agent", "acme", time.Now().UTC().Format(time.RFC3339))}}
			case "parent":
				config["facts"] = map[string]any{"parent": map[string]string{"url": "https://github.com/acme/widgets/issues/1"}}
			case "blocker":
				config["facts"] = map[string]any{"blockedBy": map[string]any{"nodes": []map[string]string{{"state": "OPEN"}}}}
			case "spec":
				config["facts"] = map[string]any{"subIssues": map[string]int{"totalCount": 1}}
			case "triage":
				config["data"] = map[string]any{"labels": []map[string]string{{"name": "ready-for-agent"}, {"name": "needs-triage"}}}
			case "started":
				f.git(f.repo, "push", "origin", "HEAD:refs/heads/issue-7")
			case "pr":
				pickup["prs"] = map[string]any{"issue-7": []map[string]any{{"headRefName": "issue-7-branch-2", "isCrossRepository": false}}}
			}
			f.pickupConfig(pickup)
			output, err := f.run("pickup")
			if err != nil || !strings.Contains(output, "Pickup skipped") {
				t.Fatalf("wanted skip: %v: %s", err, output)
			}
			f.assertNoSession()
			f.assertLabels("enhancement", "ready-for-agent")
		})
	}
}

func TestPickupSameIdentityOtherCloneCannotDispatchReservedIssue(t *testing.T) {
	f := newFixture(t)
	_, done := f.start("stream", "pickup")
	waitFor(t, "reserved Harness session", func() bool { return strings.Contains(f.output(), `"session_id"`) })
	other := filepath.Join(f.root, "other")
	f.git(f.root, "clone", "-b", "main", filepath.Join(f.root, "origin.git"), other)
	f.git(other, "remote", "set-url", "origin", "https://github.com/acme/widgets.git")
	f.git(other, "config", "url."+filepath.Join(f.root, "origin.git")+".insteadOf", "https://github.com/acme/widgets.git")
	// Simulate a stale GitHub read while the first host is still running.
	f.writeState("open", "OWNER", "stream")
	cmd := exec.Command(binary, "pickup")
	cmd.Dir, cmd.Env = other, f.env
	output, err := cmd.CombinedOutput()
	if err != nil || !strings.Contains(string(output), "Pickup skipped") {
		t.Fatalf("competing clone: %v: %s", err, output)
	}
	if err := os.WriteFile(filepath.Join(f.root, "finish"), nil, 0600); err != nil {
		t.Fatal(err)
	}
	if err := <-done; err != nil {
		t.Fatalf("owner: %v: %s", err, f.output())
	}
	sessions, err := os.ReadFile(filepath.Join(f.root, "sessions.jsonl"))
	if err != nil || len(strings.Split(strings.TrimSpace(string(sessions)), "\n")) != 1 {
		t.Fatalf("duplicate dispatch: %s, %v", sessions, err)
	}
}

func TestPickupReservationEndsAfterInterruption(t *testing.T) {
	f := newFixture(t)
	cmd, done := f.start("stream", "pickup")
	waitFor(t, "reserved Harness session", func() bool { return strings.Contains(f.output(), `"session_id"`) })
	if err := cmd.Process.Signal(syscall.SIGINT); err != nil {
		t.Fatal(err)
	}
	if err := <-done; err == nil {
		t.Fatalf("interrupted command succeeded: %s", f.output())
	}
	if got := f.git(f.repo, "ls-remote", "origin", "refs/heads/thirdshift/pickup/*"); got != "" {
		t.Fatalf("interruption retained reservation: %s", got)
	}
	f.assertLabels("enhancement", "ready-for-agent")
}

func TestPickupReleaseNeverDeletesChangedOwner(t *testing.T) {
	f := newFixture(t)
	_, done := f.start("stream", "pickup")
	waitFor(t, "reserved Harness session", func() bool { return strings.Contains(f.output(), `"session_id"`) })
	replacement := f.git(f.repo, "rev-parse", "HEAD")
	f.git(f.repo, "push", "--force", "origin", replacement+":refs/heads/thirdshift/pickup/7")
	if err := os.WriteFile(filepath.Join(f.root, "finish"), nil, 0600); err != nil {
		t.Fatal(err)
	}
	if err := <-done; err == nil || !strings.Contains(f.output(), "our token") {
		t.Fatalf("wanted release recovery diagnostic: %v: %s", err, f.output())
	}
	if got := f.git(f.repo, "ls-remote", "origin", "refs/heads/thirdshift/pickup/7"); !strings.HasPrefix(got, replacement+"\t") {
		t.Fatalf("changed reservation deleted: %s", got)
	}
}

func TestPickupReservationPermissionFailureFailsClosed(t *testing.T) {
	f := newFixture(t)
	if err := os.WriteFile(filepath.Join(f.root, "origin.git/hooks/pre-receive"), []byte("#!/bin/sh\necho permission-denied >&2\nexit 1\n"), 0755); err != nil {
		t.Fatal(err)
	}
	output, err := f.run("pickup")
	if err == nil || !strings.Contains(output, "our token") || !strings.Contains(output, "refs/heads/thirdshift/pickup/7") {
		t.Fatalf("wanted fail-closed recovery diagnostic: %v: %s", err, output)
	}
	f.assertNoSession()
	f.assertLabels("enhancement", "ready-for-agent")
}

func TestPickupSameSecondHandbackAndTrustedMember(t *testing.T) {
	f := newFixture(t)
	events := append(readyEvents(), []map[string]any{
		labelEvent("labeled", "in-progress", "acme", "2020-01-02T00:00:00Z"),
		labelEvent("labeled", "ready-for-agent", "member", "2020-01-03T00:00:00Z"),
		labelEvent("unlabeled", "in-progress", "acme", "2020-01-03T00:00:00Z"),
	})
	f.pickupConfig(map[string]any{"issues": map[string]any{"7": map[string]any{"events": events}}, "members": map[string]int{"member": 204}})
	output, err := f.run("pickup")
	if err != nil || !strings.Contains(output, `"result":"done"`) {
		t.Fatalf("same-second hand-back: %v: %s", err, output)
	}
}

func TestPickupRefusesClaimWhenGateLabelAppearsAfterRead(t *testing.T) {
	for _, label := range []string{"wontfix", "priority"} {
		t.Run(label, func(t *testing.T) {
			f := newFixture(t)
			f.writeState("open", "OWNER", "reservation")
			f.pickupConfig(map[string]any{"poison_label": label})
			output, err := f.run("pickup")
			if label == "priority" {
				if err != nil || !strings.Contains(output, `"result":"done"`) {
					t.Fatalf("benign label should still dispatch: %v: %s", err, output)
				}
				f.assertLabels("enhancement", "priority", "ready-for-agent")
				return
			}
			if err != nil || !strings.Contains(output, "Pickup skipped") {
				t.Fatalf("wanted skip, got %v: %s", err, output)
			}
			f.assertNoSession()
			f.assertLabels("enhancement", "ready-for-agent", "wontfix")
			if got := f.git(f.repo, "ls-remote", "origin", "refs/heads/thirdshift/pickup/*"); got != "" {
				t.Fatalf("stale reservation retained: %s", got)
			}
		})
	}
}

func TestPickupSelectsLowestReadyAcrossPages(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "reservation")
	f.pickupConfig(map[string]any{"pages": [][]map[string]int{{{"number": 12}}, {{"number": 7}}}})
	output, err := f.run("pickup")
	if err != nil || !strings.Contains(output, `"result":"done"`) {
		t.Fatalf("Pickup run: %v: %s", err, output)
	}
	if got := f.git(f.worktree, "branch", "--show-current"); got != "issue-7" {
		t.Fatalf("selected %s, want issue-7", got)
	}
	if got := f.git(f.repo, "ls-remote", "--heads", "origin", "refs/heads/thirdshift/pickup/*"); got != "" {
		t.Fatalf("reservation not released: %s", got)
	}
	f.assertLabels("enhancement", "ready-for-agent")
}
