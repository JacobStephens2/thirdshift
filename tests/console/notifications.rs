use super::*;

const SUBJECT: &str = "[thirdshift] Result <script>";
const BODY: &str = "Ticket outcomes:\n#7 ready\nhttps://github.com/acme/widgets/pull/10\n<script>alert(1)</script>\n";

#[test]
fn saved_notifications_are_shared_by_command_details_and_cli_for_every_run_kind() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let kinds = [
        "Run",
        "Spec run",
        "Triage run",
        "Security run",
        "Architect run",
        "Pickup run",
    ];
    for (index, kind) in kinds.iter().enumerate() {
        let number = index as u64 + 1;
        let mut value = record(home.path(), number, Some("failed"), 0);
        value["kind"] = json!(kind);
        write_record(home.path(), number, &value);
        fs::write(
            dir.join(format!("{number}-fixture.notification.txt")),
            format!("Subject: {SUBJECT}\n\n{BODY}"),
        )
        .unwrap();
    }
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    for number in 1..=kinds.len() {
        let selector = dir
            .join(format!("{number}-fixture.command.json"))
            .to_string_lossy()
            .into_owned();
        let route = reqwest::Url::parse_with_params(
            &format!("{}/api/v1/peek", console.address),
            &[("command", &selector)],
        )
        .unwrap();
        let details = api::json(reqwest::blocking::get(route).unwrap());
        assert_eq!(
            details["command"]["notification"],
            json!({"subject": SUBJECT, "body": BODY}),
            "{}",
            kinds[number - 1]
        );
        assert_eq!(
            api::cli(home.path(), &["show", &selector, "--json"]),
            details
        );
        let shown = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .env("HOME", home.path())
            .args(["show", &selector])
            .output()
            .unwrap();
        assert!(shown.status.success());
        assert!(
            String::from_utf8(shown.stdout)
                .unwrap()
                .contains(&format!("Subject: {SUBJECT}\n\n{BODY}"))
        );
        let route = reqwest::Url::parse_with_params(
            &format!("{}/peek", console.address),
            &[("command", &selector)],
        )
        .unwrap();
        let html = reqwest::blocking::get(route).unwrap().text().unwrap();
        assert!(html.contains("aria-label=\"Run notification\""), "{html}");
        assert!(html.contains("Result &#60;script&#62;"), "{html}");
        assert!(
            html.contains("<a href=\"https://github.com/acme/widgets/pull/10\""),
            "{html}"
        );
        assert!(!html.contains("<script>alert(1)</script>"));
    }
    let inventory = api::json(console.get("/api/v1/logs"));
    assert!(!inventory.to_string().contains("notification.txt"));
}

#[test]
fn unavailable_notifications_leave_command_details_available() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 7, Some("failed"), 0);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let path = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/7-fixture.notification.txt");
    let absent = || {
        let document = api::json(console.get("/api/v1/peek?command=widgets%237"));
        assert!(
            document["command"].get("notification").unwrap().is_null(),
            "{document}"
        );
        let html = console.get("/peek?command=widgets%237").text().unwrap();
        assert!(!html.contains("aria-label=\"Run notification\""), "{html}");
    };
    absent();
    fs::write(&path, "malformed notification").unwrap();
    absent();
    fs::write(&path, [0xff, 0xfe]).unwrap();
    absent();
    fs::write(
        &path,
        format!("Subject: oversized\n\n{}", "x".repeat(4 * 1024 * 1024)),
    )
    .unwrap();
    absent();
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    absent();
}
