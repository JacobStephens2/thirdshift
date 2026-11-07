package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"os"
	"strings"
	"time"
)

type reservation struct{ launch, ref, token string }

func reserve(ctx context.Context, launch string, issue issueURL) (*reservation, error) {
	ref := "refs/heads/thirdshift/pickup/" + issue.number
	nonce := make([]byte, 32)
	if _, err := rand.Read(nonce); err != nil {
		return nil, err
	}
	tree, err := command(ctx, launch, "git", "mktree")
	if err != nil {
		return nil, err
	}
	host, err := os.Hostname()
	if err != nil {
		host = "unknown"
	}
	message := fmt.Sprintf("thirdshift Pickup reservation for %s\n\nToken: %s\nHost: %s\nLaunch: %s\nProcess: %d\nStarted: %s",
		issue.url, hex.EncodeToString(nonce), host, launch, os.Getpid(), time.Now().UTC().Format(time.RFC3339Nano))
	token, err := command(ctx, launch, "git", "-c", "commit.gpgSign=false", "commit-tree", string(tree), "-m", message)
	if err != nil {
		return nil, err
	}
	r := &reservation{launch, ref, string(token)}
	_, err = command(ctx, launch, "git", "push", "--no-verify", "--force-with-lease="+ref+":", "origin", r.token+":"+ref)
	if err == nil {
		return r, nil
	}
	// A lost response can mean our create succeeded. Only evidence of a
	// different owner permits moving on; uncertainty retains the ref.
	completion, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	remote, readErr := command(completion, launch, "git", "ls-remote", "--refs", "origin", ref)
	fields := strings.Fields(string(remote))
	if readErr == nil && len(fields) == 2 && fields[1] == ref && fields[0] != r.token {
		return nil, nil
	}
	return nil, fmt.Errorf("could not reserve %s; check origin by hand (our token %s; remote read: %v): %w", ref, r.token, readErr, err)
}

func (r *reservation) release() error {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	_, err := command(ctx, r.launch, "git", "push", "--no-verify", "--force-with-lease="+r.ref+":"+r.token, "origin", ":"+r.ref)
	if err != nil {
		return fmt.Errorf("could not release %s; check origin by hand (our token %s): %w", r.ref, r.token, err)
	}
	return nil
}
