use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Weak};
use std::task::{Context, Poll, Waker};

use anyhow::anyhow;
use tokio::sync::watch;

use super::*;
use crate::console::changes::Revision;

#[derive(Clone)]
struct Changes(Arc<Mutex<Option<watch::Sender<Revision>>>>);

impl Changes {
    fn commands(&self) {
        self.send(|revision| revision.commands += 1);
    }

    fn accounts(&self) {
        self.send(|revision| revision.accounts += 1);
    }

    fn close(&self) {
        self.0.lock().unwrap().take();
    }

    fn send(&self, change: impl FnOnce(&mut Revision)) {
        self.0.lock().unwrap().as_ref().unwrap().send_modify(change);
    }
}

struct Fixture {
    subscription: Subscription,
    changes: Changes,
    checks: Arc<AtomicUsize>,
    lease: Weak<Following>,
}

impl Fixture {
    fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }
}

/// The script receives the 1-based number of each authorization check.
fn subscribe(script: impl Fn(&Changes, usize) -> Result<bool> + Send + Sync + 'static) -> Fixture {
    let (sender, receiver) = watch::channel(Revision::default());
    let changes = Changes(Arc::new(Mutex::new(Some(sender))));
    let checks = Arc::new(AtomicUsize::new(0));
    let (scripted, counted) = (changes.clone(), checks.clone());
    let check = Check::Scripted(Arc::new(move || {
        script(&scripted, counted.fetch_add(1, Ordering::SeqCst) + 1)
    }));
    let lease = Arc::new(Following::idle());
    Fixture {
        lease: Arc::downgrade(&lease),
        subscription: Subscription::with(receiver, check, lease),
        changes,
        checks,
    }
}

fn allowed() -> Fixture {
    subscribe(|_, _| Ok(true))
}

fn run(test: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(test);
}

// One poll without a real waker: Pending means the subscription is asleep.
fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

fn granted_now(subscription: &mut Subscription, mode: Readiness) -> bool {
    match poll_once(pin!(subscription.ready(mode))) {
        Poll::Ready(result) => result.unwrap(),
        Poll::Pending => false,
    }
}

#[test]
fn a_connection_gets_its_first_work_turn_after_one_check_in_either_mode() {
    run(async {
        for mode in [Readiness::Wait, Readiness::Drain] {
            let mut fixture = allowed();
            assert!(fixture.subscription.ready(mode).await.unwrap());
            assert_eq!(fixture.checks(), 1);
        }
    });
}

#[test]
fn wait_sleeps_until_commands_change_but_drain_permits_work_now() {
    run(async {
        let mut fixture = allowed();
        assert!(fixture.subscription.ready(Readiness::Wait).await.unwrap());
        for _ in 0..3 {
            assert!(granted_now(&mut fixture.subscription, Readiness::Drain));
        }
        {
            let mut waiting = pin!(fixture.subscription.ready(Readiness::Wait));
            assert!(poll_once(waiting.as_mut()).is_pending());
            fixture.changes.commands();
            assert!(waiting.await.unwrap());
        }
        // Work turns at the same accounts revision do not reread accounts.
        assert_eq!(fixture.checks(), 1);
    });
}

#[test]
fn commands_changes_coalesce_to_the_latest_revision() {
    run(async {
        let mut fixture = allowed();
        assert!(fixture.subscription.ready(Readiness::Wait).await.unwrap());
        for _ in 0..3 {
            fixture.changes.commands();
        }
        assert!(granted_now(&mut fixture.subscription, Readiness::Wait));
        assert!(!granted_now(&mut fixture.subscription, Readiness::Wait));
    });
}

#[test]
fn an_account_only_change_rechecks_without_a_work_turn() {
    run(async {
        // Closing during the recheck makes a wrongly permitted turn observable.
        let mut fixture = subscribe(|changes, check| {
            if check == 2 {
                changes.close();
            }
            Ok(true)
        });
        assert!(fixture.subscription.ready(Readiness::Wait).await.unwrap());
        fixture.changes.accounts();
        assert!(!fixture.subscription.ready(Readiness::Wait).await.unwrap());
        assert_eq!(fixture.checks(), 2);
    });
}

#[test]
fn an_account_change_after_a_wait_is_rechecked_before_the_next_notice() {
    run(async {
        let mut fixture = allowed();
        assert!(fixture.subscription.ready(Readiness::Wait).await.unwrap());
        fixture.changes.accounts();
        fixture.changes.commands();
        assert!(fixture.subscription.ready(Readiness::Wait).await.unwrap());
        assert_eq!(fixture.checks(), 2);
    });
}

#[test]
fn a_revision_arriving_during_authorization_is_checked_before_work() {
    run(async {
        for (newer, permitted) in [(true, true), (false, false)] {
            let mut fixture = subscribe(move |changes, check| {
                if check == 1 {
                    changes.accounts();
                    return Ok(true);
                }
                Ok(newer)
            });
            assert_eq!(
                fixture.subscription.ready(Readiness::Wait).await.unwrap(),
                permitted
            );
            assert_eq!(fixture.checks(), 2);
        }
    });
}

#[test]
fn a_change_during_work_is_kept_for_the_next_wait() {
    run(async {
        for first in [Readiness::Wait, Readiness::Drain] {
            let mut fixture = allowed();
            assert!(fixture.subscription.ready(first).await.unwrap());
            // The caller is reading when the change arrives.
            fixture.changes.commands();
            assert!(granted_now(&mut fixture.subscription, Readiness::Wait));
            assert!(!granted_now(&mut fixture.subscription, Readiness::Wait));
        }
    });
}

#[test]
fn closure_permits_no_further_work_and_releases_the_lease() {
    run(async {
        for mode in [Readiness::Wait, Readiness::Drain] {
            // Closure after an unseen change, and while asleep.
            let mut fixture = allowed();
            assert!(fixture.subscription.ready(mode).await.unwrap());
            fixture.changes.commands();
            fixture.changes.close();
            assert!(!fixture.subscription.ready(mode).await.unwrap());
            assert!(fixture.lease.upgrade().is_none());

            let mut fixture = allowed();
            assert!(fixture.subscription.ready(mode).await.unwrap());
            {
                let mut waiting = pin!(fixture.subscription.ready(Readiness::Wait));
                assert!(poll_once(waiting.as_mut()).is_pending());
                fixture.changes.close();
                assert!(!waiting.await.unwrap());
            }
            assert!(fixture.lease.upgrade().is_none());

            // Closure during a successful check, where only the recheck stops work.
            let mut fixture = subscribe(|changes, _| {
                changes.close();
                Ok(true)
            });
            assert!(!fixture.subscription.ready(mode).await.unwrap());
            assert_eq!(fixture.checks(), 1);
            assert!(fixture.lease.upgrade().is_none());
        }
    });
}

#[test]
fn denial_and_check_failures_end_the_subscription_permanently() {
    run(async {
        let denied = subscribe(|_, _| Ok(false));
        let failed = subscribe(|_, _| Err(anyhow!("accounts unreadable")));
        // A panicking check stands in for a failed blocking task.
        let panicked = subscribe(|_, _| panic!("scripted blocking-task failure"));
        for (mut fixture, error) in [(denied, false), (failed, true), (panicked, true)] {
            let first = fixture.subscription.ready(Readiness::Drain).await;
            assert_eq!(first.is_err(), error);
            assert!(!first.unwrap_or(false));
            assert!(fixture.lease.upgrade().is_none());
            fixture.changes.accounts();
            fixture.changes.commands();
            for mode in [Readiness::Drain, Readiness::Wait] {
                assert!(!fixture.subscription.ready(mode).await.unwrap());
            }
            assert_eq!(fixture.checks(), 1);
        }
    });
}

#[test]
fn dropping_a_subscription_releases_its_lease() {
    run(async {
        let mut fixture = allowed();
        assert!(fixture.subscription.ready(Readiness::Wait).await.unwrap());
        assert!(fixture.lease.upgrade().is_some());
        let lease = fixture.lease.clone();
        drop(fixture);
        assert!(lease.upgrade().is_none());
    });
}
