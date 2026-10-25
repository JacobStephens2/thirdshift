use super::*;

const PR: &str = "https://github.com/acme/widgets/pull/23";
const CRITERIA: [&str; 5] = ["Correct", "Scoped", "Tested", "Honest", "Standards"];

fn cli(home: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .env("HOME", home)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn csrf(html: &str) -> &str {
    html.split("name=\"csrf\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
}

fn fixture(home: &Path) {
    let output = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/console/scoring_fixture.py"
        ))
        .arg(home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn origin_command(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
    command.env("HOME", home).env(
        "PATH",
        format!(
            "{}:{}",
            home.join("bin").display(),
            std::env::var("PATH").unwrap()
        ),
    );
    command
}

#[test]
fn scoring_actions_leave_console_terminable() {
    for path in ["/scores/mine", "/bug-origins/settle"] {
        let home = tempfile::tempdir().unwrap();
        fixture(home.path());
        let mut start = origin_command(home.path());
        start.args(["console", "--no-sign-in"]);
        let mut console = Console::start_command(start);
        let html = console.get("/scores").text().unwrap();
        let mut form = vec![("csrf".to_owned(), csrf(&html).to_owned())];
        if path == "/scores/mine" {
            form.push(("pull_request".into(), PR.into()));
            for criterion in CRITERIA {
                form.push((criterion.into(), "pass".into()));
                form.push((format!("{criterion}_reason"), "My reason".into()));
            }
        } else {
            form.push(("issue".into(), "10".into()));
            form.push(("action".into(), "reject".into()));
        }
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        assert_eq!(
            client
                .post(format!("{}{path}", console.address))
                .form(&form)
                .send()
                .unwrap()
                .status(),
            303
        );
        // Exact child PID; Drop still kills and reaps it if this assertion fails.
        assert_eq!(
            unsafe { libc::kill(i32::try_from(console.child.id()).unwrap(), libc::SIGTERM) },
            0
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if console.child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{path} swallowed Console SIGTERM"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn bug_origin_actions_match_cli_settlements_and_require_csrf() {
    let home = tempfile::tempdir().unwrap();
    let cli_home = tempfile::tempdir().unwrap();
    fixture(home.path());
    fixture(cli_home.path());
    let mut start = origin_command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    let response = console.get("/bug-origins");
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    assert!(html.contains("proposed"));
    assert!(html.contains("41. &#60;script&#62;"));
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let url = format!("{}/bug-origins/settle", console.address);
    for (issue, action, flag) in [
        ("8", "confirm", "--confirm"),
        ("9", "correct", "--correct"),
        ("10", "reject", "--none"),
    ] {
        let mut form = vec![("issue", issue), ("action", action), ("pull_request", "24")];
        assert_eq!(client.post(&url).form(&form).send().unwrap().status(), 403);
        form.push(("csrf", csrf(&html)));
        let response = client.post(&url).form(&form).send().unwrap();
        assert_eq!(
            response.status(),
            303,
            "{action}: {}",
            response.text().unwrap()
        );
        let mut command = origin_command(cli_home.path());
        command.args(["bug-origins", issue, flag]);
        if action == "correct" {
            command.arg("24");
        }
        let output = command.arg("--json").output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            cli(home.path(), &["bug-origins", issue, "--json"]),
            expected
        );
    }
    let html = console.get("/bug-origins").text().unwrap();
    assert!(html.contains("confirmed"));
    assert!(html.contains("rejected"));
    assert!(html.contains("corrected"));
    assert!(html.contains("code-review"));
    assert!(html.contains("https://github.com/acme/widgets/pull/24"));
}

#[test]
fn scores_commit_five_human_scores_before_revealing_the_model_and_pair_with_cli() {
    let home = tempfile::tempdir().unwrap();
    fixture(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let response = console.get("/scores");
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    assert!(html.contains("2026-W43"));
    assert!(html.contains(PR));
    assert!(!html.contains("Model secret"));
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let url = format!("{}/scores/mine", console.address);
    let mut form = vec![("pull_request".to_owned(), PR.to_owned())];
    assert_eq!(client.post(&url).form(&form).send().unwrap().status(), 403);
    form.push(("csrf".into(), csrf(&html).into()));
    for criterion in CRITERIA.iter().take(4) {
        form.push(((*criterion).into(), "pass".into()));
        form.push((format!("{criterion}_reason"), "My reason".into()));
    }
    assert_eq!(client.post(&url).form(&form).send().unwrap().status(), 400);
    assert!(cli(home.path(), &["scores", PR, "--json"])[0]["sample"][0]["mine"].is_null());
    assert!(
        !console
            .get("/scores")
            .text()
            .unwrap()
            .contains("Model secret")
    );
    form.push(("Standards".into(), "fail".into()));
    form.push(("Standards_reason".into(), "   ".into()));
    assert_eq!(client.post(&url).form(&form).send().unwrap().status(), 400);
    form.pop();
    form.push((
        "Standards_reason".into(),
        "My <script>reason</script>".into(),
    ));
    let lock = fs::File::create(home.path().join(".thirdshift/scoring.lock")).unwrap();
    lock.lock().unwrap();
    assert_eq!(client.post(&url).form(&form).send().unwrap().status(), 400);
    assert!(cli(home.path(), &["scores", PR, "--json"])[0]["sample"][0]["mine"].is_null());
    drop(lock);
    assert_eq!(client.post(&url).form(&form).send().unwrap().status(), 303);
    let saved = cli(home.path(), &["scores", PR, "--json"]);
    assert_eq!(saved[0]["sample"][0]["mine"]["scores"][4]["passed"], false);
    assert_eq!(
        saved[0]["sample"][0]["assessment"]["scores"][0]["calibration"]["compared"],
        1
    );
    let html = console.get("/scores").text().unwrap();
    assert!(html.contains("Model secret &#60;script&#62;"));
    assert!(html.contains("My &#60;script&#62;reason&#60;/script&#62;"));
    assert!(!html.contains("<script>unsafe()"));
    let html = console.get("/scores?mine=23").text().unwrap();
    assert!(!html.contains("Model secret"));
    assert!(html.contains("Save all five Scores"));

    let scenario = support::Scenario::new();
    fixture(&scenario.path("home"));
    let entered = scenario.run_on_terminal(
        &["scores", PR, "--mine"],
        &[],
        &[
            ("Correct (pass/fail):", "pass"),
            ("Correct reason:", "My reason"),
            ("Scoped (pass/fail):", "pass"),
            ("Scoped reason:", "My reason"),
            ("Tested (pass/fail):", "pass"),
            ("Tested reason:", "My reason"),
            ("Honest (pass/fail):", "pass"),
            ("Honest reason:", "My reason"),
            ("Standards (pass/fail):", "fail"),
            ("Standards reason:", "My <script>reason</script>"),
        ],
    );
    assert_eq!(entered.code, Some(0), "{}", entered.stderr);
    let cli_saved = cli(&scenario.path("home"), &["scores", PR, "--json"]);
    assert_eq!(
        cli_saved[0]["sample"][0]["mine"]["scores"],
        saved[0]["sample"][0]["mine"]["scores"]
    );
}
