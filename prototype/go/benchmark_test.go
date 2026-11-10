package main_test

import (
	"os/exec"
	"testing"
)

func TestBenchmarkRunner(t *testing.T) {
	cmd := exec.Command("python3", "-m", "unittest", "discover", "-p", "benchmark_test.py", "-v")
	if output, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("benchmark contract: %v\n%s", err, output)
	}
}
