//! Every kind of session thirdshift runs: its Session-trace token, the Command
//! record stage it enters, and which `thirdshift logs --session` words
//! select it.

use crate::logs::Stage;

/// What a session is run as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A Custom process's numbered skill Step.
    ProcessStep(usize),
    ScoringGrader(usize),
    /// A Run's implement session, `implement`.
    Implementation,
    /// An Eval's review-only Code review, `code-review`.
    CodeReview,
    /// A Spec run's Spec review, `spec-review`.
    SpecReview,
    /// The numbered Repair, `repair-<n>`.
    Repair(usize),
    /// `security-review`.
    SecurityReview,
    /// A Security review retried once on the configured default Harness.
    SecurityReviewFallback,
    /// An Architect run's Architecture review, `architecture-review`.
    ArchitectureReview,
    Triage,
    /// One correction of an invalid Triage recommendation, retaining its investigation.
    TriageCorrection,
    /// `security-audit`.
    SecurityAudit,
    /// The Security reproduction of the numbered candidate,
    /// `security-reproduction-<n>`.
    SecurityReproduction(usize),
    /// `security-fix-publishing`.
    SecurityFixPublishing,
    /// The recheck of a fixed finding's proof-of-concept test against the
    /// merged Base branch, `security-close-check`.
    SecurityCloseCheck,
}

impl Kind {
    /// The kind's token, in its Session trace's name and progress lines.
    pub fn token(self) -> String {
        match self {
            Self::ProcessStep(n) => format!("process-step-{n}"),
            Self::ScoringGrader(n) => format!("scoring-grader-{n}"),
            Self::Implementation => "implement".to_string(),
            Self::CodeReview => "code-review".to_string(),
            Self::SpecReview => "spec-review".to_string(),
            Self::Repair(n) => format!("repair-{n}"),
            Self::SecurityReview => "security-review".to_string(),
            Self::SecurityReviewFallback => "security-review-fallback".to_string(),
            Self::ArchitectureReview => "architecture-review".to_string(),
            Self::Triage => "triage".to_string(),
            Self::TriageCorrection => "triage-correction".to_string(),
            Self::SecurityAudit => "security-audit".to_string(),
            Self::SecurityReproduction(n) => format!("security-reproduction-{n}"),
            Self::SecurityFixPublishing => "security-fix-publishing".to_string(),
            Self::SecurityCloseCheck => "security-close-check".to_string(),
        }
    }

    /// The kind `token` names, if any. A numbered kind's number is read as
    /// `--session repair` has always read a Repair's.
    fn read(token: &str) -> Option<Self> {
        const UNNUMBERED: [Kind; 11] = [
            Kind::Triage,
            Kind::TriageCorrection,
            Kind::Implementation,
            Kind::CodeReview,
            Kind::SpecReview,
            Kind::SecurityReview,
            Kind::SecurityReviewFallback,
            Kind::ArchitectureReview,
            Kind::SecurityAudit,
            Kind::SecurityFixPublishing,
            Kind::SecurityCloseCheck,
        ];
        let numbered = |prefix: &str| {
            let n = token.strip_prefix(prefix)?.parse::<u32>().ok()?;
            Some(n as usize)
        };
        UNNUMBERED
            .into_iter()
            .find(|kind| kind.token() == token)
            .or_else(|| numbered("repair-").map(Kind::Repair))
            .or_else(|| numbered("process-step-").map(Kind::ProcessStep))
            .or_else(|| numbered("scoring-grader-").map(Kind::ScoringGrader))
            .or_else(|| numbered("security-reproduction-").map(Kind::SecurityReproduction))
    }
}

/// A session as its Session trace names it: of `kind`, on its Step's
/// zero-based `route` (a later route when Plan usage moved the Step), and
/// its Resume if `resume`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Logged {
    pub kind: Kind,
    pub resume: bool,
    pub route: usize,
}

/// The suffix before a later route's number, as in `implement-route-2`.
const ROUTE: &str = "-route-";

/// `token` without a later route's `-route-<n>` suffix, with the route's
/// zero-based index: 0 with no suffix, as the first route has none.
fn without_route(token: &str) -> Option<(&str, usize)> {
    let Some((kind, number)) = token.rsplit_once(ROUTE) else {
        return Some((token, 0));
    };
    let n = number.parse::<usize>().ok().filter(|n| *n >= 2)?;
    (n.to_string() == number).then_some((kind, n - 1))
}

impl Logged {
    /// The session of `kind` itself, not its Resume.
    pub fn of(kind: Kind) -> Self {
        Logged {
            kind,
            resume: false,
            route: 0,
        }
    }

    /// The Resume of the session of `kind`.
    pub fn resume_of(kind: Kind) -> Self {
        Logged {
            kind,
            resume: true,
            route: 0,
        }
    }

    /// The same session on its Step's zero-based `route`.
    pub fn on_route(self, route: usize) -> Self {
        Logged { route, ..self }
    }

    /// The Session-trace token: the kind's, with `-route-<n>` for a later
    /// route's one-based number, then `-resume` for its Resume.
    pub fn token(self) -> String {
        let mut token = self.kind.token();
        if self.route > 0 {
            token = format!("{token}{ROUTE}{}", self.route + 1);
        }
        if self.resume {
            format!("{token}-resume")
        } else {
            token
        }
    }

    /// The session a Session trace's `token` names, if thirdshift can read it.
    pub fn read(token: &str) -> Option<Self> {
        let (rest, resume) = match token.strip_suffix("-resume") {
            Some(rest) => (rest, true),
            None => (token, false),
        };
        let (kind, route) = without_route(rest)?;
        let kind = Kind::read(kind)?;
        Some(Logged {
            kind,
            resume,
            route,
        })
    }

    /// The Command record stage the session enters. A Resume of the
    /// implement session, Code review, Spec review, a Repair or Security review enters
    /// `Resume`; any other Resume keeps its kind's stage.
    pub fn stage(self) -> Stage {
        match (self.kind, self.resume) {
            (Kind::ProcessStep(_), false) => Stage::ProcessStep,
            (Kind::ProcessStep(_), true) => Stage::Resume,
            (Kind::ScoringGrader(_), _) => Stage::Grading,
            (
                Kind::Implementation
                | Kind::CodeReview
                | Kind::SpecReview
                | Kind::Repair(_)
                | Kind::SecurityReview
                | Kind::SecurityReviewFallback,
                true,
            ) => Stage::Resume,
            (Kind::Implementation, false) => Stage::Implement,
            (Kind::CodeReview, false) => Stage::CodeReview,
            (Kind::SpecReview, false) => Stage::SpecReview,
            (Kind::Repair(_), false) => Stage::Repair,
            (Kind::SecurityReview | Kind::SecurityReviewFallback, false) => Stage::SecurityReview,
            (Kind::ArchitectureReview, _) => Stage::ArchitectureReview,
            (Kind::Triage | Kind::TriageCorrection, _) => Stage::Triage,
            (Kind::SecurityAudit, _) => Stage::SecurityAudit,
            (Kind::SecurityReproduction(_), _) => Stage::SecurityReproduction,
            (Kind::SecurityFixPublishing, _) => Stage::SecurityFixPublishing,
            (Kind::SecurityCloseCheck, _) => Stage::SecurityCloseCheck,
        }
    }
}

/// Whether `thirdshift logs --session <word>` selects the Session trace whose
/// token is `token`: one whose token is `word`, or `word` with `-resume`,
/// or, for `repair`, any numbered Repair or its Resume. A token no kind
/// reads, as older thirdshift versions may have written, is still selected
/// by the first two rules.
/// Whether `token` names a Security session or its Resume. Those traces hold
/// Security-run details and are readable only with the `security` scope.
pub(crate) fn is_security(token: &str) -> bool {
    matches!(
        Logged::read(token).map(|logged| logged.kind),
        Some(
            Kind::SecurityReview
                | Kind::SecurityReviewFallback
                | Kind::SecurityAudit
                | Kind::SecurityReproduction(_)
                | Kind::SecurityFixPublishing
                | Kind::SecurityCloseCheck
        )
    )
}

pub fn selects(word: &str, token: &str) -> bool {
    let resumed = token.strip_suffix("-resume").unwrap_or(token);
    // A later route's session is selected as its kind's first is.
    let resumed = without_route(resumed).map_or(resumed, |(kind, _)| kind);
    token == word
        || resumed == word
        || (word == "security-review" && resumed == "security-review-fallback")
        || (word == "repair"
            && matches!(
                Logged::read(token),
                Some(Logged {
                    kind: Kind::Repair(_),
                    ..
                })
            ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind, the numbered ones with a number of their own.
    const KINDS: [(Kind, &str); 12] = [
        (Kind::Triage, "triage"),
        (Kind::TriageCorrection, "triage-correction"),
        (Kind::Implementation, "implement"),
        (Kind::CodeReview, "code-review"),
        (Kind::SpecReview, "spec-review"),
        (Kind::Repair(3), "repair-3"),
        (Kind::SecurityReview, "security-review"),
        (Kind::ArchitectureReview, "architecture-review"),
        (Kind::SecurityAudit, "security-audit"),
        (Kind::SecurityReproduction(12), "security-reproduction-12"),
        (Kind::SecurityFixPublishing, "security-fix-publishing"),
        (Kind::SecurityCloseCheck, "security-close-check"),
    ];

    #[test]
    fn security_sessions_and_their_resumes_are_the_security_scope() {
        for token in [
            "security-review",
            "security-review-resume",
            "security-review-fallback",
            "security-review-fallback-resume",
            "security-audit",
            "security-reproduction-12",
            "security-reproduction-12-resume",
            "security-fix-publishing",
            "security-close-check",
        ] {
            assert!(is_security(token), "{token}");
        }
        for token in [
            "implement",
            "code-review",
            "repair-3",
            "architecture-review",
        ] {
            assert!(!is_security(token), "{token}");
        }
    }

    #[test]
    fn every_kind_reads_back_from_its_token_and_its_resumes() {
        for (kind, token) in KINDS {
            assert_eq!(Logged::of(kind).token(), token);
            assert_eq!(Logged::read(token), Some(Logged::of(kind)));
            let resume = format!("{token}-resume");
            assert_eq!(Logged::resume_of(kind).token(), resume);
            assert_eq!(Logged::read(&resume), Some(Logged::resume_of(kind)));
        }
    }

    #[test]
    fn a_later_routes_session_reads_back_and_is_selected_by_its_kind() {
        for (logged, token) in [
            (
                Logged::of(Kind::Implementation).on_route(1),
                "implement-route-2",
            ),
            (
                Logged::resume_of(Kind::Repair(3)).on_route(2),
                "repair-3-route-3-resume",
            ),
        ] {
            assert_eq!(logged.token(), token);
            assert_eq!(Logged::read(token), Some(logged));
        }
        assert!(selects("implement", "implement-route-2"));
        assert!(selects("implement", "implement-route-2-resume"));
        assert!(selects("repair", "repair-3-route-3"));
        assert!(is_security("security-review-fallback-route-2"));
        for token in [
            "implement-route-1",
            "implement-route-02",
            "implement-route-x",
        ] {
            assert_eq!(Logged::read(token), None, "{token}");
        }
    }

    #[test]
    fn malformed_tokens_read_back_as_no_kind() {
        for token in [
            "repair-",
            "repair-x",
            "repair-4294967296",
            "repair--resume",
            "security-reproduction-",
            "security-reproduction-x",
            "review",
            "-resume",
            "implement-resume-resume",
            "",
        ] {
            assert_eq!(Logged::read(token), None, "{token}");
        }
    }

    #[test]
    fn each_session_enters_its_command_record_stage() {
        let stages = [
            (Kind::Triage, "Triage", "Triage"),
            (Kind::TriageCorrection, "Triage", "Triage"),
            (Kind::Implementation, "implement", "Resume"),
            (Kind::CodeReview, "Code review", "Resume"),
            (Kind::SpecReview, "Spec review", "Resume"),
            (Kind::Repair(1), "Repair", "Resume"),
            (Kind::SecurityReview, "Security review", "Resume"),
            (
                Kind::ArchitectureReview,
                "Architecture review",
                "Architecture review",
            ),
            (Kind::SecurityAudit, "Security audit", "Security audit"),
            (
                Kind::SecurityReproduction(2),
                "Security reproduction",
                "Security reproduction",
            ),
            (
                Kind::SecurityFixPublishing,
                "Security fix publishing",
                "Security fix publishing",
            ),
            (
                Kind::SecurityCloseCheck,
                "Security close check",
                "Security close check",
            ),
        ];
        for (kind, stage, resume_stage) in stages {
            assert_eq!(Logged::of(kind).stage().name(), stage);
            assert_eq!(Logged::resume_of(kind).stage().name(), resume_stage);
        }
    }

    #[test]
    fn a_session_word_selects_its_token_its_kind_or_any_repair() {
        assert!(selects("implement", "implement"));
        assert!(selects("implement", "implement-resume"));
        assert!(selects("implement-resume", "implement-resume"));
        assert!(!selects("implement-resume", "implement"));
        assert!(!selects("implement", "spec-review"));

        assert!(selects("repair-2", "repair-2"));
        assert!(selects("repair-2", "repair-2-resume"));
        assert!(!selects("repair-2", "repair-1"));
        assert!(selects("repair", "repair-1"));
        assert!(selects("repair", "repair-2-resume"));
        assert!(!selects("repair", "repair-x"));
        assert!(!selects("security-reproduction", "security-reproduction-1"));
        assert!(selects(
            "security-reproduction-1",
            "security-reproduction-1-resume"
        ));
    }

    #[test]
    fn repair_selects_the_numbers_it_has_always_read_and_no_others() {
        assert!(selects("repair", "repair-+1"));
        assert!(selects("repair", "repair-01-resume"));
        assert!(!selects("repair", "repair-4294967296"));
        assert!(!selects("repair-1", "repair-01"));
    }

    #[test]
    fn an_unreadable_token_is_selected_by_itself_or_without_its_resume() {
        assert!(selects("review", "review"));
        assert!(selects("repair-x", "repair-x"));
        assert!(!selects("repair", "repair-x"));
        assert!(selects("legacy-resume", "legacy-resume"));
        assert!(selects("legacy", "legacy-resume"));
        assert!(!selects("legacy", "legacy-other"));
    }
}
