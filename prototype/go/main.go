package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"syscall"
)

func main() { os.Exit(entry()) }

func entry() int {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	flags := flag.NewFlagSet("thirdshift", flag.ContinueOnError)
	repairLimit := flags.Int("repair-limit", 5, "maximum Repair sessions (0 to 5)")
	if err := flags.Parse(os.Args[1:]); err != nil {
		return 2
	}
	if flags.NArg() != 1 || *repairLimit < 0 || *repairLimit > 5 {
		fmt.Fprintln(os.Stderr, "usage: thirdshift [--repair-limit 0..5] <https://github.com/owner/repo/issues/number>")
		return 2
	}
	err := ticketRun(ctx, flags.Arg(0), *repairLimit)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
	}
	switch outcome(ctx.Err(), err) {
	case runSucceeded:
		return 0
	case runFailed:
		return 1
	case runInterrupted:
		return 130
	}
	panic("invalid Run outcome")
}
