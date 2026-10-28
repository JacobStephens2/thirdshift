//! Console choices through Setup's existing outside-world script.
use super::scripted::{Call, Scripted};
use super::*;

const HOME: &str = "/home/me";
const PATH: &str = "/home/me/.thirdshift/config.toml";
const NO_NOTIFICATIONS: &[(&str, &str)] = &[
    ("Self-merges?", ""),
    ("Every Ticket run may start a Base fix", ""),
    ("fast-forwards", ""),
    ("Security runs may fix", ""),
    ("Ticket runs review", ""),
    ("Run notifications", "n"),
    ("Claude setup token", ""),
];

fn setup(outside: &mut Scripted) {
    run_setup(outside, Path::new(HOME), Path::new(PATH)).unwrap();
    assert!(outside.answers.is_empty(), "{:?}", outside.answers);
}

fn config(outside: &Scripted) -> UserConfig {
    UserConfig::parse(
        outside.user_config.as_deref().unwrap(),
        Path::new(PATH),
        Path::new(HOME),
    )
    .unwrap()
}

#[test]
fn console_allowlist_defaults_to_email_to_even_with_notifications_off() {
    let mut answers = NO_NOTIFICATIONS.to_vec();
    answers.extend([
        ("Run the Console?", "y"),
        ("Console's whole address", "thirdshift.example.com"),
        ("Allowed emails", ""),
        ("Other local accounts", ""),
    ]);
    let mut outside = Scripted {
        user_config: Some("[email]\nto = 'owner@example.com'\n".into()),
        ..Scripted::answering(&answers)
    };
    setup(&mut outside);
    assert_eq!(
        config(&outside).console.allowed_emails,
        ["owner@example.com"]
    );
    assert!(
        outside
            .prompts()
            .iter()
            .any(|prompt| prompt.contains("[owner@example.com]"))
    );
}

#[test]
fn console_allowlist_defaults_to_the_notification_address_just_chosen() {
    let mut answers = NO_NOTIFICATIONS[..5].to_vec();
    answers.extend([
        ("Run notifications", "y"),
        ("Send Run notifications to", "new@example.com"),
        ("Send them from", ""),
        ("Resend API key", ""),
        ("Claude setup token", ""),
        ("Run the Console?", "y"),
        ("Console's whole address", "thirdshift.example.com"),
        ("Allowed emails", ""),
        ("Other local accounts", ""),
    ]);
    let mut outside = Scripted {
        user_config: Some("[email]\nto = 'old@example.com'\n".into()),
        ..Scripted::answering(&answers)
    };
    setup(&mut outside);
    assert_eq!(config(&outside).console.allowed_emails, ["new@example.com"]);
}

#[test]
fn declining_console_retains_existing_access_and_offers_no_install() {
    for saved in [
        "",
        "[console]\naddress = 'saved.example.com' # site\nallowed_emails = ['saved@example.com'] # people\n",
    ] {
        let mut answers = NO_NOTIFICATIONS.to_vec();
        answers.push(("Run the Console?", "n"));
        let mut outside = Scripted {
            user_config: Some(saved.into()),
            ..Scripted::answering(&answers)
        };
        setup(&mut outside);
        let before = UserConfig::parse(saved, Path::new(PATH), Path::new(HOME)).unwrap();
        assert_eq!(config(&outside).console, before.console);
        assert!(outside.user_config.as_deref().unwrap().starts_with(saved));
        assert!(
            !outside
                .said()
                .iter()
                .any(|line| line.contains("console install"))
        );
        assert!(!outside.prompts().iter().any(|prompt| prompt.contains("Allowed emails") || prompt.contains("whole address")));
    }
}

#[test]
fn entering_existing_console_defaults_preserves_allowlist_layout_and_comments() {
    let mut answers = NO_NOTIFICATIONS.to_vec();
    answers.extend([
        ("Run the Console? [Y/n]", ""),
        ("[https://saved.example.com]", ""),
        ("[saved@example.com, other@example.com]", ""),
        ("Other local accounts", ""),
    ]);
    let saved = "[console] # Console\naddress = 'https://saved.example.com' # site\nallowed_emails = [\n  'saved@example.com', # owner\n  'other@example.com', # colleague\n] # people\n";
    let mut outside = Scripted {
        user_config: Some(saved.into()),
        ..Scripted::answering(&answers)
    };
    setup(&mut outside);
    assert!(outside.user_config.as_deref().unwrap().starts_with(saved));
}

#[test]
fn console_reasks_invalid_addresses_and_allowlists_without_using_the_example_as_a_default() {
    let mut answers = NO_NOTIFICATIONS.to_vec();
    answers.extend([
        ("Run the Console?", "y"),
        ("whole address (e.g. thirdshift.example.com):", ""),
        ("whole address", "http://bad.example.com"),
        ("whole address", "https://good.example.com/path"),
        ("whole address", "https://good.example.com"),
        ("Allowed emails", ""),
        ("Allowed emails", "missing-at"),
        ("Allowed emails", "bad @example.com"),
        ("Allowed emails", "owner@example.com,"),
        ("Allowed emails", "owner@example.com"),
        ("Other local accounts", ""),
    ]);
    let mut outside = Scripted::answering(&answers);
    setup(&mut outside);
    assert_eq!(config(&outside).console.address, "https://good.example.com");
    assert_eq!(
        config(&outside).console.allowed_emails,
        ["owner@example.com"]
    );
}

#[test]
fn rejected_console_sign_in_entries_are_named_and_reasked_before_writing() {
    for (answer, rejected) in [
        (
            "jstephens@etadoichuk@etadventures.com",
            "jstephens@etadoichuk@etadventures.com",
        ),
        ("@example.com", "@example.com"),
        ("owner@", "owner@"),
        ("owner@localhost", "owner@localhost"),
        ("owner@.com", "owner@.com"),
        ("a@b.com c@d.com", "a@b.com c@d.com"),
        ("a@b.com,", ""),
        (",a@b.com", ""),
        ("a@b.com, ,c@d.com", ""),
    ] {
        let mut answers = NO_NOTIFICATIONS.to_vec();
        answers.extend([
            ("Run the Console?", "y"),
            ("whole address", "thirdshift.example.com"),
            ("Allowed emails", answer),
        ]);
        let mut outside = Scripted::answering(&answers);
        let error = run_setup(&mut outside, Path::new(HOME), Path::new(PATH)).unwrap_err();
        assert!(error.to_string().contains("nothing written"), "{error}");
        assert!(outside.user_config.is_none());
        assert!(outside.effects().is_empty(), "{:?}", outside.effects());
        let expected = format!("console.allowed_emails: {rejected:?} is not an email address");
        assert!(
            outside.said().contains(&expected.as_str()),
            "{:?}",
            outside.said()
        );

        answers.push(("Allowed emails", "a@b.com, c@d.com"));
        answers.push(("Other local accounts", ""));
        let mut outside = Scripted::answering(&answers);
        setup(&mut outside);
        assert_eq!(
            config(&outside).console.allowed_emails,
            ["a@b.com", "c@d.com"]
        );
        assert!(
            outside.said().contains(&expected.as_str()),
            "{:?}",
            outside.said()
        );
        assert_eq!(
            outside
                .prompts()
                .iter()
                .filter(|prompt| prompt.contains("Allowed emails"))
                .count(),
            2
        );
    }
}

#[test]
fn eof_during_console_questions_saves_no_config_credentials_or_test_email() {
    let mut answers = NO_NOTIFICATIONS[..5].to_vec();
    answers.extend([
        ("Run notifications", "y"),
        ("Send Run notifications to", "owner@example.com"),
        ("Send them from", ""),
        ("Resend API key", "re_secret"),
        ("Send a test email", "y"),
        ("Claude setup token", ""),
        ("Run the Console?", "y"),
        ("whole address", "https://good.example.com"),
    ]);
    for end in answers.len() - 2..=answers.len() {
        let mut outside = Scripted::answering(&answers[..end]);
        let error = run_setup(&mut outside, Path::new(HOME), Path::new(PATH)).unwrap_err();
        assert!(error.to_string().contains("nothing written"));
        assert!(outside.user_config.is_none());
        assert!(
            !outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::SaveKey(_) | Call::SendTest { .. }))
        );
    }
}

#[test]
fn first_run_setup_also_saves_console_access_and_offers_install() {
    let mut answers = vec![("Set your defaults now?", "y")];
    answers.extend_from_slice(NO_NOTIFICATIONS);
    answers.extend([
        ("Run the Console?", "y"),
        ("whole address", "thirdshift.example.com"),
        ("Allowed emails", "owner@example.com"),
        ("Other local accounts", ""),
    ]);
    let mut outside = Scripted::answering(&answers);
    run_offer(&mut outside, Path::new(HOME), Path::new(PATH)).unwrap();
    assert!(outside.answers.is_empty());
    assert_eq!(config(&outside).console.address, "thirdshift.example.com");
    assert!(
        outside
            .said()
            .iter()
            .any(|line| line.contains("thirdshift console install"))
    );
}

#[test]
fn review_changed_console_allowlist_keeps_entry_comments() {
    let mut answers = NO_NOTIFICATIONS.to_vec();
    answers.extend([
        ("Run the Console?", "y"),
        ("Console's whole address", ""),
        ("Allowed emails", "saved@example.com, added@example.net"),
        ("Other local accounts", ""),
    ]);
    let saved = "[console] # Console\naddress = 'https://saved.example.com' # site\nallowed_emails = [\n  'saved@example.com', # owner retained\n] # people\n";
    let mut outside = Scripted {
        user_config: Some(saved.into()),
        ..Scripted::answering(&answers)
    };
    setup(&mut outside);
    assert_eq!(
        config(&outside).console.allowed_emails,
        ["saved@example.com", "added@example.net"]
    );
    let text = outside.user_config.as_deref().unwrap();
    for comment in ["# Console", "# site", "# owner retained", "# people"] {
        assert!(text.contains(comment), "lost {comment}:\n{text}");
    }
}

#[test]
fn replacing_or_removing_allowlist_entries_keeps_the_comments() {
    for (answer, expected) in [
        (
            "new@example.com, other@example.com",
            vec!["new@example.com", "other@example.com"],
        ),
        ("saved@example.com", vec!["saved@example.com"]),
    ] {
        let mut answers = NO_NOTIFICATIONS.to_vec();
        answers.extend([
            ("Run the Console?", "y"),
            ("whole address", ""),
            ("Allowed emails", answer),
            ("Other local accounts", ""),
        ]);
        let mut outside = Scripted {
            user_config: Some("[console]\naddress = 'saved.example.com'\nallowed_emails = [\n  'saved@example.com', # owner\n  'other@example.com', # colleague\n] # people\n".into()),
            ..Scripted::answering(&answers)
        };
        setup(&mut outside);
        assert_eq!(config(&outside).console.allowed_emails, expected);
        let text = outside.user_config.as_deref().unwrap();
        for comment in ["# owner", "# colleague", "# people"] {
            assert!(text.contains(comment), "lost {comment}:\n{text}");
        }
    }
}
