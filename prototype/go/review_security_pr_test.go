package main_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A fork pull request uses the same head branch name. It is not this
// repository's pull request, and it must not stop Delivery from publishing.
func TestSecurityCrossRepositoryPullRequestDoesNotBlockPublish(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "delivery")
	f.git(f.repo, "push", "origin", "HEAD:refs/heads/issue-7")
	pr := `{"number":8,"url":"https://github.com/acme/widgets/pull/8","state":"OPEN","isDraft":false,"headRefName":"issue-7","baseRefName":"main","mergeable":"MERGEABLE","isCrossRepository":true}`
	if err := os.WriteFile(filepath.Join(f.root, "pr.json"), []byte(pr), 0600); err != nil {
		t.Fatal(err)
	}
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err != nil {
		t.Fatalf("cross-repository pull request blocked delivery: %v: %s", err, output)
	}
}

// An outsider can open a fork pull request whose head branch is issue-<n>.
// That pull request is not this repository's work. A Ticket run that publishes
// nothing must release its Claim, and must ask gh for isCrossRepository so a
// real listing can be filtered. A same-repository pull request still keeps it.
func TestSecurityForkPullRequestDoesNotKeepClaim(t *testing.T) {
	for _, cross := range []bool{false, true} {
		t.Run(map[bool]string{false: "same-repository", true: "fork"}[cross], func(t *testing.T) {
			f := newFixture(t)
			f.writeState("open", "OWNER", "success")
			f.pickupConfig(map[string]any{
				"prs": map[string]any{
					"issue-7": []map[string]any{{
						"number":            9,
						"headRefName":       "issue-7",
						"isCrossRepository": cross,
					}},
				},
			})
			output, err := f.run("https://github.com/acme/widgets/issues/7")
			if err != nil {
				t.Fatalf("Ticket run: %v: %s", err, output)
			}
			if cross {
				f.assertLabels("enhancement", "ready-for-agent")
			} else {
				f.assertLabels("enhancement", "in-progress")
			}
			calls, readErr := os.ReadFile(filepath.Join(f.root, "calls.jsonl"))
			if readErr != nil {
				t.Fatal(readErr)
			}
			if !strings.Contains(string(calls), "isCrossRepository") {
				t.Fatalf("claim ending did not ask whether the pull request is from a fork: %s", calls)
			}
		})
	}
}
