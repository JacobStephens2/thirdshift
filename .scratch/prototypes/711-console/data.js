// PROTOTYPE (#711, Q27): made-up Console data. Nothing here is copied from a real repository or log.
// One machine's Commands, live first. Times are minutes before page load, so ages tick from there.

export const NOW0 = Date.now();
const ago = (min) => NOW0 - min * 60_000;

export const host = { name: "thirdshift", other: { name: "stephens", url: "#stephens-console" } };

// The order a Run's stages come in. Repair and Resume are detours, drawn beside the line rather than on it.
export const RUN_STAGES = ["implement", "review", "Delivery", "CI watch", "Self-merge", "cleanup"];
export const SPEC_STAGES = ["Tickets", "spec-review", "Delivery", "CI watch", "Self-merge", "cleanup"];
export const PASS_STAGES = {
  "Architect run": ["architecture-review"],
  "Security run": ["security-audit", "security-reproduction", "security-fix-publishing"],
  "Pickup run": [],
};

const codex = { harness: "codex", model: "gpt-6.1-sol", effort: "high" };
const claude = { harness: "claude", model: "claude-opus-5-5", effort: "xhigh" };

// kind: Run | Spec run | Architect run | Pickup run | Security run
// alive: the process is still there. outcome: null while live; ready | merged | failed | interrupted | skipped.
// A Command with alive=false and outcome=null "died at <stage>".
export const commands = [
  {
    id: "c1", kind: "Pickup run", repo: "chart35", started: ago(134), alive: true, outcome: null, ...claude,
    stopTarget: "chart35#230",
    commandLog: "chart35/commands/pickup/230-20261018T072101-0400.log",
    note: "took Ready issue chart35#230",
    children: [
      {
        id: "c1s", kind: "Spec run", issue: "chart35#230", title: "Field notes keep working offline",
        specBranch: "spec-230", stage: "Tickets", stageSince: ago(131), started: ago(131), ...claude,
        tickets: [
          { id: "t238", issue: "chart35#238", title: "Keep a local journal of note edits", state: "merged",
            started: ago(131), ended: ago(71), pr: "chart35#243", ...claude, sessions: ["s238"] },
          { id: "t239", issue: "chart35#239", title: "Queue edits made while offline", state: "running",
            stage: "review", stageSince: ago(52), started: ago(70), ...claude, latest: "Bash: cargo test",
            sessions: ["s239"] },
          { id: "t240", issue: "chart35#240", title: "Replay queued edits on reconnect", state: "waiting",
            blockedBy: ["chart35#239"] },
        ],
      },
    ],
  },
  {
    id: "c2", kind: "Run", repo: "thirdshift", issue: "thirdshift#702", title: "Fold Repair budgets into the Delivery",
    started: ago(61), alive: true, outcome: null, stage: "CI watch", stageSince: ago(20), ...codex, merge: true,
    latest: "CI on 4f1c2a9: 3 of 8 checks still running", pr: "thirdshift#707", lastEvent: ago(14),
    stopTarget: "thirdshift#702",
    commandLog: "thirdshift/commands/run/702-20261018T083012-0400.log",
    sessions: ["s702", "s702sec"],
  },
  {
    id: "c3", kind: "Run", repo: "gardenkit", issue: "gardenkit#88", title: "Rename seed packets to seed lots",
    started: ago(188), alive: false, outcome: null, stage: "implement", stageSince: ago(186), ...codex,
    latest: "Edit src/lots.rs", diedAt: ago(171), lastEvent: ago(171),
    commandLog: "gardenkit/commands/run/88-20261018T052455-0400.log",
    sessions: ["s88"],
  },
  {
    id: "c4", kind: "Architect run", repo: "chart35", started: ago(4), ended: ago(4), alive: false, outcome: "skipped",
    cause: "a Pickup run on chart35 is still running", ...claude,
  },
  {
    id: "c5", kind: "Run", repo: "thirdshift", issue: "thirdshift#698", title: "Name the Harness in the Run notification",
    started: ago(372), ended: ago(300), alive: false, outcome: "merged", pr: "thirdshift#699", ...codex, merge: true,
    summary: { diffstat: "+38 −6 in 4 files", repairs: 1, repairFor: "CI red on test", resumes: 0, unaddressed: 0 },
    commandLog: "thirdshift/commands/run/698-20261018T014801-0400.log", sessions: ["s698"],
  },
  {
    id: "c6", kind: "Security run", repo: "thirdshift", started: ago(470), ended: ago(392), alive: false, outcome: "ready",
    ...claude, secure: true,
    commandLog: "thirdshift/commands/security/20261018T001044-0400.log",
    sessions: ["s6audit", "s6repro1", "s6repro2"],
    findings: [
      { sev: "high", title: "A Command log path can follow a symlink out of the logs folder", reproduced: true,
        fix: "thirdshift#704", fixOutcome: "ready", fixPr: "thirdshift#705" },
      { sev: "low", title: "The Session log viewer echoes raw terminal escapes", reproduced: false, waits: true },
    ],
    children: [
      { id: "c6r", kind: "Run", issue: "thirdshift#704", title: "Keep Command log paths inside the logs folder",
        outcome: "ready", pr: "thirdshift#705", started: ago(430), ended: ago(392), ...claude, sessions: ["s704"] },
    ],
  },
  {
    id: "c7", kind: "Run", repo: "chart35", issue: "chart35#226", title: "Show sync status in the toolbar",
    started: ago(1230), ended: ago(1164), alive: false, outcome: "ready", pr: "chart35#227", ...claude,
    summary: { diffstat: "+112 −15 in 7 files", repairs: 0, resumes: 0, unaddressed: 2 },
    commandLog: "chart35/commands/run/226-20261017T110212-0400.log", sessions: ["s226"],
  },
  {
    id: "c8", kind: "Run", repo: "thirdshift", issue: "thirdshift#690", title: "Trim the Activity log's repeated skip lines",
    started: ago(1500), ended: ago(1410), alive: false, outcome: "failed", stage: "CI watch", ...codex,
    summary: { diffstat: "+9 −31 in 2 files", repairs: 2, repairFor: "CI red on clippy", resumes: 0, unaddressed: 0 },
    cause: "Base fix thirdshift#691 failed: CI red on clippy, which also fails on main at 1a2b3c4",
    commandLog: "thirdshift/commands/run/690-20261017T064433-0400.log", sessions: ["s690", "s690r1"],
    children: [
      { id: "c8b", kind: "Base fix", issue: "thirdshift#691", title: "Fix clippy on main", outcome: "failed",
        stage: "CI watch", cause: "CI red on clippy and the Repair found nothing to fix on the branch",
        started: ago(1460), ended: ago(1412), ...codex, sessions: ["s691"] },
    ],
  },
  {
    id: "c9", kind: "Spec run", repo: "gardenkit", issue: "gardenkit#70", title: "Labels for every seed lot",
    started: ago(1900), ended: ago(1745), alive: false, outcome: "failed", specBranch: "spec-70", ...codex,
    cause: "Ticket gardenkit#72 failed; gardenkit#73 not started",
    commandLog: "gardenkit/commands/spec/70-20261017T000105-0400.log",
    tickets: [
      { id: "t71", issue: "gardenkit#71", title: "Print a label from a lot", state: "merged", pr: "gardenkit#74",
        started: ago(1898), ended: ago(1840), ...codex, sessions: ["s71"] },
      { id: "t72", issue: "gardenkit#72", title: "Batch labels for a whole bed", state: "failed",
        cause: "the implement session ended without a commit", started: ago(1838), ended: ago(1746), ...codex,
        sessions: ["s72", "s72resume"] },
      { id: "t73", issue: "gardenkit#73", title: "Label templates per season", state: "not started" },
    ],
  },
  {
    id: "c10", kind: "Run", repo: "chart35", issue: "chart35#219", title: "Export notes as CSV",
    started: ago(2950), ended: ago(2890), alive: false, outcome: "interrupted", stage: "Delivery", ...claude,
    summary: { diffstat: "+64 −3 in 3 files", repairs: 0, resumes: 1, unaddressed: 0 },
    cause: "stopped from the Console; chart35#219 labelled needs-triage",
    commandLog: "chart35/commands/run/219-20261016T082214-0400.log", sessions: ["s219"],
  },
  {
    id: "c11", kind: "Architect run", repo: "thirdshift", started: ago(3100), ended: ago(2990), alive: false,
    outcome: "merged", ...codex, note: "plan published: thirdshift#695",
    commandLog: "thirdshift/commands/architect/20261016T055003-0400.log", sessions: ["s11arch"],
    children: [
      { id: "c11r", kind: "Run", issue: "thirdshift#695", title: "Move the Claim helpers next to the Sweep",
        outcome: "merged", pr: "thirdshift#696", started: ago(3080), ended: ago(2990), ...codex, sessions: ["s695"] },
    ],
  },
];

// ---------- Session logs, in each Harness's native event shapes ----------
// Claude Code stream-json: assistant/user messages with content blocks, system events, tool_progress.
const claudeLog = [
  { type: "system", subtype: "init", model: "claude-opus-5-5", cwd: "/home/factory/repos/chart35-issue-239" },
  { type: "assistant", message: { content: [{ type: "text", text: "Reading the Ticket and the journal that #238 merged into the Spec branch." }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u1", name: "Bash", input: { command: "gh issue view 239 --json title,body,labels" } }] } },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u1", content: "title: Queue edits made while offline\nlabels: ready-for-agent\n\nWhen the device has no network, note edits go to a queue\ninstead of failing. The queue survives an app restart." }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u2", name: "Read", input: { file_path: "src/journal.rs", offset: 1, limit: 40 } }] } },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u2", content: "pub struct Journal {\n    entries: Vec<Entry>,\n}\n\nimpl Journal {\n    pub fn record(&mut self, edit: Edit) { /* … */ }\n}" }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u3", name: "Grep", input: { pattern: "fn send_edit", path: "src" } }] } },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u3", content: "src/sync.rs:18:pub fn send_edit(edit: &Edit) -> Result<(), SyncError> {\nsrc/sync.rs:44:fn send_edit_batch(edits: &[Edit]) -> Result<(), SyncError> {" }] } },
  { type: "system", subtype: "thinking_tokens", tokens: 1840 },
  { type: "assistant", message: { content: [{ type: "thinking", thinking: "The journal already records every edit. The simplest queue is the journal's tail past the last acknowledged entry, so nothing new needs persisting. Test first: an edit made offline is still there after a restart." }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u4", name: "Edit", input: { file_path: "tests/offline.rs", old_string: "", new_string: "#[test]\nfn an_offline_edit_survives_a_restart() {\n    let mut app = App::offline();\n    app.edit(\"note-1\", \"tomatoes staked\");\n    let app = app.restart();\n    assert_eq!(app.queued().len(), 1);\n}" } }] } },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u4", content: "The file tests/offline.rs has been updated." }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u5", name: "Bash", input: { command: "cargo test --test offline" } }] } },
  { type: "tool_progress", tool_use_id: "u5", elapsed_time_seconds: 12 },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u5", is_error: true, content: "running 1 test\ntest an_offline_edit_survives_a_restart ... FAILED\n\nassertion `left == right` failed\n  left: 0\n right: 1\n\ntest result: FAILED. 0 passed; 1 failed" }] } },
  { type: "assistant", message: { content: [{ type: "text", text: "Red, as expected. Now the queue: the journal's unacknowledged tail." }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u6", name: "Edit", input: { file_path: "src/journal.rs", old_string: "pub struct Journal {\n    entries: Vec<Entry>,\n}", new_string: "pub struct Journal {\n    entries: Vec<Entry>,\n    acked: usize,\n}\n\nimpl Journal {\n    pub fn queued(&self) -> &[Entry] {\n        &self.entries[self.acked..]\n    }\n}" } }] } },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u6", content: "The file src/journal.rs has been updated." }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u7", name: "Bash", input: { command: "cargo test --test offline" } }] } },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u7", content: "running 1 test\ntest an_offline_edit_survives_a_restart ... ok\n\ntest result: ok. 1 passed; 0 failed" }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u8", name: "Bash", input: { command: "git commit -am \"Queue edits made while offline\"" } }] } },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u8", content: "[issue-239 9c2e1f0] Queue edits made while offline\n 2 files changed, 19 insertions(+)" }] } },
  { type: "assistant", message: { content: [{ type: "text", text: "Implemented. Reviewing the branch against spec-230 with the code-review skill." }] } },
  { type: "system", subtype: "task_started", description: "Standards review of issue-239 against spec-230" },
  { type: "system", subtype: "task_started", description: "Spec review of issue-239 against chart35#239" },
  { type: "system", subtype: "task_progress", description: "Standards review: 3 files read" },
  { type: "system", subtype: "task_notification", status: "completed", description: "Spec review: 1 finding" },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u9", name: "Read", input: { file_path: ".thirdshift-review/spec.md" } }] } },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u9", content: "1. The queue is not cleared when the server acknowledges an edit." }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u10", name: "Bash", input: { command: "cargo test" } }] } },
  { type: "tool_progress", tool_use_id: "u10", elapsed_time_seconds: 38 },
];

// Lines the live session appends while the page is open, one every few seconds.
export const claudeLiveTail = [
  { type: "tool_progress", tool_use_id: "u10", elapsed_time_seconds: 52 },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u10", content: "running 41 tests\n…\ntest result: ok. 41 passed; 0 failed; finished in 47.2s" }] } },
  { type: "assistant", message: { content: [{ type: "thinking", thinking: "The Spec finding is right: acknowledging must move `acked`. Add a test for it before the fix." }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u11", name: "Grep", input: { pattern: "fn on_ack", path: "src" } }] } },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u11", content: "src/sync.rs:61:fn on_ack(&mut self, id: EntryId) {" }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u12", name: "Edit", input: { file_path: "src/sync.rs", old_string: "fn on_ack(&mut self, id: EntryId) {", new_string: "fn on_ack(&mut self, id: EntryId) {\n    self.journal.ack_through(id);" } }] } },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u12", content: "The file src/sync.rs has been updated." }] } },
  { type: "assistant", message: { content: [{ type: "tool_use", id: "u13", name: "Bash", input: { command: "cargo test --test offline" } }] } },
  { type: "user", message: { content: [{ type: "tool_result", tool_use_id: "u13", content: "test result: ok. 2 passed; 0 failed" }] } },
  { type: "assistant", message: { content: [{ type: "text", text: "Fixed the Spec finding with a test. No Standards findings left that I agree with." }] } },
];

// Codex exec --json: thread/turn/item events, then turn.completed with token usage.
const codexLog = [
  { type: "thread.started", thread_id: "th_7f3a" },
  { type: "turn.started" },
  { type: "item.completed", item: { id: "i0", type: "agent_message", text: "I'll find where the Repair budgets live before moving them." } },
  { type: "item.started", item: { id: "i1", type: "command_execution", command: "rg -n 'repair_budget' src" } },
  { type: "item.completed", item: { id: "i1", type: "command_execution", command: "rg -n 'repair_budget' src", aggregated_output: "src/delivery.rs:12:const REPAIR_BUDGET: u32 = 5;\nsrc/repair.rs:30:    if used >= repair_budget() {", exit_code: 0, status: "completed" } },
  { type: "item.started", item: { id: "i2", type: "command_execution", command: "sed -n '1,40p' src/repair.rs" } },
  { type: "item.completed", item: { id: "i2", type: "command_execution", command: "sed -n '1,40p' src/repair.rs", aggregated_output: "// Repairs a Delivery may start.\npub fn repair_budget() -> u32 { 5 }", exit_code: 0, status: "completed" } },
  { type: "item.completed", item: { id: "i3", type: "agent_message", text: "The budget is read in two places. Moving it onto the Delivery so both read one field." } },
  { type: "item.started", item: { id: "i4", type: "command_execution", command: "cargo test delivery::" } },
  { type: "item.completed", item: { id: "i4", type: "command_execution", command: "cargo test delivery::", aggregated_output: "test result: ok. 23 passed; 0 failed", exit_code: 0, status: "completed" } },
  { type: "item.started", item: { id: "i5", type: "command_execution", command: "git commit -am 'Fold Repair budgets into the Delivery'" } },
  { type: "item.completed", item: { id: "i5", type: "command_execution", command: "git commit -am 'Fold Repair budgets into the Delivery'", aggregated_output: "[issue-702 4f1c2a9] Fold Repair budgets into the Delivery\n 3 files changed, 14 insertions(+), 21 deletions(-)", exit_code: 0, status: "completed" } },
  { type: "turn.completed", usage: { input_tokens: 812004, cached_input_tokens: 760320, output_tokens: 9412 } },
];

// Every Session log the page can link. Only s239 and s702 have content; the rest show a stub.
export const sessions = {
  s239: { id: "s239", kind: "implement", harness: "claude", live: true, owner: "chart35#239",
    file: "chart35/commands/pickup/230-20261018T072101-0400-239-implement.jsonl", events: claudeLog, tail: claudeLiveTail },
  s702: { id: "s702", kind: "implement", harness: "codex", live: false, owner: "thirdshift#702",
    file: "thirdshift/commands/run/702-20261018T083012-0400-implement.jsonl", events: codexLog },
  s702sec: { id: "s702sec", kind: "security-review", harness: "codex", live: false, owner: "thirdshift#702", secure: true },
  s238: { id: "s238", kind: "implement", harness: "claude", owner: "chart35#238" },
  s88: { id: "s88", kind: "implement", harness: "codex", owner: "gardenkit#88" },
  s698: { id: "s698", kind: "implement", harness: "codex", owner: "thirdshift#698" },
  s6audit: { id: "s6audit", kind: "security-audit", harness: "claude", owner: "thirdshift", secure: true },
  s6repro1: { id: "s6repro1", kind: "security-reproduction", harness: "claude", owner: "thirdshift", secure: true },
  s6repro2: { id: "s6repro2", kind: "security-reproduction", harness: "claude", owner: "thirdshift", secure: true },
  s704: { id: "s704", kind: "implement", harness: "claude", owner: "thirdshift#704" },
  s226: { id: "s226", kind: "implement", harness: "claude", owner: "chart35#226" },
  s690: { id: "s690", kind: "implement", harness: "codex", owner: "thirdshift#690" },
  s690r1: { id: "s690r1", kind: "Repair", harness: "codex", owner: "thirdshift#690" },
  s691: { id: "s691", kind: "implement", harness: "codex", owner: "thirdshift#691" },
  s71: { id: "s71", kind: "implement", harness: "codex", owner: "gardenkit#71" },
  s72: { id: "s72", kind: "implement", harness: "codex", owner: "gardenkit#72" },
  s72resume: { id: "s72resume", kind: "Resume", harness: "codex", owner: "gardenkit#72" },
  s219: { id: "s219", kind: "implement", harness: "claude", owner: "chart35#219" },
  s11arch: { id: "s11arch", kind: "architecture-review", harness: "codex", owner: "thirdshift" },
  s695: { id: "s695", kind: "implement", harness: "codex", owner: "thirdshift#695" },
};

// ---------- Earlier Runs on this machine ----------
// Stage minutes for the last Runs that finished here, newest first: where "usually 10–20 min" comes from, and the
// stage grid's columns. Made up, as everything here is.
const past = (issue, endedMin, outcome, mins, extra = {}) => ({ issue, ended: ago(endedMin), outcome, mins, ...extra });
export const pastRuns = [
  past("thirdshift#698", 300, "merged", { implement: 31, review: 14, Delivery: 2, "CI watch": 18, Repair: 9, "Self-merge": 1, cleanup: 1 }, { repairs: 1 }),
  past("chart35#238", 71, "merged", { implement: 26, review: 12, Delivery: 2, "CI watch": 15, "Self-merge": 1, cleanup: 1 }),
  past("chart35#226", 1164, "ready", { implement: 38, review: 19, Delivery: 3, "CI watch": 6 }),
  past("thirdshift#691", 1412, "failed", { implement: 12, review: 6, Delivery: 1, "CI watch": 21, Repair: 8 }, { repairs: 1 }),
  past("thirdshift#690", 1410, "failed", { implement: 29, review: 15, Delivery: 2, "CI watch": 24, Repair: 17 }, { repairs: 2 }),
  past("gardenkit#71", 1840, "merged", { implement: 22, review: 11, Delivery: 1, "CI watch": 9, "Self-merge": 1, cleanup: 1 }),
  past("gardenkit#72", 1746, "failed", { implement: 81, Resume: 11 }, { resumes: 1 }),
  past("chart35#219", 2890, "interrupted", { implement: 33, review: 16, Delivery: 4 }, { resumes: 1 }),
  past("thirdshift#704", 392, "ready", { implement: 19, review: 10, Delivery: 2, "CI watch": 7 }),
  past("thirdshift#695", 2990, "merged", { implement: 41, review: 18, Delivery: 2, "CI watch": 13, "Self-merge": 1, cleanup: 1 }),
  past("chart35#212", 4300, "merged", { implement: 24, review: 13, Delivery: 2, "CI watch": 12, "Self-merge": 1, cleanup: 1 }),
  past("thirdshift#681", 5100, "merged", { implement: 35, review: 17, Delivery: 3, "CI watch": 16, Repair: 6, "Self-merge": 1, cleanup: 1 }, { repairs: 1 }),
];

// Tokens and length per Session log, as thirdshift prints them when a session ends.
const usage = {
  s239: [5_412_330, 5_190_002, 14_220, 70], s238: [3_880_412, 3_702_118, 11_034, 58], s702: [812_004, 760_320, 9_412, 31],
  s702sec: [402_118, 377_090, 3_120, 6], s88: [1_204_556, 1_100_002, 6_801, 15], s698: [2_904_331, 2_781_009, 8_811, 45],
  s6audit: [1_788_102, 1_650_330, 7_402, 24], s6repro1: [602_441, 570_118, 2_904, 9], s6repro2: [511_920, 480_004, 2_115, 7],
  s704: [1_990_420, 1_880_010, 6_660, 29], s226: [4_102_993, 3_940_118, 12_884, 57], s690: [2_660_118, 2_501_442, 8_002, 44],
  s690r1: [902_331, 850_118, 3_010, 17], s691: [701_220, 660_004, 2_440, 20], s71: [1_840_002, 1_740_331, 5_902, 33],
  s72: [3_990_118, 3_811_204, 9_330, 81], s72resume: [602_118, 560_002, 1_802, 11], s219: [2_440_001, 2_330_118, 7_118, 53],
  s11arch: [1_402_330, 1_320_118, 5_220, 22], s695: [3_102_441, 2_960_330, 9_904, 61],
};
for (const [id, [input, cached, output, minutes]] of Object.entries(usage)) Object.assign(sessions[id], { input, cached, output, minutes });

// ---------- Session log content for every other session ----------
// Only s239 and s702 are scripted by hand above. Every other Session log gets invented content here, in its
// Harness's own event shapes (Claude Code stream-json, Codex exec --json), scripted by session kind. A session
// whose Command died ends mid-call, with no result, as a killed process would leave it.
const titleOf = {};
const diedIssues = new Set();
(function walk(list) {
  for (const r of list) {
    if (r.issue) titleOf[r.issue] = r.title;
    if (r.alive === false && !r.outcome && r.issue) diedIssues.add(r.issue);
    walk([...(r.children ?? []), ...(r.tickets ?? [])]);
  }
})(commands);
const FILES = {
  chart35: ["src/sync.rs", "src/notes.rs", "tests/sync.rs"],
  thirdshift: ["src/delivery.rs", "src/logs.rs", "tests/delivery.rs"],
  gardenkit: ["src/lots.rs", "src/labels.rs", "tests/labels.rs"],
};
// Steps: [say, text] [think, text] [bash, cmd, output, exit] [read, path, output] [grep, pattern, path, output]
// [edit, path, old, new] [task, description]
function script(s) {
  const issue = s.owner.includes("#") ? s.owner : null;
  const repo = s.owner.split("#")[0];
  const n = issue?.split("#")[1];
  const title = issue ? titleOf[issue] ?? "the issue" : "";
  const [src, src2, test] = FILES[repo] ?? FILES.thirdshift;
  const fail = (cmd, out) => ["bash", cmd, out, 101];
  switch (s.kind) {
    case "implement": return [
      ["say", `Reading ${issue}: ${title}.`],
      ["bash", `gh issue view ${n} --json title,body,labels`, `title: ${title}\nlabels: in-progress\n\nThe change should be covered by a test.`, 0],
      ["grep", "pub fn", src, `${src}:12:pub fn load(path: &Path) -> Result<Self>\n${src}:40:pub fn save(&self) -> Result<()>`],
      ["read", src, "pub struct Store {\n    items: Vec<Item>,\n}\n\nimpl Store {\n    pub fn load(path: &Path) -> Result<Self> { /* … */ }\n}"],
      ["think", "Smallest change first: a failing test that states the behaviour the issue asks for, then the code."],
      ["edit", test, "", `#[test]\nfn ${(title.split(" ")[0] ?? "it").toLowerCase()}_works() {\n    let store = Store::fixture();\n    assert!(store.check());\n}`],
      fail(`cargo test --test ${test.split("/")[1].replace(".rs", "")}`, "running 1 test\ntest it_works ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed"),
      ["edit", src, "impl Store {", "impl Store {\n    pub fn check(&self) -> bool {\n        !self.items.is_empty()\n    }"],
      ["bash", `cargo test --test ${test.split("/")[1].replace(".rs", "")}`, "running 1 test\ntest it_works ... ok\n\ntest result: ok. 1 passed; 0 failed", 0],
      ["bash", `git commit -am "${title}"`, `[issue-${n} 3e91a0c] ${title}\n 2 files changed, 11 insertions(+)`, 0],
      ["say", "Implemented. Reviewing the branch against the Base branch with the code-review skill."],
      ["task", `Standards review of issue-${n}`],
      ["task", `Spec review of issue-${n} against ${issue}`],
      ["bash", "cargo test", "test result: ok. 64 passed; 0 failed; finished in 38.0s", 0],
      ["say", "No findings I disagree with are left. Opening the pull request, ready for review."],
    ];
    case "Repair": return [
      ["say", "CI is red on clippy for this head; reproducing it locally."],
      ["bash", "gh run view --log-failed", "clippy: error: this `if` has identical blocks\n  --> src/logs.rs:88:5", 1],
      fail("cargo clippy --all-targets -- -D warnings", "error: this `if` has identical blocks\n  --> src/logs.rs:88:5\nerror: could not compile `thirdshift`"),
      ["read", "src/logs.rs", "if skipped {\n    write_line(&line)\n} else {\n    write_line(&line)\n}"],
      ["think", "The lint also fires on main at the merge base, so this is an Inherited failure; nothing on the branch to fix."],
      ["say", "The failing lint is on main as well, not on this branch. Nothing to fix here."],
    ];
    case "Resume": return [
      ["say", "Resuming: the last turn ended with `cargo test` still running in the background. Running it in the foreground."],
      fail("cargo test", "running 18 tests\ntest labels_for_bed ... FAILED\n\ntest result: FAILED. 17 passed; 1 failed"),
      ["read", src2, "pub fn labels_for_bed(bed: &Bed) -> Vec<Label> {\n    todo!()\n}"],
      ["say", "The batch path is still a todo!(); the earlier session never got to it. Stopping here: the Run will fail with that cause."],
    ];
    case "security-review": return [
      ["say", `Reviewing the change on issue-${n} against main for vulnerabilities.`],
      ["bash", "git diff --stat main...HEAD", " src/delivery.rs | 21 +++++++++------\n src/repair.rs   | 14 ++++------\n 2 files changed, 14 insertions(+), 21 deletions(-)", 0],
      ["grep", "Command::new|std::fs::remove", "src", "src/delivery.rs:131:    Command::new(\"git\").args([\"push\", \"origin\", branch])"],
      ["read", "src/delivery.rs", "let branch = validated_branch(&run)?;\nCommand::new(\"git\").args([\"push\", \"origin\", branch])"],
      ["think", "The branch name is validated before it reaches git, and it is passed as one argument, never through a shell."],
      ["say", "No Security findings in this change."],
    ];
    case "security-audit": return [
      ["say", "Auditing main for vulnerabilities, starting where the factory touches paths and processes."],
      ["grep", "Path::join|canonicalize", "src", "src/logs.rs:52:    let path = root.join(name);\nsrc/logs.rs:97:    fs::read_to_string(root.join(rel))?"],
      ["read", "src/logs.rs", "pub fn open_log(root: &Path, rel: &str) -> io::Result<String> {\n    fs::read_to_string(root.join(rel))\n}"],
      ["think", "rel is not checked for .. or a symlink, so a crafted name could read outside the logs folder."],
      ["grep", "\\x1b|escape", "src/console", "(no matches)"],
      ["say", "Two candidate Security findings: a Command log path that can leave the logs folder (high), and raw terminal escapes echoed by the log viewer (low). Handing both to reproduction."],
    ];
    case "security-reproduction": return s.id === "s6repro1" ? [
      ["say", "Reproducing: a Command log path can follow a symlink out of the logs folder."],
      ["edit", "tests/poc_log_path.rs", "", "#[test]\nfn log_path_stays_inside() {\n    let root = scratch_logs_with_symlink_out();\n    assert!(open_log(&root, \"link/secret\").is_err());\n}"],
      fail("cargo test --test poc_log_path", "test log_path_stays_inside ... FAILED\nread outside the logs folder: ok(\"made-up secret\")"),
      ["say", "Reproduced: the proof-of-concept test fails on main. Severity high."],
    ] : [
      ["say", "Reproducing: the Session log viewer echoes raw terminal escapes."],
      ["edit", "tests/poc_escapes.rs", "", "#[test]\nfn escapes_are_shown_as_text() {\n    let html = render_line(\"\\u{1b}[31mred\");\n    assert!(!html.contains('\\u{1b}'));\n}"],
      ["bash", "cargo test --test poc_escapes", "test escapes_are_shown_as_text ... ok", 0],
      ["say", "Not reproduced: the viewer already escapes them. It waits for the Day shift and is never fixed unattended."],
    ];
    case "architecture-review": return [
      ["say", "Architecture review of main: looking for detritus that recent features left."],
      ["bash", "rg --files src | wc -l", "48", 0],
      ["grep", "fn release_claim|fn sweep", "src", "src/claim.rs:30:pub fn release_claim(…)\nsrc/pickup.rs:212:fn sweep(…)"],
      ["read", "src/pickup.rs", "// The Sweep lives here, but it only edits Claims.\nfn sweep(repo: &Repo) -> Result<()> { /* … */ }"],
      ["think", "The Sweep only edits Claims; it belongs beside the Claim helpers, and pickup.rs gets shorter."],
      ["say", "Publishing an Architect plan: Move the Claim helpers next to the Sweep."],
    ];
    default: return [["say", `A ${s.kind} session for ${s.owner}.`]];
  }
}
// Codex has no Read, Grep or Edit tools: they appear as the shell commands it would run.
function asCodex(step) {
  const [k, a, b, c] = step;
  if (k === "read") return ["bash", `sed -n '1,40p' ${a}`, b, 0];
  if (k === "grep") return ["bash", `rg -n '${a}' ${b}`, c, 0];
  if (k === "edit") return ["bash", `apply_patch <<'PATCH' (${a})`, `Success. Updated the following files:\nM ${a}`, 0];
  return step;
}
function events(s) {
  let steps = script(s);
  const died = diedIssues.has(s.owner);
  if (died) steps = steps.slice(0, 8); // ends on the Edit the process was killed in
  const out = [];
  let k = 0;
  if (s.harness === "claude") {
    out.push({ type: "system", subtype: "init", model: "claude-opus-5-5", cwd: `/home/factory/repos/${s.owner.replace("#", "-issue-")}` });
    for (const step of steps) {
      const id = `g${s.id}${k++}`;
      const [t, a, b, c, d] = step;
      const use = (name, input) => out.push({ type: "assistant", message: { content: [{ type: "tool_use", id, name, input }] } });
      const res = (content, err) => out.push({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: id, content, ...(err ? { is_error: true } : {}) }] } });
      if (t === "say") out.push({ type: "assistant", message: { content: [{ type: "text", text: a }] } });
      else if (t === "think") out.push({ type: "assistant", message: { content: [{ type: "thinking", thinking: a }] } });
      else if (t === "task") out.push({ type: "system", subtype: "task_started", description: a });
      else if (t === "bash") { use("Bash", { command: a }); res(b, c !== 0); }
      else if (t === "read") { use("Read", { file_path: a }); res(b); }
      else if (t === "grep") { use("Grep", { pattern: a, path: b }); res(c); }
      else if (t === "edit") { use("Edit", { file_path: a, old_string: b, new_string: c }); res(`The file ${a} has been updated.`); }
    }
    if (died) out.pop(); // the last call never got its result
  } else {
    out.push({ type: "thread.started", thread_id: `th_${s.id}` }, { type: "turn.started" });
    for (const step of steps.map(asCodex)) {
      const id = `i${k++}`;
      const [t, a, b, c] = step;
      if (t === "say") out.push({ type: "item.completed", item: { id, type: "agent_message", text: a } });
      else if (t === "bash") {
        out.push({ type: "item.started", item: { id, type: "command_execution", command: a } });
        out.push({ type: "item.completed", item: { id, type: "command_execution", command: a, aggregated_output: b, exit_code: c, status: "completed" } });
      }
    }
    if (died) out.pop();
    else out.push({ type: "turn.completed", usage: { input_tokens: s.input ?? 0, cached_input_tokens: s.cached ?? 0, output_tokens: s.output ?? 0 } });
  }
  return out;
}
for (const s of Object.values(sessions)) {
  if (s.events) continue;
  s.events = events(s);
  s.file ??= `${s.owner.split("#")[0]}/commands/${s.id}-${s.kind}.jsonl`;
}
