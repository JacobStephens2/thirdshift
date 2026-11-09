package main_test

import (
	"strings"
	"testing"
)

func TestSpecStaleClaimMustNotReleaseSomeoneElsesClaim(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "reservation")
	f.pickupConfig(map[string]any{"poison_label": "in-progress"})
	output, err := f.run("pickup")
	if err != nil || !strings.Contains(output, "Pickup skipped") {
		t.Fatalf("wanted stale selection skip: %v: %s", err, output)
	}
	f.assertNoSession()
	f.assertLabels("enhancement", "ready-for-agent", "in-progress")
}
