//! When a live Console stream may do its next work turn. Keeps the viewer
//! authorized without extending idle sign-in, sleeps until a relevant change
//! and holds the shared live-work observer until the stream ends.
use std::sync::Arc;

use anyhow::Result;

use super::auth::Viewer;
use super::changes::{Notifier, Receiver, Revision};
use super::watching::Following;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Readiness {
    /// Catch up immediately once, then sleep until Commands change.
    Wait,
    /// Page change notices include development assets as well as Commands.
    Page,
    /// Permit work now, e.g. output already buffered behind the last batch.
    Drain,
}

pub(super) struct Subscription {
    changes: Receiver,
    check: Check,
    authorized_accounts: Option<u64>,
    granted: Option<Revision>,
    // Released on termination; None means no further work turn.
    lease: Option<Arc<Following>>,
}

impl Subscription {
    pub fn new(changes: Receiver, viewer: Viewer, notifier: &Notifier) -> Self {
        let lease = notifier.follow(changes.clone());
        Self::with(changes, Check::Viewer(viewer), lease)
    }

    fn with(changes: Receiver, check: Check, lease: Arc<Following>) -> Self {
        Self {
            changes,
            check,
            authorized_accounts: None,
            granted: None,
            lease: Some(lease),
        }
    }

    /// True permits one work turn; false is normal closure. An error ends
    /// the subscription too, and every later call returns false.
    pub async fn ready(&mut self, mode: Readiness) -> Result<bool> {
        if self.lease.is_none() {
            return Ok(false);
        }
        let result = self.next(mode).await;
        if !matches!(result, Ok(true)) {
            self.lease = None;
        }
        result
    }

    pub fn revision(&self) -> Revision {
        self.granted.expect("a work turn has been granted")
    }

    async fn next(&mut self, mode: Readiness) -> Result<bool> {
        loop {
            if self.changes.has_changed().is_err() {
                return Ok(false);
            }
            // Copy out: a watch borrow must not be held across an await.
            let current = *self.changes.borrow_and_update();
            if self.authorized_accounts != Some(current.accounts) {
                let check = self.check.clone();
                if !tokio::task::spawn_blocking(move || check.allowed()).await?? {
                    return Ok(false);
                }
                // Recheck closure and any revision that arrived meanwhile.
                self.authorized_accounts = Some(current.accounts);
                continue;
            }
            if mode == Readiness::Drain
                || self.granted.is_none_or(|previous| {
                    previous.commands != current.commands
                        || (mode == Readiness::Page
                            && (previous.styles != current.styles
                                || previous.scripts != current.scripts))
                })
            {
                // A change during the caller's work stays unseen until it waits.
                self.granted = Some(current);
                return Ok(true);
            }
            if self.changes.changed().await.is_err() {
                return Ok(false);
            }
        }
    }
}

#[derive(Clone)]
enum Check {
    Viewer(Viewer),
    #[cfg(test)]
    Scripted(Arc<dyn Fn() -> Result<bool> + Send + Sync>),
}

impl Check {
    fn allowed(&self) -> Result<bool> {
        match self {
            Self::Viewer(viewer) => viewer.still_allowed(),
            #[cfg(test)]
            Self::Scripted(check) => check(),
        }
    }
}

#[cfg(test)]
mod tests;
