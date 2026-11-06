package main

import (
	"bytes"
	"context"
	"fmt"
	"os/exec"
	"sort"
	"strconv"
	"strings"
	"time"
)

type historyEvent struct {
	Event string                  `json:"event"`
	At    time.Time               `json:"created_at"`
	Actor *struct{ Login string } `json:"actor"`
	Label struct{ Name string }   `json:"label"`
}

// The complete event history determines the selection's Ready generation.
// Fold Claim events before Ready at a shared timestamp, keeping Claim order.
func readReady(ctx context.Context, launch string, issue issueURL) (issueData, int, bool, error) {
	data, err := readIssue(ctx, issue)
	if err != nil {
		return data, 0, false, err
	}
	if data.State != "open" || len(data.PullRequest) != 0 || !trusted(data.Association) || !data.hasLabel("ready-for-agent") {
		return data, 0, false, nil
	}
	for _, label := range []string{"in-progress", "needs-triage", "needs-info", "ready-for-human", "wontfix", "base-fix", "security-fix"} {
		if data.hasLabel(label) {
			return data, 0, false, nil
		}
	}
	var pages [][]historyEvent
	if err := ghJSON(ctx, &pages, "api", "repos/"+issue.repo+"/issues/"+issue.number+"/events?per_page=100", "--paginate", "--slurp"); err != nil {
		return data, 0, false, err
	}
	var events []historyEvent
	for _, page := range pages {
		events = append(events, page...)
	}
	for _, event := range events {
		if event.At.IsZero() {
			return data, 0, false, fmt.Errorf("issue history has no created_at")
		}
	}
	sort.SliceStable(events, func(i, j int) bool {
		if events[i].At.Equal(events[j].At) {
			return events[i].Label.Name != "ready-for-agent" && events[j].Label.Name == "ready-for-agent"
		}
		return events[i].At.Before(events[j].At)
	})
	var readyAt time.Time
	var readyActor string
	count, claimed, released, handedBack := 0, false, false, false
	for _, event := range events {
		applied := event.Event == "labeled"
		if event.Event != "labeled" && event.Event != "unlabeled" {
			continue
		}
		switch event.Label.Name {
		case "in-progress":
			if applied {
				count++
				claimed, released, handedBack = true, false, false
			} else if claimed {
				claimed, released = false, true
			}
		case "ready-for-agent":
			if applied {
				readyAt, readyActor = event.At, ""
				if event.Actor != nil {
					readyActor = event.Actor.Login
				}
				handedBack = released && !claimed
			} else {
				handedBack = false
			}
		}
	}
	if claimed || readyAt.IsZero() || time.Since(readyAt) < 30*time.Minute {
		return data, count, false, nil
	}
	ok, err := trustedActor(ctx, issue.repo, readyActor)
	if err != nil || !ok {
		return data, count, false, err
	}
	for _, event := range events {
		if event.Event != "renamed" || !event.At.After(readyAt) {
			continue
		}
		actor := ""
		if event.Actor != nil {
			actor = event.Actor.Login
		}
		ok, err := trustedActor(ctx, issue.repo, actor)
		if err != nil || !ok {
			return data, count, false, err
		}
	}
	ok, err = readyFacts(ctx, issue, readyAt)
	if err != nil || !ok {
		return data, count, false, err
	}
	refs, err := command(ctx, launch, "git", "ls-remote", "--heads", "origin", "refs/heads/issue-"+issue.number, "refs/heads/issue-"+issue.number+"-branch-*")
	if err != nil {
		return data, count, false, err
	}
	firstBranch := "issue-" + issue.number
	var prs []struct {
		HeadRefName       string
		IsCrossRepository bool
	}
	if err := ghJSON(ctx, &prs, "pr", "list", "--repo", issue.repo, "--search", "head:"+firstBranch, "--state", "all", "--limit", "1000", "--json", "headRefName,isCrossRepository"); err != nil {
		return data, count, false, err
	}
	started := len(refs) != 0 || len(prs) == 1000
	for _, pr := range prs {
		suffix := strings.TrimPrefix(pr.HeadRefName, firstBranch+"-branch-")
		number, _ := strconv.Atoi(suffix)
		if !pr.IsCrossRepository && (pr.HeadRefName == firstBranch || (suffix != pr.HeadRefName && number >= 2)) {
			started = true
		}
	}
	return data, count, handedBack || (count == 0 && !started), nil
}

func trustedActor(ctx context.Context, repo, login string) (bool, error) {
	if login == "" {
		return false, nil
	}
	owner := strings.SplitN(repo, "/", 2)[0]
	if strings.EqualFold(owner, login) {
		return true, nil
	}
	ok, err := membership(ctx, "repos/"+repo+"/collaborators/"+login)
	if err != nil || ok {
		return ok, err
	}
	var repository struct{ Owner struct{ Type string } }
	if err := ghJSON(ctx, &repository, "api", "repos/"+repo); err != nil {
		return false, err
	}
	if repository.Owner.Type == "Organization" {
		return membership(ctx, "orgs/"+owner+"/members/"+login)
	}
	return false, nil
}

func membership(ctx context.Context, path string) (bool, error) {
	cmd := exec.Command("gh", "api", "--include", path)
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	err := supervise(ctx, cmd)
	fields := strings.Fields(strings.SplitN(stdout.String(), "\n", 2)[0])
	if len(fields) >= 2 {
		if fields[1] == "204" && err == nil {
			return true, nil
		}
		if fields[1] == "404" && ctx.Err() == nil {
			return false, nil
		}
	}
	return false, fmt.Errorf("could not check Trusted account membership for %s: %v: %s", path, err, stderr.String())
}

const readyFactsQuery = `query($owner: String!, $repo: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      parent { url }
      subIssues(first: 1) { totalCount }
      blockedBy(first: 100) { nodes { state } pageInfo { hasNextPage } }
      shaped: timelineItems(last: 1, itemTypes: [SUB_ISSUE_ADDED_EVENT, SUB_ISSUE_REMOVED_EVENT, BLOCKED_BY_ADDED_EVENT, BLOCKED_BY_REMOVED_EVENT]) {
        nodes {
          ... on SubIssueAddedEvent { createdAt }
          ... on SubIssueRemovedEvent { createdAt }
          ... on BlockedByAddedEvent { createdAt }
          ... on BlockedByRemovedEvent { createdAt }
        }
      }
      edits: userContentEdits(first: 100, after: $after) {
        nodes { editedAt editor { login } }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}`

// This vertical slice dispatches Ticket runs only. Specs are passed over,
// rather than executing a multi-Ticket Spec as if it were one Ticket.
func readyFacts(ctx context.Context, issue issueURL, readyAt time.Time) (bool, error) {
	parts := strings.SplitN(issue.repo, "/", 2)
	after := ""
	for {
		var response struct {
			Errors []struct{ Message string }
			Data   struct {
				Repository struct {
					Issue *struct {
						Parent    *struct{ URL string }
						SubIssues struct{ TotalCount int }
						BlockedBy struct {
							Nodes    []struct{ State string }
							PageInfo struct{ HasNextPage bool }
						}
						Shaped struct {
							Nodes []struct{ CreatedAt time.Time }
						}
						Edits struct {
							Nodes []struct {
								EditedAt time.Time
								Editor   *struct{ Login string }
							}
							PageInfo struct {
								HasNextPage bool
								EndCursor   string
							}
						}
					}
				}
			}
		}
		args := []string{"api", "graphql", "-f", "query=" + readyFactsQuery, "-f", "owner=" + parts[0], "-f", "repo=" + parts[1], "-F", "number=" + issue.number}
		if after != "" {
			args = append(args, "-f", "after="+after)
		}
		if err := ghJSON(ctx, &response, args...); err != nil {
			return false, err
		}
		facts := response.Data.Repository.Issue
		if len(response.Errors) != 0 || facts == nil {
			return false, fmt.Errorf("invalid Ready facts response: %v", response.Errors)
		}
		if facts.Parent != nil || facts.SubIssues.TotalCount != 0 || facts.BlockedBy.PageInfo.HasNextPage {
			return false, nil
		}
		for _, blocker := range facts.BlockedBy.Nodes {
			if blocker.State != "CLOSED" {
				return false, nil
			}
		}
		for _, shaped := range facts.Shaped.Nodes {
			if shaped.CreatedAt.IsZero() {
				return false, fmt.Errorf("shaping event has no createdAt")
			}
			if time.Since(shaped.CreatedAt) < 30*time.Minute {
				return false, nil
			}
		}
		for _, edit := range facts.Edits.Nodes {
			if edit.EditedAt.IsZero() {
				return false, fmt.Errorf("body edit has no editedAt")
			}
			if !edit.EditedAt.After(readyAt) {
				continue
			}
			actor := ""
			if edit.Editor != nil {
				actor = edit.Editor.Login
			}
			ok, err := trustedActor(ctx, issue.repo, actor)
			if err != nil || !ok {
				return false, err
			}
		}
		page := facts.Edits.PageInfo
		if !page.HasNextPage {
			return true, nil
		}
		if page.EndCursor == "" || page.EndCursor == after {
			return false, fmt.Errorf("body edit history pagination did not advance")
		}
		after = page.EndCursor
	}
}
