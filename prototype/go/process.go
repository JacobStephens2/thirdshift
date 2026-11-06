package main

import (
	"bytes"
	"context"
	"fmt"
	"os/exec"
	"syscall"
	"time"
)

// Every child owns a process group. Completion uses a fresh context so a
// recorded interruption never prevents Claim ending.
const stopGrace = 2 * time.Second

func supervise(ctx context.Context, cmd *exec.Cmd) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	cmd.WaitDelay = stopGrace
	if err := cmd.Start(); err != nil {
		return err
	}
	pid := cmd.Process.Pid
	defer syscall.Kill(-pid, syscall.SIGKILL)
	done := make(chan error, 1)
	go func() { done <- cmd.Wait() }()
	select {
	case err := <-done:
		return err
	case <-ctx.Done():
		for _, sig := range []syscall.Signal{syscall.SIGINT, syscall.SIGTERM} {
			_ = syscall.Kill(-pid, sig)
			timer := time.NewTimer(stopGrace)
			select {
			case <-done:
				timer.Stop()
				return ctx.Err()
			case <-timer.C:
			}
		}
		_ = syscall.Kill(-pid, syscall.SIGKILL)
		<-done
		return ctx.Err()
	}
}

func command(ctx context.Context, dir, name string, args ...string) ([]byte, error) {
	cmd := exec.Command(name, args...)
	cmd.Dir = dir
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	if err := supervise(ctx, cmd); err != nil {
		return nil, fmt.Errorf("%s %v: %w: %s", name, args, err, stderr.String())
	}
	return bytes.TrimSpace(stdout.Bytes()), nil
}
