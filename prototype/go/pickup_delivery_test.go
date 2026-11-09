package main_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPickupDeliversAndRepairsReservedTicket(t *testing.T) {
	for _, tc := range []struct {
		mode, widget string
		args         []string
		draft        bool
		sessions     int
	}{
		{"delivery", "broken", []string{"pickup"}, false, 1},
		{"ci-repair", "repaired", []string{"--repair-limit", "1", "pickup"}, false, 2},
		{"conflict-repair", "resolved", []string{"pickup"}, false, 2},
		{"salvage", "broken", []string{"pickup"}, true, 1},
		{"ci-still-red", "attempt 1", []string{"--repair-limit", "0", "pickup"}, true, 1},
	} {
		t.Run(tc.mode, func(t *testing.T) {
			f := newFixture(t)
			f.env = append(f.env, "EXPECT_PICKUP_RESERVATION=1")
			f.writeState("open", "OWNER", tc.mode)
			if tc.mode == "conflict-repair" {
				if err := os.WriteFile(filepath.Join(f.repo, "widget.txt"), []byte("original\n"), 0600); err != nil {
					t.Fatal(err)
				}
				f.git(f.repo, "add", "widget.txt")
				f.git(f.repo, "commit", "-m", "Original widget")
				f.git(f.repo, "push", "origin", "main")
			}
			output, err := f.run(tc.args...)
			if (err != nil) != tc.draft {
				t.Fatalf("Pickup: %v: %s", err, output)
			}
			if tc.mode == "ci-still-red" && !strings.Contains(output, "Repair limit reached (0)") {
				t.Fatalf("Pickup ignored the Repair limit: %s", output)
			}
			f.assertPR(tc.draft)
			labels := []string{"enhancement", "in-progress"}
			if tc.draft {
				labels = append(labels, "agent-failed")
			}
			f.assertLabels(labels...)
			if got := f.git(f.repo, "show", "origin/issue-7:widget.txt"); got != tc.widget {
				t.Fatalf("delivered widget = %q, want %q", got, tc.widget)
			}
			sessions, err := os.ReadFile(filepath.Join(f.root, "sessions.jsonl"))
			if err != nil {
				t.Fatal(err)
			}
			if got := len(strings.Split(strings.TrimSpace(string(sessions)), "\n")); got != tc.sessions {
				t.Fatalf("Harness sessions = %d, want %d", got, tc.sessions)
			}
			if got := f.git(f.repo, "ls-remote", "origin", "refs/heads/thirdshift/pickup/*"); got != "" {
				t.Fatalf("completed Ticket run retained reservation: %s", got)
			}
		})
	}
}
