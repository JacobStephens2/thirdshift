//! Plan usage: running sessions record subscription Harnesses' windows and
//! limits in `~/.thirdshift/plan-usage.json`, `thirdshift plan-usage` adds
//! official non-billed reads at most once a minute and shows the User
//! config's routes, `--clear` forgets cooldowns, and headless agy keeps to
//! included usage unless its route allows credits.

mod support;

use std::os::unix::fs::PermissionsExt;

use serde_json::{Value, json};
use support::Scenario;

/// The agent commits its work and opens a PR.
const OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

/// A future Unix time, so a recorded reset is still ahead.
const RESET: i64 = 4_000_000_000;

/// Replace the fake `name` on the scenario's PATH with a bash script.
fn scripted(scenario: &Scenario, name: &str, body: &str) {
    let path = scenario.path("bin").join(name);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, format!("#!/bin/bash\n{body}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Take the fake `name` off the scenario's PATH.
fn uninstalled(scenario: &Scenario, name: &str) {
    std::fs::remove_file(scenario.path("bin").join(name)).unwrap();
}

fn plan_usage(scenario: &Scenario, args: &[&str]) -> Value {
    let mut all = vec!["plan-usage", "--json"];
    all.extend(args);
    let result = scenario.run(&all);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    serde_json::from_str(&result.stdout).unwrap()
}

fn row<'a>(view: &'a Value, harness: &str) -> &'a Value {
    view["harnesses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["harness"] == harness)
        .unwrap_or_else(|| panic!("no {harness} row: {view:#}"))
}

fn harnesses(view: &Value) -> Vec<&str> {
    view["harnesses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["harness"].as_str().unwrap())
        .collect()
}

fn window<'a>(row: &'a Value, name: &str) -> &'a Value {
    row["windows"]
        .as_array()
        .unwrap_or_else(|| panic!("no windows: {row:#}"))
        .iter()
        .find(|window| window["name"] == name)
        .unwrap_or_else(|| panic!("no {name} window: {row:#}"))
}

fn saved(scenario: &Scenario) -> Value {
    serde_json::from_slice(
        &std::fs::read(scenario.path("home/.thirdshift/plan-usage.json")).unwrap(),
    )
    .unwrap()
}

#[test]
fn sessions_record_claude_windows_at_every_utilization_and_codex_token_counts() {
    for (harness, events) in [
        (
            "claude",
            vec![
                json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed",
                    "rateLimitType": "five_hour", "utilization": 0.12, "resetsAt": RESET}}),
                json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed_warning",
                    "rateLimitType": "seven_day", "utilization": 0.91, "resetsAt": RESET}}),
            ],
        ),
        (
            "codex",
            vec![json!({"type": "token_count", "rate_limits": {
                "primary": {"used_percent": 7.0, "window_minutes": 300, "resets_at": RESET},
                "secondary": {"used_percent": 95.0, "window_minutes": 10080, "resets_at": RESET}
            }})],
        ),
    ] {
        let scenario = Scenario::new();
        let echoes: String = events
            .iter()
            .map(|event| format!("echo '{event}'\n"))
            .collect();
        scenario.agent_does(&format!("{echoes}{OPENS_PR}"));
        let result = scenario.run(&[&scenario.issue_url(7), "--harness", harness]);
        assert_eq!(result.code, Some(0), "{harness}: {}", result.stderr);

        let view = plan_usage(&scenario, &[]);
        let row = row(&view, harness);
        assert_eq!(row["source"], "session", "{harness}: {row:#}");
        assert!(row["cooldown"].is_null(), "{harness}: {row:#}");
        if harness == "claude" {
            assert_eq!(window(row, "five_hour")["used_percent"], json!(12.0));
            assert_eq!(window(row, "five_hour")["status"], "allowed");
            assert_eq!(window(row, "seven_day")["used_percent"], json!(91.0));
        } else {
            assert_eq!(window(row, "primary")["used_percent"], json!(7.0));
            assert_eq!(window(row, "primary")["duration_minutes"], json!(300));
            assert_eq!(window(row, "secondary")["used_percent"], json!(95.0));
        }
    }
}

#[test]
fn terminal_usage_limits_record_a_cooldown_on_every_subscription_harness() {
    let claude = json!({"type": "rate_limit_event", "rate_limit_info": {"status": "rejected",
        "rateLimitType": "seven_day", "resetsAt": RESET, "overageStatus": "rejected"}});
    let grok = json!({"type": "result", "subtype": "error_during_execution", "is_error": true,
        "errors": ["usage limit reached for your plan"]});
    let muse = json!({"payload_type": "run.terminal.failed", "payload": {"reason": "usage limit exceeded"}});
    let agy =
        json!({"event": "result", "result": {"status": "ERROR", "error": "usage limit exceeded"}});
    for (harness, script) in [
        ("claude", format!("echo '{claude}'\nexit 1\n")),
        (
            "codex",
            "echo \"You've hit your usage limit.\" > \"$FAKE_CODEX_ERROR\"\nexit 1\n".to_string(),
        ),
        ("grok", format!("echo '{grok}'\nexit 1\n")),
        ("muse", format!("echo '{muse}'\nexit 1\n")),
        ("agy", format!("echo '{agy}'\nexit 1\n")),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(&script);
        let result = scenario.run(&[&scenario.issue_url(7), "--harness", harness]);
        assert_ne!(result.code, Some(0), "{harness}: {}", result.stderr);

        let saved = saved(&scenario);
        let cooldown = &saved["harnesses"][harness]["cooldown"];
        assert!(cooldown.is_object(), "{harness}: {saved:#}");
        if harness == "claude" {
            assert_eq!(cooldown["window"], "seven_day");
            assert_eq!(cooldown["overage"], "rejected");
            assert_eq!(cooldown["until"], "2096-10-02T07:06:40Z");
            assert!(cooldown["backoff_minutes"].is_null());
        } else {
            assert_eq!(
                cooldown["backoff_minutes"],
                json!(60),
                "{harness}: {saved:#}"
            );
        }
        let view = plan_usage(&scenario, &[]);
        assert!(
            row(&view, harness)["cooldown"].is_object(),
            "{harness}: {view:#}"
        );
    }
}

/// A fake `codex app-server` and `muse serve` answering one JSON-RPC
/// request per line, and a `claude auth status --json`, each counting its
/// reads in `reads.<cli>`.
fn readable_fakes(scenario: &Scenario) {
    let reads = scenario.path("reads");
    let codex = json!({"id": 1, "result": {
        "accountId": "acct-7", "planType": "pro", "ordinaryUsageAllowed": true,
        "rateLimits": {
            "primary": {"usedPercent": 87, "windowDurationMins": 10080, "resetsAt": RESET},
            "secondary": {"usedPercent": 20, "windowDurationMins": 300, "resetsAt": RESET}
        },
        "credits": {"hasCredits": true, "balance": "28719.46"}
    }});
    scripted(
        scenario,
        "codex",
        &format!(
            "[ \"$1\" = app-server ] || exit 2\necho read >> '{reads}.codex'\n\
             while read -r line; do\n  case \"$line\" in\n    *account/rateLimits/read*) echo '{codex}' ;;\n  esac\ndone\n",
            reads = reads.display()
        ),
    );
    let usage = json!({"id": 1, "result": {"usage": {"tier": "pro", "observedAtMs": 1_700_000_000_000_i64,
        "window": {"usedPercent": 30, "resetsAtMs": RESET * 1000, "windowDurationMins": 300},
        "weekly": {"usedPercent": 64, "resetsAtMs": RESET * 1000}}}});
    let account = json!({"id": 2, "result": {"state": "ready", "label": "muse-me", "credentialRequired": false}});
    scripted(
        scenario,
        "muse",
        &format!(
            "[ \"$1\" = serve ] || exit 2\necho read >> '{reads}.muse'\n\
             while read -r line; do\n  case \"$line\" in\n    *usage/read*) echo '{usage}' ;;\n    *account/read*) echo '{account}' ;;\n  esac\ndone\n",
            reads = reads.display()
        ),
    );
    let status = json!({"loggedIn": true, "authMethod": "claude.ai", "subscriptionType": "max",
        "planTier": "default_claude_max_20x", "orgId": "org-7", "email": "me@example.com"});
    scripted(
        scenario,
        "claude",
        &format!(
            "[ \"$*\" = 'auth status --json' ] || exit 2\necho read >> '{reads}.claude'\necho '{status}'\n",
            reads = reads.display()
        ),
    );
}

fn reads(scenario: &Scenario, cli: &str) -> usize {
    std::fs::read_to_string(scenario.path(&format!("reads.{cli}")))
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

#[test]
fn plan_usage_reads_codex_muse_and_claude_on_demand_and_reuses_them_for_a_minute() {
    let scenario = Scenario::new();
    readable_fakes(&scenario);

    let view = plan_usage(&scenario, &[]);
    assert_eq!(harnesses(&view), ["claude", "codex", "muse"], "{view:#}");
    let claude = row(&view, "claude");
    assert_eq!(claude["active"], true);
    assert_eq!(claude["account"]["plan"], "max");
    assert_eq!(claude["account"]["tier"], "default_claude_max_20x");
    assert_eq!(claude["account"]["label"], "me@example.com");
    assert_eq!(claude["account"]["organization"], "org-7");
    assert_eq!(claude["account"]["signed_in"], true);
    let codex = row(&view, "codex");
    assert_eq!(codex["source"], "codex app-server");
    assert_eq!(codex["account"]["plan"], "pro");
    assert_eq!(codex["account"]["id"], "acct-7");
    assert_eq!(codex["ordinary_usage_allowed"], true);
    assert_eq!(codex["credits"]["balance"], "28719.46");
    assert_eq!(window(codex, "primary")["used_percent"], json!(87.0));
    assert_eq!(window(codex, "primary")["duration_minutes"], json!(10080));
    let muse = row(&view, "muse");
    assert_eq!(muse["source"], "muse serve");
    assert_eq!(muse["account"]["plan"], "pro");
    assert_eq!(muse["account"]["label"], "muse-me");
    assert_eq!(window(muse, "weekly")["used_percent"], json!(64.0));
    assert_eq!(muse["observed_at"], "2023-11-14T22:13:20Z");
    for cli in ["codex", "muse", "claude"] {
        assert_eq!(reads(&scenario, cli), 1, "{cli}");
    }

    // A second view within the minute reads nothing again.
    let text = scenario.run(&["plan-usage"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(
        text.stdout.starts_with("claude (active)\n"),
        "{}",
        text.stdout
    );
    assert!(
        text.stdout.contains("  primary: 87% used"),
        "{}",
        text.stdout
    );
    assert!(
        text.stdout.contains("  credits: 28719.46"),
        "{}",
        text.stdout
    );
    for cli in ["codex", "muse", "claude"] {
        assert_eq!(reads(&scenario, cli), 1, "{cli}");
    }

    // Once the saved reads are a minute old they run again.
    let path = scenario.path("home/.thirdshift/plan-usage.json");
    let mut saved = saved(&scenario);
    for cli in ["codex", "muse", "claude"] {
        saved["harnesses"][cli]["read_at"] = json!("2020-01-01T00:00:00Z");
    }
    std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    plan_usage(&scenario, &[]);
    for cli in ["codex", "muse", "claude"] {
        assert_eq!(reads(&scenario, cli), 2, "{cli}");
    }
}

#[test]
fn plan_usage_shows_configured_routes_in_order_and_omits_api_key_and_unused_harnesses() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "[harness]\ndefault = \"codex\"\n\
         routes = [\"codex\", { harness = \"agy\", model = \"gemini-3.8-flash-high\", max_utilization = 80 }, \"opencode\"]\n\
         max_utilization = 90\n\
         [security]\nharness = \"grok\"\nroutes = [\"claude\", \"kimi\"]\n",
    );
    uninstalled(&scenario, "agy");
    // Claude Code is signed out; Muse has no read and nothing observed.
    let result = scenario.run_with_env(
        &["plan-usage", "--json"],
        &[("FAKE_CLAUDE_SIGNED_OUT", "1")],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(
        harnesses(&view),
        ["codex", "agy", "grok", "claude"],
        "{view:#}"
    );

    let codex = row(&view, "codex");
    assert_eq!(codex["active"], true);
    let settings: Vec<&str> = codex["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|route| route["setting"].as_str().unwrap())
        .collect();
    assert_eq!(settings, ["harness.default", "harness.routes"]);
    assert_eq!(codex["routes"][1]["position"], 1);
    assert_eq!(codex["routes"][1]["active"], true);
    assert_eq!(codex["routes"][1]["max_utilization"], 90);

    let agy = row(&view, "agy");
    assert_eq!(agy["installed"], false);
    assert_eq!(agy["routes"][0]["position"], 2);
    assert_eq!(agy["routes"][0]["model"], "gemini-3.8-flash-high");
    assert_eq!(agy["routes"][0]["max_utilization"], 80);
    assert_eq!(agy["routes"][0]["availability"], "not_installed");

    assert_eq!(
        row(&view, "grok")["routes"][0]["setting"],
        "security.harness"
    );
    let claude = row(&view, "claude");
    assert_eq!(claude["routes"][0]["setting"], "security.routes");
    assert_eq!(claude["routes"][0]["availability"], "signed_out");

    let text = scenario.run_with_env(&["plan-usage"], &[("FAKE_CLAUDE_SIGNED_OUT", "1")]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    for expected in [
        "codex (active)\n  route: harness.default, ceiling 90%: available\n  route: harness.routes 1, ceiling 90%: available, active\n",
        "agy\n  route: harness.routes 2, gemini-3.8-flash-high, ceiling 80%: not installed\n",
        "  route: security.routes 1, ceiling 90%: signed out\n",
    ] {
        assert!(
            text.stdout.contains(expected),
            "{expected}\n---\n{}",
            text.stdout
        );
    }
    for omitted in ["opencode", "kimi", "muse"] {
        assert!(!text.stdout.contains(omitted), "{omitted}: {}", text.stdout);
    }
}

#[test]
fn plan_usage_clear_forgets_cooldowns_for_one_harness_or_all() {
    let scenario = Scenario::new();
    let cooldown = json!({"since": "2026-01-01T00:00:00Z", "until": "2099-01-01T00:00:00Z", "backoff_minutes": 120});
    let path = scenario.path("home/.thirdshift/plan-usage.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        json!({"harnesses": {
            "agy": {"observed_at": "2026-01-01T00:00:00Z", "cooldown": cooldown},
            "grok": {"observed_at": "2026-01-01T00:00:00Z", "cooldown": cooldown}
        }})
        .to_string(),
    )
    .unwrap();

    let result = scenario.run(&["plan-usage", "--clear", "agy"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        result.stdout,
        "Cleared recorded Plan usage cooldowns and revalidation for agy\n"
    );
    let kept = saved(&scenario);
    assert!(kept["harnesses"]["agy"]["cooldown"].is_null(), "{kept:#}");
    assert_eq!(
        kept["harnesses"]["agy"]["observed_at"],
        "2026-01-01T00:00:00Z"
    );
    assert!(
        kept["harnesses"]["grok"]["cooldown"].is_object(),
        "{kept:#}"
    );

    let cleared = plan_usage(&scenario, &["--clear"]);
    assert_eq!(
        cleared,
        json!({"version": 1, "cleared": ["claude", "codex", "muse", "agy", "grok"]})
    );
    assert!(saved(&scenario)["harnesses"]["grok"]["cooldown"].is_null());

    for args in [
        &["plan-usage", "--clear", "opencode"][..],
        &["plan-usage", "--clear", "agy", "grok"],
        &["plan-usage", "--fleet"],
    ] {
        let refused = scenario.run(args);
        assert_eq!(refused.code, Some(2), "{args:?}: {}", refused.stdout);
    }
    let refused = scenario.run(&["plan-usage", "--clear", "kimi"]);
    assert!(
        refused.stderr.contains("not a subscription Harness"),
        "{}",
        refused.stderr
    );
}

#[test]
fn user_config_routes_and_ceilings_are_validated_and_listed_with_their_defaults() {
    let scenario = Scenario::new();
    let listed = scenario.run(&["config", "--json"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
    let setting = |key: &str| {
        listed["settings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|setting| setting["key"] == key)
            .unwrap_or_else(|| panic!("no {key}"))
            .clone()
    };
    assert_eq!(setting("harness.routes")["default"], json!([]));
    assert_eq!(setting("harness.max_utilization")["default"], json!(100));
    assert_eq!(setting("security.routes")["default"], json!([]));

    let set = scenario.run(&[
        "config",
        "harness.routes",
        r#"["claude", { harness = "agy", model = "gemini-3.8-flash-high", max_utilization = 85, allow_credits = true }]"#,
        "--json",
    ]);
    assert_eq!(set.code, Some(0), "{}", set.stderr);
    let set: Value = serde_json::from_str(&set.stdout).unwrap();
    assert_eq!(set["value"][1]["allow_credits"], true);

    for (key, value, expected) in [
        (
            "harness.max_utilization",
            "0",
            "harness.max_utilization must be a whole percentage from 1 to 100",
        ),
        ("harness.max_utilization", "101", "from 1 to 100"),
        ("harness.routes", r#"["gemini"]"#, "harness.routes entry 1"),
        (
            "security.routes",
            r#"[{ harness = "codex", allow_credits = 1 }]"#,
            "allow_credits must be true or false",
        ),
        (
            "security.routes",
            r#"[{ harness = "codex", ceiling = 80 }]"#,
            "unknown key ceiling",
        ),
    ] {
        let refused = scenario.run(&["config", key, value]);
        assert_eq!(refused.code, Some(1), "{key} {value}: {}", refused.stdout);
        assert!(
            refused.stderr.contains(expected),
            "{key} {value}: {}",
            refused.stderr
        );
    }
}

fn agy_settings(scenario: &Scenario) -> Vec<Option<String>> {
    scenario
        .agy_calls()
        .iter()
        .map(|call| {
            let argv: Vec<&str> = call["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap())
                .collect();
            argv.iter()
                .position(|arg| *arg == "--settings")
                .map(|at| argv[at + 1].to_string())
        })
        .collect()
}

#[test]
fn headless_agy_keeps_to_included_usage_unless_its_route_allows_credits() {
    let guarded = Some(r#"{"useG1Credits":false}"#.to_string());
    for (config, args, expected) in [
        ("", &["--harness", "agy"][..], guarded.clone()),
        (
            "[harness]\ndefault = \"agy\"\nroutes = [{ harness = \"agy\" }]\n",
            &[][..],
            guarded.clone(),
        ),
        (
            "[harness]\ndefault = \"agy\"\nroutes = [{ harness = \"agy\", allow_credits = true }]\n",
            &[][..],
            None,
        ),
        // A Harness named on the command stays on included usage.
        (
            "[harness]\nroutes = [{ harness = \"agy\", allow_credits = true }]\n",
            &["--harness", "agy"][..],
            guarded.clone(),
        ),
    ] {
        let scenario = Scenario::new();
        if !config.is_empty() {
            scenario.user_config_is(config);
        }
        scenario.agent_does(OPENS_PR);
        let issue = scenario.issue_url(7);
        let mut all = vec![issue.as_str()];
        all.extend(args);
        let result = scenario.run(&all);
        assert_eq!(result.code, Some(0), "{config}: {}", result.stderr);
        let settings = agy_settings(&scenario);
        assert!(!settings.is_empty(), "{config}");
        assert!(
            settings.iter().all(|setting| *setting == expected),
            "{config} {args:?}: {settings:?}"
        );
    }
}
