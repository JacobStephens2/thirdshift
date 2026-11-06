package main_test

import (
	"debug/elf"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"
)

var binary string

func TestMain(m *testing.M) {
	dir, err := os.MkdirTemp("", "thirdshift-go-test-")
	if err != nil {
		panic(err)
	}
	binary = filepath.Join(dir, "thirdshift")
	cmd := exec.Command("go", "build", "-o", binary, ".")
	cmd.Env = append(os.Environ(), "CGO_ENABLED=0")
	if output, err := cmd.CombinedOutput(); err != nil {
		panic(string(output) + err.Error())
	}
	code := m.Run()
	os.RemoveAll(dir)
	os.Exit(code)
}

type fixture struct {
	t                    *testing.T
	root, repo, worktree string
	env                  []string
}

func newFixture(t *testing.T) *fixture {
	t.Helper()
	root := t.TempDir()
	f := &fixture{t: t, root: root, repo: filepath.Join(root, "repo"), worktree: filepath.Join(root, "repo-issue-7")}
	tools := filepath.Join(root, "tools")
	if err := os.MkdirAll(tools, 0755); err != nil {
		t.Fatal(err)
	}
	f.env = append(os.Environ(), "PATH="+tools+":"+os.Getenv("PATH"), "FIXTURE="+root,
		"GIT_CONFIG_GLOBAL=/dev/null", "GIT_CONFIG_NOSYSTEM=1", "GIT_TERMINAL_PROMPT=0",
		"GIT_AUTHOR_NAME=Test", "GIT_AUTHOR_EMAIL=test@example.com", "GIT_COMMITTER_NAME=Test", "GIT_COMMITTER_EMAIL=test@example.com")
	for _, name := range []string{"gh", "claude"} {
		data, err := os.ReadFile(filepath.Join("testdata", name+".py"))
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(tools, name), data, 0755); err != nil {
			t.Fatal(err)
		}
	}
	f.git(root, "init", "--bare", "origin.git")
	f.git(root, "init", "-b", "main", "repo")
	f.git(f.repo, "-c", "core.hooksPath=/dev/null", "commit", "--allow-empty", "-m", "base")
	f.git(f.repo, "remote", "add", "origin", "https://github.com/acme/widgets.git")
	f.git(f.repo, "config", "url."+filepath.Join(root, "origin.git")+".insteadOf", "https://github.com/acme/widgets.git")
	f.git(f.repo, "push", "origin", "main")
	f.writeState("open", "OWNER", "success")
	return f
}

func (f *fixture) git(dir string, args ...string) string {
	f.t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir, cmd.Env = dir, f.env
	output, err := cmd.CombinedOutput()
	if err != nil {
		f.t.Fatalf("git %v: %s: %v", args, output, err)
	}
	return strings.TrimSpace(string(output))
}

func (f *fixture) writeState(state, association, mode string) {
	f.t.Helper()
	data, err := json.Marshal(map[string]any{
		"state": state, "author_association": association, "title": "Add widgets", "body": "Make widgets.",
		"labels": []map[string]string{{"name": "ready-for-agent"}, {"name": "enhancement"}},
		"mode":   mode,
	})
	if err != nil {
		f.t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(f.root, "state.json"), data, 0600); err != nil {
		f.t.Fatal(err)
	}
}

func (f *fixture) run(args ...string) (string, error) {
	cmd := exec.Command(binary, args...)
	cmd.Dir, cmd.Env = f.repo, f.env
	output, err := cmd.CombinedOutput()
	return string(output), err
}

func TestPreflightRejectsOriginMismatch(t *testing.T) {
	f := newFixture(t)
	output, err := f.run("https://github.com/other/repo/issues/7")
	if err == nil || !strings.Contains(output, "Origin match") {
		t.Fatalf("wanted Origin match failure, got %v: %s", err, output)
	}
	if _, err := os.Stat(filepath.Join(f.root, "calls.jsonl")); !os.IsNotExist(err) {
		t.Fatal("origin mismatch contacted GitHub")
	}
}

func TestPreflightRejectsClosedOrUntrustedIssue(t *testing.T) {
	for _, tc := range []struct{ state, association, message string }{
		{"closed", "OWNER", "open issue"}, {"open", "NONE", "Trusted account"},
	} {
		t.Run(tc.message, func(t *testing.T) {
			f := newFixture(t)
			f.writeState(tc.state, tc.association, "success")
			output, err := f.run("https://github.com/acme/widgets/issues/7")
			if err == nil || !strings.Contains(output, tc.message) {
				t.Fatalf("wanted %s failure, got %v: %s", tc.message, err, output)
			}
			calls, _ := os.ReadFile(filepath.Join(f.root, "calls.jsonl"))
			if strings.Contains(string(calls), `"edit"`) {
				t.Fatal("pre-flight failure made a Claim")
			}
			if _, err := os.Stat(f.worktree); !os.IsNotExist(err) {
				t.Fatal("pre-flight failure made a Worktree")
			}
		})
	}
}

func TestTicketRunLinksEmbeddedSkillsAndStreamsSession(t *testing.T) {
	f := newFixture(t)
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err != nil {
		t.Fatalf("Ticket run: %v: %s", err, output)
	}
	if !strings.Contains(output, `"session_id":"fixture"`) || !strings.Contains(output, `"result":"done"`) {
		t.Fatalf("missing streaming events: %s", output)
	}
	if got := f.git(f.worktree, "branch", "--show-current"); got != "issue-7" {
		t.Fatalf("Issue branch = %s", got)
	}
	if got := f.git(f.worktree, "status", "--porcelain", "--untracked-files=all"); got != "" {
		t.Fatalf("Factory skills and issue leaked into git status: %s", got)
	}
	data, err := os.ReadFile(filepath.Join(f.worktree, ".claude/skills/thirdshift-tdd/tests.md"))
	if err != nil || !strings.Contains(string(data), "Good and Bad Tests") {
		t.Fatalf("embedded supporting skill file: %v", err)
	}
	calls, _ := os.ReadFile(filepath.Join(f.root, "calls.jsonl"))
	if !strings.Contains(string(calls), `"--add-label", "in-progress"`) || !strings.Contains(string(calls), `"--remove-label", "ready-for-agent"`) {
		t.Fatalf("missing Claim transition: %s", calls)
	}
	f.assertLabels("enhancement", "ready-for-agent")
}

func (f *fixture) assertLabels(want ...string) {
	f.t.Helper()
	data, err := os.ReadFile(filepath.Join(f.root, "state.json"))
	if err != nil {
		f.t.Fatal(err)
	}
	var state struct {
		Labels []struct {
			Name string `json:"name"`
		} `json:"labels"`
	}
	if err := json.Unmarshal(data, &state); err != nil {
		f.t.Fatal(err)
	}
	labels := make(map[string]bool)
	for _, label := range state.Labels {
		labels[label.Name] = true
	}
	if len(labels) != len(want) {
		f.t.Fatalf("labels = %v, want %v", labels, want)
	}
	for _, name := range want {
		if !labels[name] {
			f.t.Fatalf("missing label %s: %v", name, labels)
		}
	}
}

func TestBinaryIsStaticLinux(t *testing.T) {
	file, err := elf.Open(binary)
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	for _, program := range file.Progs {
		if program.Type == elf.PT_INTERP {
			t.Fatal("binary needs a dynamic loader")
		}
	}
	libs, err := file.ImportedLibraries()
	if err != nil || len(libs) != 0 {
		t.Fatalf("dynamic libraries: %v, %v", libs, err)
	}
}

func waitFor(t *testing.T, description string, check func() bool) {
	t.Helper()
	deadline := time.Now().Add(20 * time.Second)
	for time.Now().Before(deadline) {
		if check() {
			return
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", description)
}

func (f *fixture) start(mode string) (*exec.Cmd, <-chan error) {
	f.t.Helper()
	f.writeState("open", "OWNER", mode)
	cmd := exec.Command(binary, "https://github.com/acme/widgets/issues/7")
	cmd.Dir, cmd.Env = f.repo, f.env
	output, err := os.Create(filepath.Join(f.root, "output"))
	if err != nil {
		f.t.Fatal(err)
	}
	cmd.Stdout, cmd.Stderr = output, output
	if err := cmd.Start(); err != nil {
		f.t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() { err := cmd.Wait(); output.Close(); done <- err }()
	f.t.Cleanup(func() {
		// Only stop processes identified by this fixture's own task and PID files.
		for _, name := range []string{"parent-ready", "child-ready"} {
			data, _ := os.ReadFile(filepath.Join(f.root, name))
			var pid int
			if _, err := fmt.Sscanf(string(data), "%d", &pid); err == nil && pid > 0 {
				syscall.Kill(pid, syscall.SIGKILL)
			}
		}
		cmd.Process.Kill()
	})
	return cmd, done
}

func (f *fixture) output() string {
	data, _ := os.ReadFile(filepath.Join(f.root, "output"))
	return string(data)
}

func TestHarnessOutputStreamsBeforeExit(t *testing.T) {
	f := newFixture(t)
	_, done := f.start("stream")
	waitFor(t, "streamed init before Harness exit", func() bool { return strings.Contains(f.output(), `"session_id":"fixture"`) })
	select {
	case err := <-done:
		t.Fatalf("Harness exited before release: %v: %s", err, f.output())
	default:
	}
	if err := os.WriteFile(filepath.Join(f.root, "finish"), nil, 0600); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("session: %v: %s", err, f.output())
		}
	case <-time.After(20 * time.Second):
		t.Fatal("session did not finish")
	}
}

func TestHarnessProtocolFailuresReleaseClaim(t *testing.T) {
	for _, mode := range []string{"malformed", "no-result", "error-result"} {
		t.Run(mode, func(t *testing.T) {
			f := newFixture(t)
			f.writeState("open", "OWNER", mode)
			output, err := f.run("https://github.com/acme/widgets/issues/7")
			if err == nil || !strings.Contains(output, "Harness") {
				t.Fatalf("wanted protocol failure: %v: %s", err, output)
			}
			f.assertLabels("enhancement", "ready-for-agent")
		})
	}
}

func TestInterruptTerminatesHarnessProcessGroupAndEndsClaim(t *testing.T) {
	for _, mode := range []string{"stubborn", "leader-exits"} {
		t.Run(mode, func(t *testing.T) {
			f := newFixture(t)
			cmd, done := f.start(mode)
			for _, name := range []string{"parent-ready", "child-ready"} {
				waitFor(t, name, func() bool { data, _ := os.ReadFile(filepath.Join(f.root, name)); return len(data) > 0 })
			}
			if err := cmd.Process.Signal(os.Interrupt); err != nil {
				t.Fatal(err)
			}
			select {
			case err := <-done:
				if exit, ok := err.(*exec.ExitError); !ok || exit.ExitCode() != 130 {
					t.Fatalf("interrupt outcome: %v: %s", err, f.output())
				}
			case <-time.After(20 * time.Second):
				t.Fatal("interruption did not finish")
			}
			signals, _ := os.ReadFile(filepath.Join(f.root, "signals"))
			for _, role := range []string{"parent", "child"} {
				if !strings.Contains(string(signals), role+":2\n") {
					t.Fatalf("%s missed SIGINT: %s", role, signals)
				}
				if mode == "stubborn" && !strings.Contains(string(signals), role+":15\n") {
					t.Fatalf("%s missed SIGTERM: %s", role, signals)
				}
				pid, _ := os.ReadFile(filepath.Join(f.root, role+"-ready"))
				waitFor(t, role+" termination", func() bool {
					stat, err := os.ReadFile("/proc/" + string(pid) + "/stat")
					return os.IsNotExist(err) || strings.Contains(string(stat), ") Z ")
				})
			}
			f.assertLabels("enhancement", "ready-for-agent")
		})
	}
}

func TestInterruptedClaimMutationStillEndsClaim(t *testing.T) {
	f := newFixture(t)
	f.writeState("open", "OWNER", "interrupt-claim")
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if exit, ok := err.(*exec.ExitError); !ok || exit.ExitCode() != 130 {
		t.Fatalf("interrupt outcome: %v: %s", err, output)
	}
	f.assertLabels("enhancement", "ready-for-agent")
}

func TestClaimEndingReflectsOriginAndIssueState(t *testing.T) {
	for _, tc := range []struct {
		mode   string
		fails  bool
		labels []string
	}{
		{"push", false, []string{"enhancement", "in-progress"}},
		{"push-fail", true, []string{"enhancement", "in-progress", "agent-failed"}},
		{"close", false, []string{"enhancement"}},
	} {
		t.Run(tc.mode, func(t *testing.T) {
			f := newFixture(t)
			f.writeState("open", "OWNER", tc.mode)
			output, err := f.run("https://github.com/acme/widgets/issues/7")
			if (err != nil) != tc.fails {
				t.Fatalf("session outcome: %v: %s", err, output)
			}
			f.assertLabels(tc.labels...)
		})
	}
}

func TestWorktreeFailureReleasesClaimWithoutLaunchingHarness(t *testing.T) {
	f := newFixture(t)
	f.git(f.repo, "branch", "issue-7")
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err == nil || !strings.Contains(output, "already exists") {
		t.Fatalf("expected branch collision: %v: %s", err, output)
	}
	if strings.Contains(output, `"session_id"`) {
		t.Fatal("started Harness after Worktree failure")
	}
	f.assertLabels("enhancement", "ready-for-agent")
}

func TestIssueFileSymlinkIsNotFollowed(t *testing.T) {
	f := newFixture(t)
	canary := filepath.Join(f.root, "operator-secret.txt")
	const original = "operator secret\n"
	if err := os.WriteFile(canary, []byte(original), 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(canary, filepath.Join(f.repo, ".thirdshift-issue.md")); err != nil {
		t.Fatal(err)
	}
	f.git(f.repo, "add", ".thirdshift-issue.md")
	f.git(f.repo, "commit", "-m", "plant issue symlink")
	f.git(f.repo, "push", "origin", "main")

	output, err := f.run("https://github.com/acme/widgets/issues/7")
	got, readErr := os.ReadFile(canary)
	if readErr != nil {
		t.Fatal(readErr)
	}
	if string(got) != original {
		t.Fatalf("issue write followed symlink outside the worktree: %q\nrun err=%v output=%s", got, err, output)
	}
	info, statErr := os.Lstat(filepath.Join(f.worktree, ".thirdshift-issue.md"))
	if statErr != nil || info.Mode()&os.ModeSymlink == 0 {
		t.Fatalf("worktree issue path = %v, %v; want the planted symlink", info, statErr)
	}
	if err == nil || !strings.Contains(output, "symlink") {
		t.Fatalf("wanted symlink refusal, got %v: %s", err, output)
	}
	if strings.Contains(output, `"session_id"`) {
		t.Fatal("started Harness after refusing the symlink")
	}
	f.assertLabels("enhancement", "ready-for-agent")
}

func TestReviewClaimReleaseRestoresOriginalLabels(t *testing.T) {
	f := newFixture(t)
	path := filepath.Join(f.root, "state.json")
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var state map[string]any
	if err := json.Unmarshal(raw, &state); err != nil {
		t.Fatal(err)
	}
	state["labels"] = []map[string]string{{"name": "enhancement"}}
	raw, err = json.Marshal(state)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, raw, 0600); err != nil {
		t.Fatal(err)
	}
	output, err := f.run("https://github.com/acme/widgets/issues/7")
	if err != nil {
		t.Fatalf("Ticket run: %v: %s", err, output)
	}
	f.assertLabels("enhancement")
}
