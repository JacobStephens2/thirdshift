package main

import (
	"context"
	"errors"
	"fmt"
)

func (d *delivery) salvage(ctx context.Context) error {
	if merge, _ := command(ctx, d.worktree, "git", "rev-parse", "-q", "--verify", "MERGE_HEAD"); len(merge) > 0 {
		if _, err := command(ctx, d.worktree, "git", "merge", "--abort"); err != nil {
			return err
		}
	}
	// Draft before attempting a push: a rejected push must not leave a failed
	// Ticket run's existing pull request looking ready for review.
	if err := d.findPR(ctx); err != nil {
		return err
	}
	var draftErr error
	if d.prNumber != 0 {
		draftErr = d.draft(ctx)
	}
	ahead, err := command(ctx, d.worktree, "git", "rev-list", "--count", d.base+"..HEAD")
	if err != nil {
		return errors.Join(draftErr, err)
	}
	if string(ahead) != "0" {
		if _, err := d.publish(ctx); err != nil {
			return errors.Join(draftErr, fmt.Errorf("Failed ticket run salvage: %w", err))
		}
	}
	if d.prNumber != 0 {
		return errors.Join(draftErr, d.draft(ctx))
	}
	return draftErr
}
