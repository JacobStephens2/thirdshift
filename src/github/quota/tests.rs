use super::*;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use store::Account;
use wait::{WaitBudget, WaitCapExceeded};

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z")
        .unwrap()
        .to_utc()
}

fn quota(remaining: u64) -> Quota {
    Quota {
        remaining,
        reset_at: now() + chrono::Duration::hours(1),
    }
}

#[test]
fn waits_are_isolated_by_host_user_and_resource_and_expire_at_reset() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("quota.json"));
    let alice = Account::new("github.com", "Alice");
    let bob = Account::new("github.com", "Bob");
    let enterprise = Account::new("git.example.com", "Alice");
    store
        .observe(&alice, Resource::Core, quota(499), now())
        .unwrap();
    store
        .observe(&bob, Resource::Graphql, quota(123), now())
        .unwrap();
    store
        .observe(&enterprise, Resource::Core, quota(42), now())
        .unwrap();
    let waits = store
        .waiting(&Account::new("GITHUB.COM", "ALICE"), now())
        .unwrap();
    assert_eq!(waits.len(), 1);
    assert_eq!(waits[0].resource, Resource::Core);
    assert_eq!(waits[0].remaining(), 499);
    assert_eq!(waits[0].reset_at(), now() + chrono::Duration::hours(1));
    assert_eq!(
        store.waiting(&bob, now()).unwrap()[0].resource,
        Resource::Graphql
    );
    assert_eq!(
        store.waiting(&enterprise, now()).unwrap()[0]
            .quota
            .remaining,
        42
    );
    assert!(
        store
            .waiting(&alice, now() + chrono::Duration::hours(1))
            .unwrap()
            .is_empty()
    );
    store
        .observe(&alice, Resource::Core, quota(500), now())
        .unwrap();
    assert!(store.waiting(&alice, now()).unwrap().is_empty());
    assert_eq!(store.waiting(&bob, now()).unwrap().len(), 1);
}

#[test]
fn concurrent_writers_preserve_each_other_and_readers_only_see_complete_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("quota.json");
    let barrier = std::sync::Barrier::new(17);
    std::thread::scope(|scope| {
        for user in 0..16 {
            let path = &path;
            let barrier = &barrier;
            scope.spawn(move || {
                let store = Store::new(path.clone());
                let account = Account::new("github.com", &format!("user{user}"));
                barrier.wait();
                for resource in [Resource::Core, Resource::Graphql] {
                    store
                        .observe(&account, resource, quota(100 + user), now())
                        .unwrap();
                }
            });
        }
        barrier.wait();
        let store = Store::new(path.clone());
        for _ in 0..100 {
            store
                .waiting(&Account::new("github.com", "user0"), now())
                .unwrap();
        }
    });
    let store = Store::new(path);
    for user in 0..16 {
        let waits = store
            .waiting(&Account::new("github.com", &format!("user{user}")), now())
            .unwrap();
        assert_eq!(waits.len(), 2);
        assert!(waits.iter().all(|wait| wait.quota.remaining == 100 + user));
    }
}

#[test]
fn quota_record_lock_stays_private_to_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let lock_path = dir.path().join("quota.lock");
    // A lock created under the usual umask is group- and world-readable.
    // Opening it read-only is enough to take an exclusive flock.
    std::fs::write(&lock_path, []).unwrap();
    std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let store = Store::new(dir.path().join("quota.json"));
    store
        .observe(
            &Account::new("github.com", "octocat"),
            Resource::Core,
            quota(100),
            now(),
        )
        .unwrap();
    let mode = std::fs::metadata(&lock_path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o077,
        0,
        "quota lock mode {mode:o} lets another local user block the owner"
    );
}

#[test]
fn wait_calculation_uses_injected_clock_and_inclusive_jitter_endpoints() {
    let reset = now() + chrono::Duration::minutes(5);
    for (jitter, expected) in [(0, 360), (37, 397), (60, 420)] {
        let mut budget = WaitBudget::default();
        assert_eq!(
            budget.next_with(reset, now, || jitter),
            Ok(Duration::from_secs(expected))
        );
    }
}

#[test]
fn a_commands_wait_total_accepts_two_hours_and_rejects_more_without_spending_it() {
    let mut budget = WaitBudget::default();
    let reset = now() + chrono::Duration::seconds(3539);
    assert_eq!(
        budget.next_with(reset, now, || 0),
        Ok(Duration::from_secs(3599))
    );
    assert_eq!(
        budget.next_with(reset, now, || 0),
        Ok(Duration::from_secs(3599))
    );
    assert_eq!(budget.next_with(now(), now, || 0), Err(WaitCapExceeded));
    let reset = now() - chrono::Duration::seconds(58);
    assert_eq!(
        budget.next_with(reset, now, || 0),
        Ok(Duration::from_secs(2))
    );
    assert_eq!(budget.next_with(reset, now, || 0), Err(WaitCapExceeded));
}
