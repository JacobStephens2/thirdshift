package main

import (
	"context"
	"fmt"
)

func makeClaim(ctx context.Context, issue issueURL, data issueData) error {
	if _, err := command(ctx, "", "gh", "label", "create", "in-progress", "--repo", issue.repo,
		"--description", "A Claim: thirdshift has taken this issue", "--force"); err != nil {
		return err
	}
	args := []string{"issue", "edit", issue.number, "--repo", issue.repo, "--add-label", "in-progress"}
	for _, label := range []string{"ready-for-agent", "agent-failed"} {
		if data.hasLabel(label) {
			args = append(args, "--remove-label", label)
		}
	}
	_, err := command(ctx, "", "gh", args...)
	return err
}

// This slice ends before Delivery: release only when nothing remains on origin.
// Never reuse the cancelled Ticket-run context here.
func endClaim(ctx context.Context, launch string, issue issueURL, branch string, initial issueData, failed bool) error {
	data, err := readIssue(ctx, issue)
	if err != nil {
		return err
	}
	if !data.hasLabel("in-progress") {
		return nil
	}
	args := []string{"issue", "edit", issue.number, "--repo", issue.repo}
	unchanged := len(args)
	if data.State == "open" {
		refs, err := command(ctx, launch, "git", "ls-remote", "--heads", "origin", "refs/heads/"+branch)
		if err != nil {
			return fmt.Errorf("ending Claim: %w", err)
		}
		var prs []struct {
			Number int `json:"number"`
		}
		if err := ghJSON(ctx, &prs, "pr", "list", "--repo", issue.repo, "--head", branch, "--state", "open", "--json", "number"); err != nil {
			return err
		}
		if len(refs) != 0 || len(prs) != 0 {
			if !failed {
				return nil
			}
			if _, err := command(ctx, "", "gh", "label", "create", "agent-failed", "--repo", issue.repo, "--description", "Failed Ticket run", "--force"); err != nil {
				return err
			}
			args = append(args, "--add-label", "agent-failed")
		} else {
			if !initial.hasLabel("in-progress") {
				args = append(args, "--remove-label", "in-progress")
			}
			if initial.hasLabel("ready-for-agent") {
				args = append(args, "--add-label", "ready-for-agent")
			}
		}
	} else {
		args = append(args, "--remove-label", "in-progress")
		if data.hasLabel("agent-failed") {
			args = append(args, "--remove-label", "agent-failed")
		}
	}
	if len(args) == unchanged {
		return nil
	}
	_, err = command(ctx, "", "gh", args...)
	return err
}
