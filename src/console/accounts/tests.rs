use super::*;

fn signed_in(accounts: &mut Accounts) -> String {
    let SignIn::SignedIn(token) = accounts.sign_in("owner@example.test", "password").unwrap()
    else {
        panic!("sign-in refused")
    };
    token
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
