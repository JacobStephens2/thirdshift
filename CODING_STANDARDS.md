Follow principles and ideas in the .agents/skills/codebase-design skill.

## Deadlines in tests

CI runners are shared and often loaded; a macOS runner can take over a second just to start a process. A test that needs work to finish inside a short deadline fails there now and then, and only on main, where the full suite runs.

- A deadline a test gives the code under test, such as a `--timeout` flag, bounds everything run under it, not only the work meant to time out. List what else must finish inside it: process start, sessions, Graders, checks, output the test reads afterwards.
- When a test needs something to time out, make that thing hang, looping or sleeping well past the deadline, and give the deadline several seconds of room for everything meant to finish. A deadline of about one second is too short when anything that must finish runs under it.
- When a change makes an existing deadline cover more work, check the tests that set that deadline.
- To wait for an event, poll for its file or state with a generous upper bound instead of sleeping a fixed time.
