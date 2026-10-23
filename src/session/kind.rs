//! Every kind of session thirdshift runs: its Session-trace token, the Command
//! record stage it enters, and which `thirdshift logs --session` words
//! select it.

use crate::logs::Stage;

/// What a session is run as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A Run's implement session, `implement`.
    Implementation,
    /// A Spec run's Spec review, `spec-review`.
    SpecReview,
    /// The numbered Repair, `repair-<n>`.
    Repair(usize),
    /// `security-review`.
    SecurityReview,
    /// An Architect run's Architecture review, `architecture-review`.
    ArchitectureReview,
    /// `security-audit`.
    SecurityAudit,
    /// The Security reproduction of the numbered candidate,
    /// `security-reproduction-<n>`.
    SecurityReproduction(usize),
    /// `security-fix-publishing`.
    SecurityFixPublishing,
}

impl Kind {
    /// The kind's token, in its Session trace's name and progress lines.
    pub fn token(self) -> String {
        match self {
            Self::Implementation => "implement".to_string(),
            Self::SpecReview => "spec-review".to_string(),
            Self::Repair(n) => format!("repair-{n}"),
            Self::SecurityReview => "security-review".to_string(),
            Self::ArchitectureReview => "architecture-review".to_string(),
            Self::SecurityAudit => "security-audit".to_string(),
            Self::SecurityReproduction(n) => format!("security-reproduction-{n}"),
            Self::SecurityFixPublishing => "security-fix-publishing".to_string(),
        }
    }

    /// The kind `token` names, if any. A numbered kind's number is read as
    /// `--session repair` has always read a Repair's.
    fn read(token: &str) -> Option<Self> {
        const UNNUMBERED: [Kind; 6] = [
            Kind::Implementation,
            Kind::SpecReview,
            Kind::SecurityReview,
            Kind::ArchitectureReview,
            Kind::SecurityAudit,
            Kind::SecurityFixPublishing,
        ];
        let numbered = |prefix: &str| {
            let n = token.strip_prefix(prefix)?.parse::<u32>().ok()?;
            Some(n as usize)
        };
        UNNUMBERED
            .into_iter()
            .find(|kind| kind.token() == token)
            .or_else(|| numbered("repair-").map(Kind::Repair))
            .or_else(|| numbered("security-reproduction-").map(Kind::SecurityReproduction))
    }
}

/// A session as its Session trace names it: of `kind`, and its Resume if
/// `resume`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Logged {
    pub kind: Kind,
    pub resume: bool,
}

impl Logged {
    /// The session of `kind` itself, not its Resume.
    pub fn of(kind: Kind) -> Self {
        Logged {
            kind,
            resume: false,
        }
    }

    /// The Resume of the session of `kind`.
    pub fn resume_of(kind: Kind) -> Self {
        Logged { kind, resume: true }
    }

    /// The Session-trace token: the kind's, with `-resume` for its Resume.
    pub fn token(self) -> String {
        let token = self.kind.token();
        if self.resume {
            format!("{token}-resume")
        } else {
            token
        }
    }

    /// The session a Session trace's `token` names, if thirdshift can read it.
    pub fn read(token: &str) -> Option<Self> {
        match token.strip_suffix("-resume") {
            Some(kind) => Kind::read(kind).map(Self::resume_of),
            None => Kind::read(token).map(Self::of),
        }
    }

    /// The Command record stage the session enters. A Resume of the
    /// implement session, Spec review, a Repair or Security review enters
    /// `Resume`; any other Resume keeps its kind's stage.
    pub fn stage(self) -> Stage {
        match (self.kind, self.resume) {
            (
                Kind::Implementation | Kind::SpecReview | Kind::Repair(_) | Kind::SecurityReview,
                true,
            ) => Stage::Resume,
            (Kind::Implementation, false) => Stage::Implement,
            (Kind::SpecReview, false) => Stage::SpecReview,
            (Kind::Repair(_), false) => Stage::Repair,
            (Kind::SecurityReview, false) => Stage::SecurityReview,
            (Kind::ArchitectureReview, _) => Stage::ArchitectureReview,
            (Kind::SecurityAudit, _) => Stage::SecurityAudit,
            (Kind::SecurityReproduction(_), _) => Stage::SecurityReproduction,
            (Kind::SecurityFixPublishing, _) => Stage::SecurityFixPublishing,
        }
    }
}

/// Whether `thirdshift logs --session <word>` selects the Session trace whose
/// token is `token`: one whose token is `word`, or `word` with `-resume`,
/// or, for `repair`, any numbered Repair or its Resume. A token no kind
/// reads, as older thirdshift versions may have written, is still selected
/// by the first two rules.
pub fn selects(word: &str, token: &str) -> bool {
    let resumed = token.strip_suffix("-resume").unwrap_or(token);
    token == word
        || resumed == word
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
    const KINDS: [(Kind, &str); 8] = [
        (Kind::Implementation, "implement"),
        (Kind::SpecReview, "spec-review"),
        (Kind::Repair(3), "repair-3"),
        (Kind::SecurityReview, "security-review"),
        (Kind::ArchitectureReview, "architecture-review"),
        (Kind::SecurityAudit, "security-audit"),
        (Kind::SecurityReproduction(12), "security-reproduction-12"),
        (Kind::SecurityFixPublishing, "security-fix-publishing"),
    ];

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
            (Kind::Implementation, "implement", "Resume"),
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
