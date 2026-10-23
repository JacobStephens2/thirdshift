use super::*;

fn signed_in(accounts: &mut Accounts) -> String {
    let SignIn::SignedIn(token) = accounts.sign_in("owner@example.test", "password").unwrap()
    else {
        panic!("sign-in refused")
    };
    token
}

#[test]
fn email_codes_expire_at_fifteen_minutes_and_allow_exactly_five_tries() {
    let allowed = vec!["owner@example.test".into()];
    for (elapsed, wrong_tries, accepted) in [(899, 4, true), (900, 0, false), (1, 5, false)] {
        let dir = tempfile::tempdir().unwrap();
        let mut accounts = Accounts::open(dir.path(), &allowed, 1_000).unwrap();
        let mut code = String::new();
        accounts
            .send_code("owner@example.test", |sent| {
                code = sent.into();
                Ok(())
            })
            .unwrap();
        drop(accounts);
        let wrong = if code == "000000" { "111111" } else { "000000" };
        for _ in 0..wrong_tries {
            let mut accounts = Accounts::open(dir.path(), &allowed, 1_000 + elapsed).unwrap();
            assert!(matches!(
                accounts
                    .set_password_by_code("owner@example.test", wrong, "password")
                    .unwrap(),
                SignIn::Refused
            ));
        }
        let mut accounts = Accounts::open(dir.path(), &allowed, 1_000 + elapsed).unwrap();
        assert_eq!(
            matches!(
                accounts
                    .set_password_by_code("owner@example.test", &code, "password")
                    .unwrap(),
                SignIn::SignedIn(_)
            ),
            accepted
        );
        assert!(matches!(
            accounts
                .set_password_by_code("owner@example.test", &code, "replay")
                .unwrap(),
            SignIn::Refused
        ));
    }
}

#[test]
fn code_send_limits_survive_restarts_and_password_changes_and_are_per_address() {
    let dir = tempfile::tempdir().unwrap();
    let allowed = vec!["owner@example.test".into(), "other@example.test".into()];
    let mut sends = 0;
    let mut code = String::new();
    for (time, expected) in [(1_000, 1), (1_059, 1), (1_060, 2)] {
        let mut accounts = Accounts::open(dir.path(), &allowed, time).unwrap();
        accounts
            .send_code("owner@example.test", |sent| {
                sends += 1;
                code = sent.into();
                Ok(())
            })
            .unwrap();
        assert_eq!(sends, expected);
    }
    {
        let mut accounts = Accounts::open(dir.path(), &allowed, 1_061).unwrap();
        assert!(matches!(
            accounts
                .set_password_by_code("owner@example.test", &code, "password")
                .unwrap(),
            SignIn::SignedIn(_)
        ));
    }
    for index in 2..11 {
        let mut accounts = Accounts::open(dir.path(), &allowed, 1_000 + index * 60).unwrap();
        accounts
            .send_code("owner@example.test", |_| {
                sends += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(sends, (index + 1).min(10));
    }
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_000 + 24 * 60 * 60 - 1).unwrap();
    accounts
        .send_code("owner@example.test", |_| panic!("daily limit bypassed"))
        .unwrap();
    accounts
        .send_code("unknown@example.test", |_| panic!("unknown address sent"))
        .unwrap();
    accounts
        .send_code("other@example.test", |_| {
            sends += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(sends, 11);
    drop(accounts);
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_000 + 24 * 60 * 60).unwrap();
    accounts
        .send_code("owner@example.test", |_| {
            sends += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(sends, 12, "first send ages out of the rolling day");
}

#[test]
fn five_wrong_passwords_slow_the_address_to_one_attempt_per_minute() {
    let dir = tempfile::tempdir().unwrap();
    let allowed = vec!["owner@example.test".into(), "other@example.test".into()];
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_000).unwrap();
    accounts
        .set_password("owner@example.test", "password")
        .unwrap();
    accounts
        .set_password("other@example.test", "password")
        .unwrap();
    for _ in 0..5 {
        assert!(matches!(
            accounts.sign_in("owner@example.test", "wrong").unwrap(),
            SignIn::Refused
        ));
    }
    assert!(
        !matches!(
            accounts.sign_in("owner@example.test", "password").unwrap(),
            SignIn::SignedIn(_)
        ),
        "sixth try must wait"
    );
    assert!(matches!(
        accounts.sign_in("other@example.test", "password").unwrap(),
        SignIn::SignedIn(_)
    ));
    drop(accounts);
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_059).unwrap();
    assert!(!matches!(
        accounts.sign_in("owner@example.test", "password").unwrap(),
        SignIn::SignedIn(_)
    ));
    drop(accounts);
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_060).unwrap();
    assert!(matches!(
        accounts.sign_in("owner@example.test", "wrong").unwrap(),
        SignIn::Refused
    ));
    assert!(
        !matches!(
            accounts.sign_in("owner@example.test", "password").unwrap(),
            SignIn::SignedIn(_)
        ),
        "each wrong try restarts the minute"
    );
    drop(accounts);
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_120).unwrap();
    assert!(matches!(
        accounts.sign_in("owner@example.test", "password").unwrap(),
        SignIn::SignedIn(_)
    ));
    assert!(
        matches!(
            accounts.sign_in("owner@example.test", "password").unwrap(),
            SignIn::SignedIn(_)
        ),
        "success resets the slowdown"
    );
}

#[test]
fn session_activity_is_saved_at_most_once_per_minute() {
    let dir = tempfile::tempdir().unwrap();
    let allowed = vec!["owner@example.test".into()];
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_000).unwrap();
    accounts
        .set_password("owner@example.test", "password")
        .unwrap();
    let token = signed_in(&mut accounts);
    drop(accounts);
    let path = dir.path().join("console-accounts.json");
    let before = fs::read(&path).unwrap();
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_059).unwrap();
    assert!(accounts.session(&token, true).unwrap().is_some());
    drop(accounts);
    assert!(
        fs::read(&path).unwrap() == before,
        "read-only activity rewrote the account store"
    );
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_060).unwrap();
    assert!(accounts.session(&token, true).unwrap().is_some());
    drop(accounts);
    assert!(fs::read(&path).unwrap() != before);
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_060 + LIFETIME - 1).unwrap();
    assert!(accounts.session(&token, false).unwrap().is_some());
    drop(accounts);
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_060 + LIFETIME).unwrap();
    assert!(accounts.session(&token, false).unwrap().is_none());
}

#[test]
fn sessions_expire_thirty_days_after_last_use_and_allowlist_removal_revokes_them() {
    let dir = tempfile::tempdir().unwrap();
    let allowed = vec!["owner@example.test".into()];
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_000).unwrap();
    accounts
        .set_password("owner@example.test", "password")
        .unwrap();
    let token = signed_in(&mut accounts);
    drop(accounts);
    let month = 30 * 24 * 60 * 60;
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_000 + month - 1).unwrap();
    assert!(accounts.session(&token, true).unwrap().is_some());
    drop(accounts);
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_000 + month * 2 - 2).unwrap();
    assert!(accounts.session(&token, false).unwrap().is_some());
    drop(accounts);
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_000 + month * 2 - 1).unwrap();
    assert!(
        accounts.session(&token, true).unwrap().is_none(),
        "expires at 30 days, even across restarts"
    );
    let token = signed_in(&mut accounts);
    drop(accounts);
    drop(Accounts::open(dir.path(), &[], 1_000 + month * 2).unwrap());
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_001 + month * 2).unwrap();
    assert!(
        accounts.session(&token, true).unwrap().is_none(),
        "re-adding the address must not revive sessions"
    );
    assert!(
        matches!(
            accounts.sign_in("owner@example.test", "password").unwrap(),
            SignIn::SignedIn(_)
        ),
        "the password is kept"
    );
}

#[test]
fn passwords_and_sessions_survive_reopening_and_password_changes_revoke_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let allowed = vec!["owner@example.test".into()];
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_000).unwrap();
    accounts
        .set_password("owner@example.test", "first password")
        .unwrap();
    assert!(matches!(
        accounts.sign_in("owner@example.test", "wrong").unwrap(),
        SignIn::Refused
    ));
    let SignIn::SignedIn(token) = accounts
        .sign_in("owner@example.test", "first password")
        .unwrap()
    else {
        panic!("sign-in refused")
    };
    drop(accounts);
    let mut accounts = Accounts::open(dir.path(), &allowed, 1_001).unwrap();
    assert_eq!(
        accounts.session(&token, true).unwrap().as_deref(),
        Some("owner@example.test")
    );
    accounts
        .set_password("owner@example.test", "new password")
        .unwrap();
    assert!(accounts.session(&token, true).unwrap().is_none());
}
