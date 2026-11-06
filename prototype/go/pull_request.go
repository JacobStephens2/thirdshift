package main

import (
	"context"
	"fmt"
	"strconv"
)

type pullRequest struct {
	Number                                                      int
	URL, State, HeadRefName, BaseRefName, HeadRefOid, Mergeable string
	IsDraft, IsCrossRepository                                  bool
}

// Capture a single PR number, then use that number for every later observation
// and mutation. A closed PR is never replaced or reopened by Delivery.
// A fork pull request is not this repository's branch and is ignored.
func (d *delivery) findPR(ctx context.Context) error {
	if d.prNumber != 0 {
		_, err := d.observePR(ctx, "")
		return err
	}
	var prs []struct {
		Number            int
		IsCrossRepository bool
	}
	if err := ghJSON(ctx, &prs, "pr", "list", "--repo", d.issue.repo, "--head", d.branch,
		"--state", "all", "--json", "number,isCrossRepository"); err != nil {
		return err
	}
	var same []int
	for _, pr := range prs {
		if !pr.IsCrossRepository {
			same = append(same, pr.Number)
		}
	}
	if len(same) > 1 {
		return fmt.Errorf("multiple pull requests for %s", d.branch)
	}
	if len(same) == 1 {
		d.prNumber = same[0]
		_, err := d.observePR(ctx, "")
		return err
	}
	return nil
}

func (d *delivery) observePR(ctx context.Context, head string) (pullRequest, error) {
	var pr pullRequest
	err := ghJSON(ctx, &pr, "pr", "view", strconv.Itoa(d.prNumber), "--repo", d.issue.repo,
		"--json", "number,url,state,isDraft,headRefName,baseRefName,headRefOid,mergeable,isCrossRepository")
	if err != nil {
		return pr, err
	}
	if pr.Number != d.prNumber || pr.State != "OPEN" || pr.HeadRefName != d.branch ||
		pr.BaseRefName != d.base || pr.IsCrossRepository {
		return pr, fmt.Errorf("pull request is no longer actionable: #%d state=%s head=%s base=%s", pr.Number, pr.State, pr.HeadRefName, pr.BaseRefName)
	}
	if head != "" && pr.HeadRefOid != head {
		return pr, fmt.Errorf("pull request head moved: wanted %s, got %s", head, pr.HeadRefOid)
	}
	return pr, nil
}

func (d *delivery) publish(ctx context.Context) (string, error) {
	if err := d.findPR(ctx); err != nil {
		return "", err
	}
	if _, err := command(ctx, d.worktree, "git", "push", "origin", "HEAD:refs/heads/"+d.branch); err != nil {
		return "", err
	}
	if d.prNumber == 0 {
		if _, err := command(ctx, d.worktree, "gh", "pr", "create", "--repo", d.issue.repo,
			"--head", d.branch, "--base", d.base, "--draft", "--title", d.title,
			"--body", "Closes #"+d.issue.number+"\n\nGo prototype Ticket run; Delivery is checking this head."); err != nil {
			return "", err
		}
		if err := d.findPR(ctx); err != nil {
			return "", err
		}
		if d.prNumber == 0 {
			return "", fmt.Errorf("published pull request was not found")
		}
	}
	head, err := command(ctx, d.worktree, "git", "rev-parse", "HEAD")
	if err != nil {
		return "", err
	}
	_, err = d.observePR(ctx, string(head))
	return string(head), err
}

func (d *delivery) draft(ctx context.Context) error {
	pr, err := d.observePR(ctx, "")
	if err != nil || pr.IsDraft {
		return err
	}
	_, err = command(ctx, d.worktree, "gh", "pr", "ready", strconv.Itoa(d.prNumber), "--repo", d.issue.repo, "--undo")
	return err
}
