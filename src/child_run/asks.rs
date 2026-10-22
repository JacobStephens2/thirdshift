//! What every child Run a Run starts is asked, whichever kind it is: built
//! once in the Run, after its Harness check, and handed on unchanged.

use crate::asks::Asks;
use crate::harness::Choice;

/// The Command's checked Harness, Model and Effort, those of its Security
/// review, its Security fix ask and its Security review ask, as every child
/// Run is given them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ChildAsks {
    /// The Harness, Model and Effort a child Run's sessions run on, as the
    /// Command's checks settled them.
    pub harness: Choice,
    /// What a child Run's Security review runs on instead of
    /// [`ChildAsks::harness`], if anything, as the Command's checks settled
    /// it.
    pub security_review_harness: Option<Choice>,
    /// The Command's configured default, preserved for review fallback.
    pub security_review_default: Option<Choice>,
    /// Whether a Security run may fix reproduced findings.
    pub security_fix: bool,
    /// Whether the Command asked for a Security review, for every child Run too.
    pub security_review: bool,
}

impl ChildAsks {
    /// What a Run asked `asks` hands on to every child Run it starts. Built
    /// only once `asks.harness`, and `asks.security_review_harness`, have
    /// been checked: by this Run, or by the
    /// command that started it.
    pub fn of(asks: &Asks) -> ChildAsks {
        ChildAsks {
            harness: asks.harness.clone(),
            security_review_harness: asks.security_review_harness.clone(),
            security_review_default: asks.security_review_default.clone(),
            security_fix: asks.security_fix,
            security_review: asks.security_review,
        }
    }
}
