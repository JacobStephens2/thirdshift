package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
)

func harnessSession(ctx context.Context, worktree, prompt string) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	reader, writer, err := os.Pipe()
	if err != nil {
		return err
	}
	defer reader.Close()
	cmd := exec.Command("claude", "-p", "--permission-mode", "auto", "--output-format", "stream-json", "--verbose", prompt)
	cmd.Dir, cmd.Stdout, cmd.Stderr = worktree, writer, os.Stderr
	// A nil Stdin connects the Harness to the null device.
	done := make(chan error, 1)
	go func() {
		err := supervise(ctx, cmd)
		writer.Close()
		done <- err
	}()
	decoder, encoder := json.NewDecoder(reader), json.NewEncoder(os.Stdout)
	var streamErr error
	completed := false
	for {
		var event map[string]json.RawMessage
		err := decoder.Decode(&event)
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			streamErr = fmt.Errorf("Harness streaming JSON: %w", err)
			break
		}
		var kind string
		if err := json.Unmarshal(event["type"], &kind); err != nil || kind == "" {
			streamErr = fmt.Errorf("Harness event has no type")
			break
		}
		if err := encoder.Encode(event); err != nil {
			streamErr = err
			break
		}
		if kind == "result" {
			var failed *bool
			if err := json.Unmarshal(event["is_error"], &failed); err != nil || failed == nil {
				streamErr = fmt.Errorf("Harness result has no is_error")
				break
			}
			if *failed {
				streamErr = fmt.Errorf("Harness reported an error result")
				break
			}
			completed = true
		}
	}
	if streamErr != nil {
		cancel()
	}
	processErr := <-done
	if streamErr == nil && processErr == nil && !completed {
		streamErr = fmt.Errorf("Harness ended without a result event")
	}
	return errors.Join(streamErr, processErr)
}
