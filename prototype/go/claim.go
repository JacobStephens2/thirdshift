package main

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"
)

func makeClaim(ctx context.Context, issue issueURL, data issueData) error {
	if _, err := command(ctx, "", "gh", "label", "create", "in-progress", "--repo", issue.repo,
		"--description", "A Claim: thirdshift has taken this issue", "--force"); err != nil {
		return err
	}
	return changeLabels(ctx, issue, data, []string{"in-progress"}, []string{"ready-for-agent", "agent-failed"})
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
	var add, remove []string
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
			add = append(add, "agent-failed")
		} else {
			if !initial.hasLabel("in-progress") {
				remove = append(remove, "in-progress")
			}
			if initial.hasLabel("ready-for-agent") {
				add = append(add, "ready-for-agent")
			}
		}
	} else {
		remove = append(remove, "in-progress")
		if data.hasLabel("agent-failed") {
			remove = append(remove, "agent-failed")
		}
	}
	if len(add) == 0 && len(remove) == 0 {
		return nil
	}
	return changeLabels(ctx, issue, data, add, remove)
}
