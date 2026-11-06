package main_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Each model Claim transition must correspond to one atomic remote replacement.
// gh issue edit --add-label/--remove-label starts independent mutations.
func TestPickupClaimRequestsMatchModelAtomicTransitions(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "reservation")
	output, err := f.run("pickup")
	if err != nil {
		t.Fatalf("Pickup run: %v: %s", err, output)
	}
	f.assertLabels("enhancement", "ready-for-agent")
	raw, err := os.ReadFile(filepath.Join(f.root, "calls.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	puts := 0
	for _, line := range strings.Split(strings.TrimSpace(string(raw)), "\n") {
		var args []string
		if err := json.Unmarshal([]byte(line), &args); err != nil {
			t.Fatal(err)
		}
		if len(args) >= 2 && args[0] == "issue" && args[1] == "edit" {
			t.Errorf("Claim transition uses gh issue edit, whose label additions/removals are separate concurrent GraphQL mutations: %v", args)
		}
		if len(args) < 2 || args[0] != "api" {
			continue
		}
		endpoint, method := false, ""
		for i, arg := range args {
			if arg == "repos/acme/widgets/issues/7/labels" {
				endpoint = true
			}
			if (arg == "--method" || arg == "-X") && i+1 < len(args) {
				method = args[i+1]
			}
			if strings.HasPrefix(arg, "--method=") {
				method = strings.TrimPrefix(arg, "--method=")
			}
		}
		if endpoint {
			if method != "PUT" {
				t.Errorf("labels endpoint needs atomic full replacement via PUT, got %v", args)
			}
			puts++
		}
	}
	if puts != 2 {
		t.Errorf("want one label PUT for Claim acquisition and one for ending, got %d", puts)
	}
}
