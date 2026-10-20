//! Console choices through Setup's existing outside-world script.
use super::scripted::{Call, Scripted};
use super::*;

const HOME: &str = "/home/me";
const PATH: &str = "/home/me/.thirdshift/config.toml";
const NO_NOTIFICATIONS: &[(&str, &str)] = &[
    ("Merge run?", ""),
    ("fast-forwards", ""),
    ("Security runs may fix", ""),
    ("Runs review", ""),
    ("Run notifications", "n"),
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
    let mut answers = NO_NOTIFICATIONS[..4].to_vec();
    answers.extend([
        ("Run notifications", "y"),
        ("Send Run notifications to", "new@example.com"),
        ("Send them from", ""),
        ("Resend API key", ""),
        ("Run the Console?", "y"),
        ("Console's whole address", "thirdshift.example.com"),
        ("Allowed emails", ""),
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
fn eof_during_console_questions_saves_no_config_credentials_or_test_email() {
    let mut answers = NO_NOTIFICATIONS[..4].to_vec();
    answers.extend([
        ("Run notifications", "y"),
        ("Send Run notifications to", "owner@example.com"),
        ("Send them from", ""),
        ("Resend API key", "re_secret"),
        ("Send a test email", "y"),
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
