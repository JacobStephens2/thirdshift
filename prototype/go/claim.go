package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
)

// errStaleClaim means the issue read that authorized this Claim is no longer
// the issue's labels. Pickup passes over it. A direct Ticket run stops.
// The whole-label PUT must not erase that newer decision.
var errStaleClaim = errors.New("ready state changed after the readiness read")

// Labels that stop a Claim when they appear after the readiness snapshot.
// agent-failed and in-progress are included so a later failure or Claim is
// not cleared by this attempt's replacement.
var claimGateLabels = []string{
	"in-progress", "agent-failed", "needs-triage", "needs-info",
	"ready-for-human", "wontfix", "base-fix", "security-fix",
}

// The bool records whether the label PUT was attempted. Even a failed response
// may have applied our Claim; a rejected pre-flight read cannot have done so.
func makeClaim(ctx context.Context, issue issueURL, data issueData) (bool, error) {
	if _, err := command(ctx, "", "gh", "label", "create", "in-progress", "--repo", issue.repo,
		"--description", "A Claim: thirdshift has taken this issue", "--force"); err != nil {
		return false, err
	}
	// The readiness snapshot is from before history, trust and reservation
	// checks. Replace labels from a new read, and do not claim if that read
	// shows the Day shift closed the gate.
	fresh, err := readIssue(ctx, issue)
	if err != nil {
		return false, err
	}
	if err := rejectStaleClaim(data, fresh); err != nil {
		return false, err
	}
	return true, changeLabels(ctx, issue, fresh, []string{"in-progress"}, []string{"ready-for-agent", "agent-failed"})
}

func rejectStaleClaim(before, fresh issueData) error {
	if before.State == "open" && fresh.State != "open" {
		return fmt.Errorf("%w: issue is no longer open", errStaleClaim)
	}
	if len(before.PullRequest) == 0 && len(fresh.PullRequest) != 0 {
		return fmt.Errorf("%w: issue is now a pull request", errStaleClaim)
	}
	if before.hasLabel("ready-for-agent") && !fresh.hasLabel("ready-for-agent") {
		return fmt.Errorf("%w: ready-for-agent was removed", errStaleClaim)
	}
	for _, label := range claimGateLabels {
		if fresh.hasLabel(label) && !before.hasLabel(label) {
			return fmt.Errorf("%w: %s was applied", errStaleClaim, label)
		}
	}
	return nil
}

// One whole-label PUT matches the model's atomic Ready/Claim transition.
// gh issue edit starts independent add/remove mutations and cannot do that.
func changeLabels(ctx context.Context, issue issueURL, current issueData, add, remove []string) error {
	labels := []string{}
	for _, label := range current.Labels {
		if !slices.Contains(remove, label.Name) {
			labels = append(labels, label.Name)
		}
	}
	for _, label := range add {
		if !slices.Contains(labels, label) {
			labels = append(labels, label)
		}
	}
	payload, err := json.Marshal(struct {
		Labels []string `json:"labels"`
	}{labels})
	if err != nil {
		return err
	}
	_, err = commandInput(ctx, "", payload, "gh", "api", "repos/"+issue.repo+"/issues/"+issue.number+"/labels", "--method", "PUT", "--input", "-")
	return err
}

// Never reuse the cancelled Ticket-run context here.
func endClaim(ctx context.Context, launch string, issue issueURL, branch string, initial issueData, result runOutcome) error {
	data, err := readIssue(ctx, issue)
	if err != nil {
		return err
	}
	if !data.hasLabel("in-progress") {
		return nil
	}
	var add, remove []string
	state := claimClosed
	if data.State == "open" {
		refs, err := command(ctx, launch, "git", "ls-remote", "--heads", "origin", "refs/heads/"+branch)
		if err != nil {
			return fmt.Errorf("ending Claim: %w", err)
		}
		var prs []struct {
			Number            int
			IsCrossRepository bool
		}
		// A fork pull request uses this head name but is not the run's work.
		// Counting it would keep the Claim, or mark it failed, with nothing on origin.
		if err := ghJSON(ctx, &prs, "pr", "list", "--repo", issue.repo, "--head", branch, "--state", "open", "--json", "number,isCrossRepository"); err != nil {
			return err
		}
		ownPR := false
		for _, pr := range prs {
			if !pr.IsCrossRepository {
				ownPR = true
				break
			}
		}
		if len(refs) != 0 || ownPR {
			state = claimHeld
			if result.failed() {
				state = claimFailed
			}
		} else {
			state = claimReleased
		}
	}
	switch state {
	case claimHeld:
		return nil
	case claimFailed:
		if _, err := command(ctx, "", "gh", "label", "create", "agent-failed", "--repo", issue.repo, "--description", "Failed Ticket run", "--force"); err != nil {
			return err
		}
		add = append(add, "agent-failed")
	case claimReleased:
		if !initial.hasLabel("in-progress") {
			remove = append(remove, "in-progress")
		}
		if initial.hasLabel("ready-for-agent") {
			add = append(add, "ready-for-agent")
		}
	case claimClosed:
		remove = append(remove, "in-progress")
		if data.hasLabel("agent-failed") {
			remove = append(remove, "agent-failed")
		}
	}
	if len(add) == 0 && len(remove) == 0 {
		return nil
	}
	// Branch and PR reads sit between the ending snapshot and this PUT.
	// Replace from a current read so a label applied in that window is kept.
	fresh, err := readIssue(ctx, issue)
	if err != nil {
		return err
	}
	return changeLabels(ctx, issue, fresh, add, remove)
}
