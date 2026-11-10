//! The blind pairwise page: what a reader sees before and after their calls,
//! the order of the calls, append-only records and the CLI twin.
use super::*;

const BASELINE_MODEL: &str = "opus-fixture";
const CANDIDATE_MODEL: &str = "gpt-fixture";

/// A paused process Experiment with one retained pair. The Baseline's
/// change touches `left`, the Candidate's `right`; the panel prefers the
/// Baseline and calls only it ready.
fn fixture(home: &Path) -> (String, String) {
    use sha2::{Digest, Sha256};
    let id = "f".repeat(64);
    let directory = home.join(".thirdshift/evals/experiments").join(&id);
    // The Experiment keeps its own copy of the task's folder path.
    let task = home.join("retained-task");
    fs::create_dir_all(&task).unwrap();
    fs::write(
        task.join("issue.md"),
        "# Count the widgets\n\nCount them twice.\n\n## Acceptance criteria\n\n- [ ] The count is shown\n- [ ] A <second> count matches\n",
    )
    .unwrap();
    fs::create_dir_all(directory.join("pairs")).unwrap();
    fs::write(directory.join("experiment.json"), serde_json::to_vec(&json!({
        "format": 1, "id": id, "state": "paused", "created_at": "2026-11-01T10:00:00Z",
        "baseline": {"commit": "a".repeat(40), "harness": "claude", "model": BASELINE_MODEL, "effort": null, "process": "ticket-run"},
        "candidate": {"commit": "b".repeat(40), "harness": "codex", "model": CANDIDATE_MODEL, "effort": null, "process": "ticket-run"},
        "suites": ["process"], "prepared": null,
        "bar": {"margin": -0.1, "credible_level": 0.95, "prior": 1.0, "seed": 0, "draws": 100,
            "trials_per_task": {"process": 1}, "timeout_seconds": 1800,
            "void_rules": {"retries": 2, "causes": ["memory_cap"]}},
        "tasks": [{"suite": "process", "name": "fixture", "directory": task,
            "manifest": {"format": 1, "state": "approved", "session": "process",
                "repository": "repository", "base": "c".repeat(40), "change": "d".repeat(40),
                "criteria": "Count the widgets.", "inputs": {".thirdshift-issue.md": "issue.md"},
                "trial": {"issue_url": "https://github.com/acme/widgets/issues/7", "base": "main",
                    "branch": "issue-7", "pr_url": "https://github.com/acme/widgets/pull/1"}}}]
    })).unwrap()).unwrap();
    let side = |name: &str| {
        json!({
            "diff": [
                {"path": format!("tests/{name}_test.py"), "test": true, "patch": format!("+def test_{name}(): pass\n")},
                {"path": format!("src/{name}.py"), "test": false, "patch": format!("+def {name}(): return 2\n")},
            ],
            "test_files": [format!("tests/{name}_test.py")], "fails_on_base": true,
            "passes_on_other": true, "disagreement": false, "base": [], "cross": [], "error": null
        })
    };
    let pair = format!("{:x}--pair--0", Sha256::digest(b"process\0fixture"));
    fs::write(
        directory.join("pairs").join(format!("{pair}.json")),
        serde_json::to_vec(&json!({
            "suite": "process", "task": "fixture", "trial": 0,
            "baseline": side("left"), "candidate": side("right"),
            "panel": {"baseline_ready": true, "candidate_ready": false, "preference": "baseline"}
        }))
        .unwrap(),
    )
    .unwrap();
    (id, pair)
}

fn post(
    console: &Console,
    token: &str,
    path: &str,
    form: &[(&str, &str)],
) -> reqwest::blocking::Response {
    actions::post(console, token, path, form)
}

/// Which side holds the Baseline's change, read from the blind view's diffs.
fn baseline_side(view: &Value) -> &'static str {
    if view["a"]["diff"].to_string().contains("src/left.py") {
        "a"
    } else {
        "b"
    }
}

#[test]
fn the_blind_page_names_no_arm_and_shows_the_ticket_checklist_diffs_and_evidence() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let (id, pair) = fixture(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let response = console.get(&format!("/experiments/{id}/pairs/{pair}"));
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    let main = html
        .split_once("<main")
        .unwrap()
        .1
        .split_once("</main>")
        .unwrap()
        .0;
    for hidden in [
        "Baseline",
        "baseline",
        "Candidate",
        "candidate",
        &"a".repeat(40),
        &"b".repeat(40),
        "claude",
        "codex",
        BASELINE_MODEL,
        CANDIDATE_MODEL,
        "ticket-run",
        "prefers",
    ] {
        assert!(
            !html.contains(hidden),
            "{hidden} shown before the calls: {html}"
        );
    }
    // The navigation links to Scores; the page itself shows none.
    assert!(!main.contains("Score"), "{main}");
    assert!(main.contains("Count them twice."), "{main}");
    assert!(
        main.contains(
            "<ol><li>The count is shown</li><li>A &#60;second&#62; count matches</li></ol>"
        ),
        "{main}"
    );
    for side in ["A", "B"] {
        let change = main
            .split_once(&format!("<article aria-label=\"Change {side}\">"))
            .unwrap()
            .1
            .split_once("</article>")
            .unwrap()
            .0;
        let code = change.find(".py</code></summary>").unwrap();
        let test = change.find("_test.py</code> · test").unwrap();
        assert!(code < test, "code before tests: {change}");
        assert!(
            change.contains("1 new test file(s); on the base: fail; on the other change: pass"),
            "{change}"
        );
    }
    assert!(main.contains("Seal both merge calls"), "{main}");
    assert!(!main.contains("Save preference"), "{main}");
}

#[test]
fn the_preference_waits_for_both_merge_calls_and_records_are_append_only() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let (id, pair) = fixture(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let reader = tokens::create(home.path(), "reader", &[]);
    let token = tokens::create(home.path(), "grader", &["--scopes", "act"]);
    let base = format!("/api/v1/experiments/{id}/pairs/{pair}");

    let refused = post(
        &console,
        &token,
        &format!("{base}/preference"),
        &[("preference", "a"), ("rubric_scope", "b")],
    );
    assert_eq!(refused.status(), 422);
    assert!(refused.text().unwrap().contains("both merge calls"));
    let one = post(
        &console,
        &token,
        &format!("{base}/merge"),
        &[("a", "ready")],
    );
    assert_eq!(one.status(), 422, "one merge call is not enough");
    assert_eq!(
        post(
            &console,
            &reader,
            &format!("{base}/merge"),
            &[("a", "ready"), ("b", "ready")]
        )
        .status(),
        403
    );

    let sealed = api::json(post(
        &console,
        &token,
        &format!("{base}/merge"),
        &[("a", "ready"), ("b", "not-ready")],
    ));
    assert_eq!(sealed["next"], "preference", "{sealed}");
    assert_eq!(sealed["revealed"], Value::Null, "{sealed}");
    let first = api::json(post(
        &console,
        &token,
        &format!("{base}/preference"),
        &[
            ("preference", "a"),
            ("rubric_tests", "b"),
            ("rubric_scope", ""),
        ],
    ));
    assert_eq!(first["mine"].as_array().unwrap().len(), 2, "{first}");
    assert_eq!(first["mine"][1]["rubric"], json!({"tests": "b"}), "{first}");

    // A second preference needs new merge calls; a second review appends.
    assert_eq!(
        post(
            &console,
            &token,
            &format!("{base}/preference"),
            &[("preference", "b")]
        )
        .status(),
        422
    );
    post(
        &console,
        &token,
        &format!("{base}/merge"),
        &[("a", "not-ready"), ("b", "not-ready")],
    );
    let second = api::json(post(
        &console,
        &token,
        &format!("{base}/preference"),
        &[("preference", "both-bad")],
    ));
    let mine = second["mine"].as_array().unwrap();
    assert_eq!(mine.len(), 4, "{second}");
    assert_eq!(
        mine[..2],
        first["mine"].as_array().unwrap()[..],
        "earlier records unchanged"
    );
    assert_eq!(mine[0]["reader"]["who"], "token grader");
    assert_eq!(mine[0]["reader"]["from_console"], true);
    let records = home
        .path()
        .join(".thirdshift/evals/experiments")
        .join(&id)
        .join("reviews")
        .join(&pair);
    let files = fs::read_dir(records)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "json")
        })
        .count();
    assert_eq!(files, 4);
}

#[test]
fn after_the_preference_the_reader_sees_the_arms_and_the_panels_calls() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let (id, pair) = fixture(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let base = format!("/experiments/{id}/pairs/{pair}");
    let html = console.get(&base).text().unwrap();
    let csrf = html
        .split("name=\"csrf\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_owned();
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let form = |path: &str, fields: &[(&str, &str)]| {
        let mut fields = fields.to_vec();
        fields.push(("csrf", &csrf));
        client
            .post(format!("{}{base}/{path}", console.address))
            .form(&fields)
            .send()
            .unwrap()
    };
    assert_eq!(
        form("merge", &[("a", "ready"), ("b", "ready")]).status(),
        303
    );
    let sealed = console.get(&base).text().unwrap();
    assert!(
        sealed.contains("merge calls: A ready · B ready"),
        "{sealed}"
    );
    assert!(sealed.contains("Save preference"), "{sealed}");
    assert!(
        !sealed.contains("Baseline") && !sealed.contains(CANDIDATE_MODEL),
        "{sealed}"
    );
    assert_eq!(form("preference", &[("preference", "tie")]).status(), 303);

    let revealed = console.get(&base).text().unwrap();
    let view = api::json(console.get(&format!("/api/v1{base}")));
    let side = baseline_side(&view);
    let (baseline, candidate) = if side == "a" { ("A", "B") } else { ("B", "A") };
    assert_eq!(view["revealed"][side], "baseline", "{view}");
    assert!(
        revealed.contains(&format!(
            "<dt>Change {baseline}</dt><dd>Baseline arm · claude · {BASELINE_MODEL}"
        )),
        "{revealed}"
    );
    assert!(
        revealed.contains(&format!(
            "<dt>Change {candidate}</dt><dd>Candidate arm · codex · {CANDIDATE_MODEL}"
        )),
        "{revealed}"
    );
    assert!(
        revealed.contains("Baseline ready · Candidate not ready · prefers Baseline"),
        "{revealed}"
    );
    assert_eq!(
        view["revealed"]["panel"]["preference"], "baseline",
        "{view}"
    );
}

#[test]
fn the_cli_records_the_same_calls_and_they_show_beside_the_verdict() {
    let cli_home = tempfile::tempdir().unwrap();
    let api_home = tempfile::tempdir().unwrap();
    configure(cli_home.path());
    configure(api_home.path());
    let (id, pair) = fixture(cli_home.path());
    fixture(api_home.path());
    let run = |args: &[&str]| {
        let mut full = vec!["experiments", id.as_str(), "--pair", pair.as_str()];
        full.extend_from_slice(args);
        Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .env("HOME", cli_home.path())
            .args(full)
            .output()
            .unwrap()
    };
    let early = run(&["--prefer", "a", "--json"]);
    assert!(!early.status.success());
    assert!(String::from_utf8_lossy(&early.stderr).contains("both merge calls"));
    let blind = run(&[]);
    let text = String::from_utf8(blind.stdout).unwrap();
    assert!(text.contains("1. The count is shown"), "{text}");
    assert!(
        !text.contains("Baseline") && !text.contains("claude"),
        "{text}"
    );
    assert!(
        run(&["--merge", "ready", "not-ready", "--json"])
            .status
            .success()
    );
    let output = run(&["--prefer", "a", "--rubric", "scope=b", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cli: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(cli["mine"][0]["reader"]["from_console"], false, "{cli}");
    assert_ne!(cli["revealed"], Value::Null, "{cli}");

    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), api_home.path());
    let token = tokens::create(api_home.path(), "grader", &["--scopes", "act"]);
    let base = format!("/api/v1/experiments/{id}/pairs/{pair}");
    post(
        &console,
        &token,
        &format!("{base}/merge"),
        &[("a", "ready"), ("b", "not-ready")],
    );
    post(
        &console,
        &token,
        &format!("{base}/preference"),
        &[("preference", "a"), ("rubric_scope", "b")],
    );
    let calls = |view: Value| {
        let mut reviews = view["people"]["reviews"].clone();
        for review in reviews.as_array_mut().unwrap() {
            let review = review.as_object_mut().unwrap();
            review.remove("reader");
            review.remove("recorded_at");
        }
        (reviews, view["people"]["agreement"].clone())
    };
    let (reviews, agreement) = calls(api::json(console.get(&format!("/api/v1/experiments/{id}"))));
    assert_eq!(
        (reviews.clone(), agreement.clone()),
        calls(api::cli(cli_home.path(), &["experiments", &id, "--json"]))
    );
    let side = baseline_side(&cli);
    let (baseline_ready, preferred) = if side == "a" {
        (true, "baseline")
    } else {
        (false, "candidate")
    };
    assert_eq!(reviews[0]["baseline_ready"], baseline_ready, "{reviews}");
    assert_eq!(reviews[0]["candidate_ready"], !baseline_ready, "{reviews}");
    assert_eq!(reviews[0]["preference"], preferred, "{reviews}");
    // The panel called only the Baseline ready.
    let agreed = if baseline_ready { 2 } else { 0 };
    assert_eq!(agreement, json!({"compared": 2, "agreed": agreed}));

    let html = console.get(&format!("/experiments/{id}")).text().unwrap();
    let section = html
        .split_once("<section aria-label=\"People's calls\">")
        .expect("people's calls beside the Verdict")
        .1
        .split_once("</section>")
        .unwrap()
        .0;
    assert!(
        section.contains(&format!("/experiments/{id}/pairs/{pair}")),
        "{section}"
    );
    assert!(section.contains("<td>token grader</td>"), "{section}");
    assert!(
        section.contains(&format!("People–panel agreement: {agreed}/2 merge calls")),
        "{section}"
    );
    let text = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .env("HOME", cli_home.path())
        .args(["experiments", &id])
        .output()
        .unwrap();
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(
        text.contains("People process/fixture #1 · from the CLI by"),
        "{text}"
    );
    assert!(
        text.contains(&format!("People–panel agreement: {agreed}/2 merge calls")),
        "{text}"
    );
}

#[test]
fn sealed_merge_calls_cannot_be_replaced_or_remade_after_the_reveal() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let (id, pair) = fixture(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let token = tokens::create(home.path(), "grader", &["--scopes", "act"]);
    let base = format!("/api/v1/experiments/{id}/pairs/{pair}");
    post(
        &console,
        &token,
        &format!("{base}/merge"),
        &[("a", "ready"), ("b", "ready")],
    );
    let replaced = post(
        &console,
        &token,
        &format!("{base}/merge"),
        &[("a", "not-ready"), ("b", "not-ready")],
    );
    assert_eq!(replaced.status(), 422);
    assert!(replaced.text().unwrap().contains("sealed"));
    post(
        &console,
        &token,
        &format!("{base}/preference"),
        &[("preference", "tie")],
    );
    // A second review after the reveal is kept, but it is not blind.
    post(
        &console,
        &token,
        &format!("{base}/merge"),
        &[("a", "not-ready"), ("b", "not-ready")],
    );
    let view = api::json(console.get(&format!("/api/v1/experiments/{id}")));
    let reviews = view["people"]["reviews"].as_array().unwrap();
    assert_eq!(reviews.len(), 2, "{view}");
    assert_eq!(
        (reviews[0]["blind"].clone(), reviews[1]["blind"].clone()),
        (json!(true), json!(false))
    );
    // The panel called only the Baseline ready; the blind calls were both ready.
    assert_eq!(
        view["people"]["agreement"],
        json!({"compared": 2, "agreed": 1}),
        "{view}"
    );
    let html = console.get(&format!("/experiments/{id}")).text().unwrap();
    assert!(html.contains("tie</td><td>yes</td>"), "{html}");
    assert!(
        html.contains("—</td><td>no, after the reveal</td>"),
        "{html}"
    );
}

/// Sets the retained Bar's people, as `--people` flags would before the run.
fn set_people(home: &Path, id: &str, people: Value) {
    let path = home
        .join(".thirdshift/evals/experiments")
        .join(id)
        .join("experiment.json");
    let mut experiment: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    experiment["bar"]["people"] = people;
    fs::write(&path, serde_json::to_vec(&experiment).unwrap()).unwrap();
}

#[test]
fn each_readers_queue_lists_their_pairs_and_what_they_have_called() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let (id, pair) = fixture(home.path());
    set_people(
        home.path(),
        &id,
        json!({"role": "counted", "readers": ["token grader", "bo@example.com"], "sample": null, "overlap": 0}),
    );
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let token = tokens::create(home.path(), "grader", &["--scopes", "act"]);
    post(
        &console,
        &token,
        &format!("/api/v1/experiments/{id}/pairs/{pair}/merge"),
        &[("a", "ready"), ("b", "not-ready")],
    );
    let view = api::json(console.get(&format!("/api/v1/experiments/{id}")));
    assert_eq!(
        view["people"]["queues"],
        json!([{"reader": "token grader", "pairs": [pair]}, {"reader": "bo@example.com", "pairs": [pair]}])
    );
    let html = console.get(&format!("/experiments/{id}")).text().unwrap();
    let section = html
        .split_once("<section aria-label=\"People's calls\">")
        .unwrap()
        .1
        .split_once("</section>")
        .unwrap()
        .0;
    assert!(section.contains("counted in the Verdict"), "{section}");
    let link = format!("<a href=\"/experiments/{id}/pairs/{pair}\">process/fixture #1</a>");
    assert!(
        section.contains(&format!(
            "<h4>token grader</h4><ul><li>{link} · called</li></ul>"
        )),
        "{section}"
    );
    assert!(
        section.contains(&format!(
            "<h4>bo@example.com</h4><ul><li>{link} · to read</li></ul>"
        )),
        "{section}"
    );
    assert!(
        section.contains("People–people agreement: no pair read by two readers"),
        "{section}"
    );
}

#[test]
fn with_people_off_no_merge_call_is_taken() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let (id, pair) = fixture(home.path());
    set_people(
        home.path(),
        &id,
        json!({"role": "off", "readers": [], "sample": null, "overlap": 0}),
    );
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let token = tokens::create(home.path(), "grader", &["--scopes", "act"]);
    let refused = post(
        &console,
        &token,
        &format!("/api/v1/experiments/{id}/pairs/{pair}/merge"),
        &[("a", "ready"), ("b", "ready")],
    );
    assert_eq!(refused.status(), 422);
    assert!(refused.text().unwrap().contains("people off"));
    let html = console.get(&format!("/experiments/{id}")).text().unwrap();
    assert!(
        html.contains("The Bar has people off: no one grades these pairs."),
        "{html}"
    );
}
