package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"syscall"
)

func main() { os.Exit(entry()) }

func entry() int {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: thirdshift <https://github.com/owner/repo/issues/number>")
		return 2
	}
	err := ticketRun(ctx, os.Args[1])
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
	}
	if ctx.Err() != nil {
		return 130
	}
	if err != nil {
		return 1
	}
	return 0
}
