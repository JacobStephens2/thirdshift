package main_test

import (
	"os"
	"path/filepath"
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
