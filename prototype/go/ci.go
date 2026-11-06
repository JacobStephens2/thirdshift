package main

import (
	"context"
	"fmt"
)

type ciObservation struct {
	state  ciState
	failed []string
}

// Read checks and commit statuses for the full published SHA, never the latest
// workflow on a branch. Only the latest status for each context counts.
func readCI(ctx context.Context, repo, head string) (ciObservation, error) {
	var pages []struct {
		CheckRuns []struct{ Name, Status, Conclusion string } `json:"check_runs"`
	}
	if err := ghJSON(ctx, &pages, "api", "repos/"+repo+"/commits/"+head+"/check-runs?filter=latest&per_page=100", "--paginate", "--slurp"); err != nil {
		return ciObservation{}, err
	}
	result := ciObservation{state: ciAbsent}
	pending, count := false, 0
	for _, page := range pages {
		for _, check := range page.CheckRuns {
			count++
			if check.Status != "completed" {
				switch check.Status {
				case "queued", "in_progress", "waiting", "requested", "pending":
					pending = true
				default:
					return result, fmt.Errorf("unknown CI check status %q", check.Status)
				}
				continue
			}
			switch check.Conclusion {
			case "success", "neutral", "skipped":
			case "failure", "timed_out", "cancelled", "action_required", "stale", "startup_failure":
				result.failed = append(result.failed, check.Name)
			default:
				return result, fmt.Errorf("unknown CI conclusion %q", check.Conclusion)
			}
		}
	}
	var statuses [][]struct{ Context, State string }
	if err := ghJSON(ctx, &statuses, "api", "repos/"+repo+"/commits/"+head+"/statuses?per_page=100", "--paginate", "--slurp"); err != nil {
		return result, err
	}
	seen := map[string]bool{}
	for _, page := range statuses {
		for _, status := range page {
			if seen[status.Context] {
				continue
			}
			seen[status.Context] = true
			count++
			switch status.State {
			case "success":
			case "pending", "expected":
				pending = true
			case "failure", "error":
				result.failed = append(result.failed, status.Context)
			default:
				return result, fmt.Errorf("unknown commit status %q", status.State)
			}
		}
	}
	open, err := workflowRuns(ctx, repo, head, &result)
	if err != nil {
		return result, err
	}
	// An open run may not have created its jobs yet. Successes already listed
	// must not hide it. A failure already in hand can start a Repair.
	if open && len(result.failed) == 0 {
		pending = true
	}
	switch {
	case pending:
		result.state = ciPending
	case len(result.failed) > 0:
		result.state = ciFailed
	case count > 0:
		result.state = ciPassed
	}
	return result, nil
}

// Latest workflow run of each workflow on this SHA. Older runs are superseded.
// A completed failure is red even when none of its jobs appeared as check runs.
func workflowRuns(ctx context.Context, repo, head string, result *ciObservation) (bool, error) {
	var pages []struct {
		WorkflowRuns []struct {
			ID         int64   `json:"id"`
			Name       string  `json:"name"`
			Status     string  `json:"status"`
			Conclusion *string `json:"conclusion"`
			CreatedAt  string  `json:"created_at"`
			WorkflowID int64   `json:"workflow_id"`
		} `json:"workflow_runs"`
	}
	if err := ghJSON(ctx, &pages, "api", "repos/"+repo+"/actions/runs?head_sha="+head+"&per_page=100", "--paginate", "--slurp"); err != nil {
		return false, err
	}
	type latestRun struct {
		id         int64
		name       string
		status     string
		conclusion *string
		created    string
	}
	latest := map[int64]latestRun{}
	for _, page := range pages {
		for _, run := range page.WorkflowRuns {
			if run.WorkflowID == 0 || run.ID == 0 || run.CreatedAt == "" || run.Status == "" {
				return false, fmt.Errorf("unreadable workflow run")
			}
			prev, ok := latest[run.WorkflowID]
			if ok && (prev.created > run.CreatedAt || (prev.created == run.CreatedAt && prev.id >= run.ID)) {
				continue
			}
			latest[run.WorkflowID] = latestRun{run.ID, run.Name, run.Status, run.Conclusion, run.CreatedAt}
		}
	}
	open := false
	for _, run := range latest {
		switch run.status {
		case "queued", "in_progress", "waiting", "requested", "pending":
			open = true
		case "completed":
			if run.conclusion == nil {
				return false, fmt.Errorf("workflow run %d completed without a conclusion", run.id)
			}
			switch *run.conclusion {
			case "success", "neutral", "skipped":
			case "failure", "timed_out", "cancelled", "action_required", "stale", "startup_failure":
				name := run.name
				if name == "" {
					name = fmt.Sprintf("workflow %d", run.id)
				}
				result.failed = append(result.failed, name)
			default:
				return false, fmt.Errorf("unknown workflow conclusion %q", *run.conclusion)
			}
		default:
			return false, fmt.Errorf("unknown workflow status %q", run.status)
		}
	}
	return open, nil
}
