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
