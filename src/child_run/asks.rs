//! What every child Run a Run starts is asked, whichever kind it is: built
//! once in the Run, after its Harness check, and handed on unchanged.

use super::Kind;
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
    /// Whether a Security run may fix reproduced findings.
    pub security_fix: bool,
    /// Whether the Command asked for a Security review: its own ask, before
    /// any Run of a kind that runs none drops it. See
    /// [`ChildAsks::runs_security_review`].
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
            security_fix: asks.security_fix,
            security_review: asks.security_review,
        }
    }

    /// Whether a Run that is the child Run `child`, if it is one, runs its
    /// own Security review. A Ticket's Run runs none: its Spec run reviews
    /// the Spec PR instead. A Base fix runs one when the Command asked for
    /// one, whichever Run started it.
    pub fn runs_security_review(&self, child: Option<&Kind>) -> bool {
        self.security_review && !matches!(child, Some(Kind::Ticket { .. }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(security_review: bool) -> ChildAsks {
        ChildAsks {
            security_review,
            ..ChildAsks::default()
        }
    }

    #[test]
    fn only_a_ticket_s_run_skips_the_security_review_the_command_asked_for() {
        let ticket = Kind::Ticket {
            spec_branch: "issue-20".to_string(),
        };
        let base_fix = Kind::BaseFix {
            base: "main".to_string(),
        };
        for (child, when_asked) in [
            (None, true),
            (Some(&ticket), false),
            (Some(&base_fix), true),
        ] {
            assert_eq!(
                asked(true).runs_security_review(child),
                when_asked,
                "{child:?}"
            );
            assert!(!asked(false).runs_security_review(child), "{child:?}");
        }
    }
}
