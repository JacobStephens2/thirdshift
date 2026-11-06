package main

import (
	"context"
	"errors"
	"fmt"
	"sort"
	"strconv"
)

func pickupRun(ctx context.Context, repairLimit int) error {
	launch, origin, err := launchOrigin(ctx)
	if err != nil {
		return err
	}
	repo := originRepo(origin)
	if repo == "" {
		return fmt.Errorf("Pickup requires a GitHub origin")
	}
	var pages [][]struct{ Number int }
	if err := ghJSON(ctx, &pages, "api", "repos/"+repo+"/issues?state=open&labels=ready-for-agent&per_page=100", "--paginate", "--slurp"); err != nil {
		return err
	}
	var numbers []int
	for _, page := range pages {
		for _, item := range page {
			if item.Number <= 0 {
				return fmt.Errorf("invalid candidate issue number %d", item.Number)
			}
			numbers = append(numbers, item.Number)
		}
	}
	sort.Ints(numbers)
	for _, number := range numbers {
		issue := issueURL{"https://github.com/" + repo + "/issues/" + strconv.Itoa(number), repo, strconv.Itoa(number)}
		_, generation, ready, err := readReady(ctx, launch, issue)
		if err != nil {
			return err
		}
		if !ready {
			continue
		}
		r, err := reserve(ctx, launch, issue)
		if err != nil {
			return err
		}
		if r == nil {
			continue
		}
		dispatched, err := pickupReserved(ctx, launch, issue, generation, r, repairLimit)
		if err != nil || dispatched {
			return err
		}
	}
	fmt.Println("Pickup skipped: no Ready issue available")
	return nil
}

func pickupReserved(ctx context.Context, launch string, issue issueURL, generation int, r *reservation, repairLimit int) (dispatched bool, runErr error) {
	defer func() { runErr = errors.Join(runErr, r.release()) }()
	data, current, ready, err := readReady(ctx, launch, issue)
	if err != nil || !ready || current != generation {
		return false, err
	}
	err = runTicket(ctx, launch, issue, data, repairLimit)
	if errors.Is(err, errStaleClaim) {
		return false, nil
	}
	return true, err
}
