//! The fixed Trusted account rule, shared by every path that starts work.
//!
//! The rule reads GitHub only through [`Reads`], each read made only when the
//! check reaches it: the author first, then the latest `ready-for-agent`
//! labelling, then the edits, body before title, stopping at the first
//! failure.

use std::fmt;

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::issue::IssueUrl;

/// The first trust check an issue failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UntrustedIssue {
    Author,
    NeverReady,
    ReadyLabel,
    /// Its `IssueText` was edited, after it was last marked ready, by an account
    /// that is not a Trusted account.
    Edited(IssueText),
}

impl fmt::Display for UntrustedIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Author => f.write_str("issue author is not a Trusted account"),
            Self::NeverReady => f.write_str("has never been labelled ready-for-agent"),
            Self::ReadyLabel => {
                f.write_str("most recent ready-for-agent label actor is not a Trusted account")
            }
            Self::Edited(edit) => write!(
                f,
                "{edit} edited after it was marked ready by an account that is not a Trusted account"
            ),
        }
    }
}

/// The part of an issue an edit changed: its body or its title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueText {
    Body,
    Title,
}

impl fmt::Display for IssueText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Body => "body",
            Self::Title => "title",
        })
    }
}

/// A `ready-for-agent` labelling or an edit of an issue: the account that
/// made it, if GitHub still knows it, and when.
#[derive(Clone, Debug)]
pub struct Action {
    pub actor: Option<String>,
    pub at: DateTime<Utc>,
}

/// What the rule reads of GitHub about an issue.
pub trait Reads {
    /// The `authorAssociation` of `issue`'s author with its repository.
    fn author_association(&self, issue: &IssueUrl) -> Result<String>;
    /// Every `ready-for-agent` labelling of `issue`, oldest first.
    fn ready_labellings(&self, issue: &IssueUrl) -> Result<Vec<Action>>;
    /// Every edit of `issue`'s `text`, each read only when
    /// the rule reaches it: an unreadable edit after the first untrusted
    /// one cannot fail the check.
    fn edits(
        &self,
        issue: &IssueUrl,
        text: IssueText,
    ) -> Result<impl Iterator<Item = Result<Action>>>;
    /// Whether `login` is a collaborator on `issue`'s repository or a member
    /// of the organization that owns it.
    fn is_member(&self, issue: &IssueUrl, login: &str) -> Result<bool>;
}

/// Check the issue author, the most recent ready-for-agent label actor,
/// and every edit to its body or title made since that labelling.
pub fn issue_trust(reads: &impl Reads, issue: &IssueUrl) -> Result<Option<UntrustedIssue>> {
    let mut check = Check::new(reads);
    if !check.trusted_author(issue)? {
        return Ok(Some(UntrustedIssue::Author));
    }
    let Some(Action { actor, at }) = reads.ready_labellings(issue)?.pop() else {
        return Ok(Some(UntrustedIssue::NeverReady));
    };
    let Some(actor) = actor else {
        return Ok(Some(UntrustedIssue::ReadyLabel));
    };
    if !check.trusted_actor(issue, &actor)? {
        return Ok(Some(UntrustedIssue::ReadyLabel));
    }
    Ok(check
        .untrusted_edit(issue, Some(at))?
        .map(UntrustedIssue::Edited))
}

/// Check a Ticket before its Run starts: its author, and every edit to
/// its body or title made since the later of the Ticket's and its
/// `spec`'s last ready-for-agent labellings by a Trusted account, or
/// every edit if neither has one. A Ticket needs no label of its own:
/// its Spec's labelling already passed the full check.
pub fn ticket_trust(
    reads: &impl Reads,
    ticket: &IssueUrl,
    spec: &IssueUrl,
) -> Result<Option<UntrustedIssue>> {
    let mut check = Check::new(reads);
    if !check.trusted_author(ticket)? {
        return Ok(Some(UntrustedIssue::Author));
    }
    let since = check
        .trusted_ready_at(ticket)?
        .max(check.trusted_ready_at(spec)?);
    Ok(check
        .untrusted_edit(ticket, since)?
        .map(UntrustedIssue::Edited))
}

/// The first edit to `issue`'s body or title made after `since`, or ever
/// if `since` is `None`, by an account that is not a Trusted account, if
/// there is one. An edit whose account GitHub no longer knows counts.
pub fn untrusted_edit(
    reads: &impl Reads,
    issue: &IssueUrl,
    since: Option<DateTime<Utc>>,
) -> Result<Option<IssueText>> {
    Check::new(reads).untrusted_edit(issue, since)
}

/// Whether an author's `authorAssociation` with the repository makes them a
/// Trusted account: its owner, a member of the organization that owns it, or
/// a collaborator on it.
pub fn trusted_association(association: &str) -> bool {
    matches!(association, "OWNER" | "MEMBER" | "COLLABORATOR")
}

/// One top-level check, with each account it has found trusted so far.
struct Check<'a, R> {
    reads: &'a R,
    /// Each account found trusted, with the repository it is trusted on.
    trusted: Vec<(String, String)>,
}

impl<'a, R: Reads> Check<'a, R> {
    fn new(reads: &'a R) -> Self {
        Self {
            reads,
            trusted: Vec::new(),
        }
    }

    fn trusted_author(&self, issue: &IssueUrl) -> Result<bool> {
        Ok(trusted_association(&self.reads.author_association(issue)?))
    }

    fn untrusted_edit(
        &mut self,
        issue: &IssueUrl,
        since: Option<DateTime<Utc>>,
    ) -> Result<Option<IssueText>> {
        for text in [IssueText::Body, IssueText::Title] {
            for edit in self.reads.edits(issue, text)? {
                let edit = edit?;
                if since.is_some_and(|since| edit.at <= since) {
                    continue;
                }
                let Some(actor) = edit.actor else {
                    return Ok(Some(text));
                };
                if !self.trusted_actor(issue, &actor)? {
                    return Ok(Some(text));
                }
            }
        }
        Ok(None)
    }

    /// When `issue` was last labelled ready-for-agent by a Trusted account,
    /// if it ever was.
    fn trusted_ready_at(&mut self, issue: &IssueUrl) -> Result<Option<DateTime<Utc>>> {
        for labelling in self.reads.ready_labellings(issue)?.into_iter().rev() {
            if let Some(actor) = &labelling.actor
                && self.trusted_actor(issue, actor)?
            {
                return Ok(Some(labelling.at));
            }
        }
        Ok(None)
    }

    /// Labellings and edits have no author association: the repository's
    /// owner is trusted, and anyone else only by actual membership.
    fn trusted_actor(&mut self, issue: &IssueUrl, actor: &str) -> Result<bool> {
        if actor.eq_ignore_ascii_case(&issue.owner) {
            return Ok(true);
        }
        let repo = issue.repo_slug();
        if self.trusted.iter().any(|(known_repo, known)| {
            known_repo.eq_ignore_ascii_case(&repo) && known.eq_ignore_ascii_case(actor)
        }) {
            return Ok(true);
        }
        if !self.reads.is_member(issue, actor)? {
            return Ok(false);
        }
        self.trusted.push((repo, actor.to_string()));
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use anyhow::bail;

    use super::*;

    const ISSUE: u64 = 7;
    const SPEC: u64 = 20;

    fn url(number: u64) -> IssueUrl {
        IssueUrl::parse(&format!("https://github.com/acme/widgets/issues/{number}")).unwrap()
    }

    fn at(day: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2020-01-{day:02}T00:00:00Z"))
            .unwrap()
            .to_utc()
    }

    fn by(actor: &str, day: u32) -> Action {
        Action {
            actor: Some(actor.to_string()),
            at: at(day),
        }
    }

    fn unknown(day: u32) -> Action {
        Action {
            actor: None,
            at: at(day),
        }
    }

    /// An issue's facts as GitHub would give them.
    #[derive(Default)]
    struct Issue {
        association: Option<&'static str>,
        labellings: Vec<Action>,
        body_edits: Vec<Action>,
        title_edits: Vec<Action>,
    }

    /// Reads from memory: `reader` and `member` are members, everyone else
    /// is not, and `broken` fails its membership read. Every membership read
    /// is recorded in `looked_up`.
    #[derive(Default)]
    struct InMemory {
        issues: HashMap<u64, Issue>,
        looked_up: RefCell<Vec<String>>,
        edits_read: RefCell<Vec<IssueText>>,
    }

    impl InMemory {
        fn with(issue: Issue) -> Self {
            Self::default().and(ISSUE, issue)
        }

        fn and(mut self, number: u64, issue: Issue) -> Self {
            self.issues.insert(number, issue);
            self
        }

        fn issue(&self, issue: &IssueUrl) -> &Issue {
            &self.issues[&issue.number]
        }
    }

    impl Reads for InMemory {
        fn author_association(&self, issue: &IssueUrl) -> Result<String> {
            Ok(self.issue(issue).association.unwrap_or("OWNER").to_string())
        }

        fn ready_labellings(&self, issue: &IssueUrl) -> Result<Vec<Action>> {
            Ok(self.issue(issue).labellings.clone())
        }

        fn edits(
            &self,
            issue: &IssueUrl,
            text: IssueText,
        ) -> Result<impl Iterator<Item = Result<Action>>> {
            self.edits_read.borrow_mut().push(text);
            let issue = self.issue(issue);
            let edits = match text {
                IssueText::Body => issue.body_edits.clone(),
                IssueText::Title => issue.title_edits.clone(),
            };
            Ok(edits.into_iter().map(Ok))
        }

        fn is_member(&self, _issue: &IssueUrl, login: &str) -> Result<bool> {
            self.looked_up.borrow_mut().push(login.to_string());
            match login {
                "reader" | "member" => Ok(true),
                "broken" => bail!("could not check Trusted account membership"),
                _ => Ok(false),
            }
        }
    }

    /// An issue by a Trusted author, last marked ready by `reader` on day 1.
    fn ready() -> Issue {
        Issue {
            labellings: vec![by("reader", 1)],
            ..Issue::default()
        }
    }

    fn issue_trust_of(reads: &InMemory) -> Option<UntrustedIssue> {
        issue_trust(reads, &url(ISSUE)).unwrap()
    }

    fn ticket_trust_of(reads: &InMemory) -> Option<UntrustedIssue> {
        ticket_trust(reads, &url(ISSUE), &url(SPEC)).unwrap()
    }

    #[test]
    fn an_issue_by_an_untrusted_author_is_untrusted_without_reading_further() {
        for association in ["NONE", "CONTRIBUTOR", "FIRST_TIME_CONTRIBUTOR"] {
            let reads = InMemory::with(Issue {
                association: Some(association),
                ..ready()
            });

            assert_eq!(issue_trust_of(&reads), Some(UntrustedIssue::Author));
            assert!(reads.looked_up.borrow().is_empty());
            assert!(reads.edits_read.borrow().is_empty());
        }
    }

    #[test]
    fn trusted_associations_are_the_owner_members_and_collaborators() {
        for association in ["OWNER", "MEMBER", "COLLABORATOR"] {
            assert!(trusted_association(association), "{association}");
        }
        for association in ["CONTRIBUTOR", "NONE", "owner", ""] {
            assert!(!trusted_association(association), "{association}");
        }
    }

    #[test]
    fn an_issue_never_labelled_ready_is_untrusted() {
        let reads = InMemory::with(Issue::default());

        let reason = issue_trust_of(&reads).unwrap();
        assert_eq!(reason, UntrustedIssue::NeverReady);
        assert_eq!(
            reason.to_string(),
            "has never been labelled ready-for-agent"
        );
        assert!(reads.edits_read.borrow().is_empty());
    }

    #[test]
    fn a_latest_labelling_with_no_actor_is_untrusted() {
        let reads = InMemory::with(Issue {
            labellings: vec![by("acme", 1), unknown(2)],
            ..Issue::default()
        });

        assert_eq!(issue_trust_of(&reads), Some(UntrustedIssue::ReadyLabel));
    }

    #[test]
    fn a_latest_labelling_by_an_untrusted_actor_is_untrusted_after_an_earlier_trusted_one() {
        let reads = InMemory::with(Issue {
            labellings: vec![by("acme", 1), by("outsider", 2)],
            ..Issue::default()
        });

        assert_eq!(issue_trust_of(&reads), Some(UntrustedIssue::ReadyLabel));
        assert!(reads.edits_read.borrow().is_empty());
    }

    #[test]
    fn a_latest_trusted_labelling_supersedes_an_earlier_untrusted_one() {
        let reads = InMemory::with(Issue {
            labellings: vec![by("outsider", 1), by("reader", 2)],
            ..Issue::default()
        });

        assert_eq!(issue_trust_of(&reads), None);
    }

    #[test]
    fn edits_at_or_before_the_labelling_are_ignored() {
        let reads = InMemory::with(Issue {
            body_edits: vec![by("outsider", 1), unknown(1)],
            title_edits: vec![by("outsider", 1)],
            ..ready()
        });

        assert_eq!(issue_trust_of(&reads), None);
        assert_eq!(*reads.looked_up.borrow(), ["reader"]);
    }

    #[test]
    fn an_edit_after_the_labelling_by_a_trusted_account_leaves_it_trusted() {
        let reads = InMemory::with(Issue {
            body_edits: vec![by("member", 2), by("ACME", 3)],
            title_edits: vec![by("reader", 2)],
            ..ready()
        });

        assert_eq!(issue_trust_of(&reads), None);
    }

    #[test]
    fn an_edit_after_the_labelling_by_an_untrusted_or_unknown_account_is_untrusted() {
        for edit in [by("outsider", 2), unknown(2)] {
            let reads = InMemory::with(Issue {
                body_edits: vec![edit],
                ..ready()
            });

            assert_eq!(
                issue_trust_of(&reads),
                Some(UntrustedIssue::Edited(IssueText::Body))
            );
            assert_eq!(
                *reads.edits_read.borrow(),
                [IssueText::Body],
                "no title edits read after a failing body edit"
            );
        }
    }

    #[test]
    fn a_title_edit_after_the_labelling_by_an_untrusted_account_is_untrusted() {
        let reads = InMemory::with(Issue {
            body_edits: vec![by("reader", 2)],
            title_edits: vec![by("outsider", 3)],
            ..ready()
        });

        assert_eq!(
            issue_trust_of(&reads),
            Some(UntrustedIssue::Edited(IssueText::Title))
        );
        assert_eq!(
            *reads.edits_read.borrow(),
            [IssueText::Body, IssueText::Title]
        );
    }

    #[test]
    fn a_ticket_counts_edits_since_the_later_of_its_own_and_its_specs_trusted_labellings() {
        for (ticket_labellings, spec_labellings, trusted) in [
            // The Spec's labelling is later than the Ticket's.
            (vec![by("acme", 1)], vec![by("acme", 3)], true),
            // The Ticket's own labelling is later than the Spec's.
            (vec![by("acme", 3)], vec![by("acme", 1)], true),
            // Neither's labelling is late enough.
            (vec![by("acme", 1)], vec![by("acme", 1)], false),
            // Only trusted labellings count, not a later untrusted one.
            (vec![], vec![by("acme", 1), by("outsider", 4)], false),
        ] {
            let reads = InMemory::with(Issue {
                labellings: ticket_labellings,
                body_edits: vec![by("outsider", 2)],
                ..Issue::default()
            })
            .and(
                SPEC,
                Issue {
                    labellings: spec_labellings,
                    ..Issue::default()
                },
            );

            let expected = (!trusted).then_some(UntrustedIssue::Edited(IssueText::Body));
            assert_eq!(ticket_trust_of(&reads), expected);
        }
    }

    #[test]
    fn a_ticket_with_no_trusted_labelling_counts_every_edit() {
        for (ticket_labellings, edit, trusted) in [
            (vec![], by("outsider", 1), false),
            (vec![by("outsider", 3)], by("outsider", 1), false),
            (vec![], by("reader", 1), true),
        ] {
            let reads = InMemory::with(Issue {
                labellings: ticket_labellings,
                title_edits: vec![edit],
                ..Issue::default()
            })
            .and(SPEC, Issue::default());

            let expected = (!trusted).then_some(UntrustedIssue::Edited(IssueText::Title));
            assert_eq!(ticket_trust_of(&reads), expected);
        }
    }

    #[test]
    fn a_ticket_by_an_untrusted_author_is_untrusted() {
        let reads = InMemory::with(Issue {
            association: Some("NONE"),
            ..Issue::default()
        })
        .and(SPEC, ready());

        assert_eq!(ticket_trust_of(&reads), Some(UntrustedIssue::Author));
        assert!(reads.looked_up.borrow().is_empty());
    }

    #[test]
    fn the_repository_owner_is_trusted_in_any_case_with_no_read() {
        let reads = InMemory::with(Issue {
            labellings: vec![by("ACME", 1)],
            body_edits: vec![by("Acme", 2)],
            title_edits: vec![by("acme", 2)],
            ..Issue::default()
        });

        assert_eq!(issue_trust_of(&reads), None);
        assert!(reads.looked_up.borrow().is_empty());
    }

    #[test]
    fn an_account_found_trusted_is_not_looked_up_again_within_one_check() {
        let reads = InMemory::with(Issue {
            labellings: vec![by("reader", 1)],
            body_edits: vec![by("reader", 2), by("READER", 3)],
            title_edits: vec![by("reader", 2), by("member", 3), by("member", 4)],
            ..Issue::default()
        });

        assert_eq!(issue_trust_of(&reads), None);
        assert_eq!(*reads.looked_up.borrow(), ["reader", "member"]);
    }

    #[test]
    fn a_failed_read_propagates_and_never_grants_trust() {
        for issue in [
            Issue {
                labellings: vec![by("broken", 1)],
                ..Issue::default()
            },
            Issue {
                body_edits: vec![by("broken", 2)],
                ..ready()
            },
        ] {
            let reads = InMemory::with(issue);

            let error = issue_trust(&reads, &url(ISSUE)).unwrap_err();

            assert!(
                format!("{error:#}").contains("could not check Trusted account membership"),
                "{error:#}"
            );
        }
    }

    #[test]
    fn untrusted_edit_with_no_time_counts_every_edit() {
        let reads = InMemory::with(Issue {
            title_edits: vec![by("outsider", 1)],
            ..Issue::default()
        });

        assert_eq!(
            untrusted_edit(&reads, &url(ISSUE), None).unwrap(),
            Some(IssueText::Title)
        );
        assert_eq!(
            untrusted_edit(&reads, &url(ISSUE), Some(at(1))).unwrap(),
            None
        );
    }
}
