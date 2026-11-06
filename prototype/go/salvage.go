package main

import (
	"context"
	"fmt"
)

func (d *delivery) salvage(ctx context.Context) error {
	if merge, _ := command(ctx, d.worktree, "git", "rev-parse", "-q", "--verify", "MERGE_HEAD"); len(merge) > 0 {
		if _, err := command(ctx, d.worktree, "git", "merge", "--abort"); err != nil {
			return err
		}
	}
	ahead, err := command(ctx, d.worktree, "git", "rev-list", "--count", d.base+"..HEAD")
	if err != nil {
		return err
	}
	if string(ahead) != "0" {
		if _, err := d.publish(ctx); err != nil {
			return fmt.Errorf("Failed ticket run salvage: %w", err)
		}
	} else if err := d.findPR(ctx); err != nil {
		return err
	}
	if d.prNumber != 0 {
		return d.draft(ctx)
	}
	return nil
}
