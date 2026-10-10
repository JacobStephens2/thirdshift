//! Publish and check a terse fix Ticket or Spec, without exposing the private write-up.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

use crate::git::Git;
use crate::github::{GitHub, SecurityRecord};
use crate::harness::Choice;
use crate::issue::{IssueUrl, Repo};
use crate::labels::{Edit, Label, NEEDS_TRIAGE, READY_FOR_AGENT};
use crate::prompt;
use crate::security::reproduction::FixSize;
use crate::session::{Logs, Purpose, Sessions};
use crate::worktree::ReviewWorktree;

pub const SECURITY_FIX: Label =
    Label::new("security-fix", "A fix for a reproduced Security finding");

pub fn publish(
    launch: &Git,
    repo: &Repo,
    base: &str,
    record: &SecurityRecord,
    url: &str,
    harness: &Choice,
) -> (Result<IssueUrl>, Option<PathBuf>) {
    let size = match record.fix_size() {
        Ok(size) => size,
        Err(error) => return (Err(error), None),
    };
    if let Some(number) = record.private_issue_number()
        && size == FixSize::Single
    {
        let ready = (|| -> Result<IssueUrl> {
            let issue = IssueUrl::parse(url)?;
            if issue.number != number || !issue.repo_slug().eq_ignore_ascii_case(&repo.slug()) {
                bail!("the private Security finding's issue does not match its record");
            }
            let github = GitHub::new();
            let viewed = github.issue(&issue)?;
            if !viewed.is_open
                || viewed
                    .labels
                    .swapped(&[NEEDS_TRIAGE], &[])
                    .unready()
                    .is_some()
                || !github.tickets(&issue)?.is_empty()
            {
                bail!("the private Security finding must be an open, ready single Ticket");
            }
            if crate::interrupt::requested() {
                bail!("interrupted");
            }
            Edit::of(
                &issue,
                viewed.labels,
                &[NEEDS_TRIAGE],
                &[READY_FOR_AGENT, SECURITY_FIX],
            )
            .apply(&github)?;
            Ok(issue)
        })();
        return (ready, None);
    }
    let worktree = match ReviewWorktree::create(launch, &repo.name, base) {
        Ok(worktree) => worktree,
        Err(error) => return (Err(error), None),
    };
    let logs = Logs::of_security_run(repo);
    let started = Utc::now();
    let prompt = prompt::security_fix(base, url, record.description());
    Sessions::within(&logs, worktree.path(), harness, |sessions| {
        let message =
            sessions.run_to_final_message(Purpose::Security, "security-fix-publishing", &prompt)?;
        let github = GitHub::new();
        // A public fix stages its proposed text for validation;
        // thirdshift creates the issues from it, and the session has no
        // independent public-issue path. A private bigger fix keeps the
        // session-created issue path: its issues stay private with the
        // repository.
        let issue = if record.private_issue_number().is_none() {
            // The session has no independent public-issue path: anything
            // created during its window is a side-effect leak outside the
            // validated staged path. Close it to stop further interaction,
            // then fail loudly so the Day shift triages the exposure.
            let leaked = github.issues_created_since(&repo.slug(), started)?;
            if !leaked.is_empty() {
                return fail_on_side_effects(&github, &leaked);
            }
            if size == FixSize::Single {
                let Some((title, body)) = message.as_deref().and_then(parse_staged_proposal) else {
                    bail!(
                        "the Security fix publishing session did not stage its proposed public issue text"
                    );
                };
                // Validate the staged text before any public issue exists;
                // a rejection blocks creation, so no public issue leaks
                // private write-up text.
                record.check_public_fix_text(&format!("{title}\n\n{body}"), url)?;
                github.create_issue_in(repo, &title, &body, &[NEEDS_TRIAGE])?
            } else {
                publish_staged_spec(&github, repo, record, url, message.as_deref(), started)?
            }
        } else {
            let line = message
                .as_deref()
                .unwrap_or_default()
                .lines()
                .map(str::trim)
                .rfind(|line| !line.is_empty())
                .unwrap_or_default();
            let prefix = match size {
                FixSize::Single => prompt::SECURITY_FIX_LINE,
                FixSize::Spec => prompt::SECURITY_FIX_SPEC_LINE,
            };
            let issue = line.strip_prefix(prefix).and_then(|url| IssueUrl::parse(url).ok()).context("the Security fix publishing session ended without the final line its prompt asks for")?;
            if !issue.repo_slug().eq_ignore_ascii_case(&repo.slug()) {
                bail!("the Security fix Ticket is not in this repository");
            }
            issue
        };
        let private = record.private_issue_number().is_some();
        if let Some(number) = record.private_issue_number()
            && issue.number != number
        {
            bail!("a private Security fix must reuse the finding's own issue");
        }
        let viewed = github.issue(&issue)?;
        if !viewed.is_open {
            bail!("the Security fix Ticket is closed");
        }
        if !private && viewed.created.timestamp() < started.timestamp() {
            bail!("the Security fix Ticket was created before this session");
        }
        if !private && !viewed.labels.has(NEEDS_TRIAGE)
            || viewed
                .labels
                .swapped(&[NEEDS_TRIAGE], &[])
                .unready()
                .is_some()
        {
            bail!(
                "the Security fix Ticket must be labelled needs-triage with no other Unready Ticket label"
            );
        }
        let tickets = github.tickets(&issue)?;
        if (size == FixSize::Spec) == tickets.is_empty() {
            bail!("the Security fix's sub-issues do not match the reproduced fix size");
        }
        if !private {
            check_issue_text(&github, &issue, record, url)?;
        }
        for child in &tickets {
            let issue = issue.sibling(child.number);
            let viewed = github.issue(&issue)?;
            if !viewed.is_open || viewed.created.timestamp() < started.timestamp() {
                bail!("a Security fix Spec's Ticket must be new and open");
            }
            if child.has_sub_issues
                || !viewed.labels.has(READY_FOR_AGENT)
                || viewed.labels.unready().is_some()
            {
                bail!("a Security fix Spec's Ticket must be ready-for-agent with no sub-issues");
            }
            check_issue_text(&github, &issue, record, url)?;
        }
        for child in tickets {
            let issue = issue.sibling(child.number);
            Edit::of(&issue, github.issue_labels(&issue)?, &[], &[SECURITY_FIX]).apply(&github)?;
        }
        if crate::interrupt::requested() {
            bail!("interrupted");
        }
        Edit::of(
            &issue,
            viewed.labels,
            &[NEEDS_TRIAGE],
            &[READY_FOR_AGENT, SECURITY_FIX],
        )
        .apply(&github)?;
        Ok(issue)
    })
}

/// One staged Spec Ticket the publishing session proposed: its title, its
/// body, and the 0-based indices of the Tickets it is blocked by.
struct StagedTicket {
    title: String,
    body: String,
    blocked_by: Vec<usize>,
}

/// The public Spec the publishing session staged for validation: its
/// proposed top-issue `(title, body)` and its Tickets, instead of
/// session-created public issues.
struct StagedSpec {
    title: String,
    body: String,
    tickets: Vec<StagedTicket>,
}

/// The staged Spec the publishing session proposed, with every Ticket's
/// blocking edges as 0-based indices. `None` when the final message holds
/// no well-formed staged spec: no marker, no top title or body, no Ticket,
/// an empty title or body, or a blocking edge that is not `none` or a list
/// of 1-based Ticket positions naming another Ticket.
fn parse_staged_spec_proposal(message: &str) -> Option<StagedSpec> {
    let (_, staged) = message.rsplit_once(prompt::STAGED_SPEC_MARKER)?;
    let staged = staged.trim_start_matches(['\n', '\r']);
    let (title_line, rest) = staged.split_once('\n')?;
    let title = title_line.strip_prefix(prompt::STAGED_TITLE_MARKER)?.trim();
    let rest = rest.strip_prefix(prompt::STAGED_BODY_MARKER)?;
    let (spec_body, tickets_text) = match rest.split_once(prompt::STAGED_TICKET_MARKER) {
        Some((body, tickets)) => (body, Some(tickets)),
        None => (rest, None),
    };
    let tickets_text = tickets_text?;
    if title.is_empty() || spec_body.trim().is_empty() {
        return None;
    }
    let mut tickets = Vec::new();
    for section in tickets_text.split(prompt::STAGED_TICKET_MARKER) {
        let (title_line, rest) = section.split_once('\n')?;
        let title = title_line.trim();
        if title.is_empty() {
            return None;
        }
        let rest = rest.strip_prefix(prompt::STAGED_BODY_MARKER)?;
        let (body, blocked) = rest.split_once(prompt::STAGED_BLOCKED_BY_MARKER)?;
        if body.trim().is_empty() {
            return None;
        }
        tickets.push((title.to_string(), body.trim().to_string(), blocked));
    }
    if tickets.is_empty() {
        return None;
    }
    let count = tickets.len();
    let mut parsed = Vec::with_capacity(count);
    for (index, (title, body, blocked)) in tickets.into_iter().enumerate() {
        parsed.push(StagedTicket {
            title,
            body,
            blocked_by: parse_blocked_by(blocked, index, count)?,
        });
    }
    Some(StagedSpec {
        title: title.to_string(),
        body: spec_body.trim().to_string(),
        tickets: parsed,
    })
}

/// The `Blocked by:` trailer of one staged Ticket as 0-based indices into
/// its Spec's Tickets: `none` is empty, else comma-separated 1-based
/// positions on a single line. Anything but blank lines after that first
/// line is malformed staged text: ignoring it would silently drop a stated
/// ordering edge, so validation fails closed instead. `None` unless every
/// position names another Ticket of the `count` staged.
fn parse_blocked_by(blocked: &str, index: usize, count: usize) -> Option<Vec<usize>> {
    let mut lines = blocked.lines();
    let line = lines.next().unwrap_or_default().trim();
    if lines.any(|line| !line.trim().is_empty()) {
        return None;
    }
    let line = line.trim_end_matches(['.', ';']);
    if line.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut edges = Vec::new();
    for position in line
        .split(',')
        .map(str::trim)
        .filter(|edge| !edge.is_empty())
    {
        let position: usize = position.parse().ok()?;
        if position == 0 || position > count || position - 1 == index {
            return None;
        }
        let edge = position - 1;
        if !edges.contains(&edge) {
            edges.push(edge);
        }
    }
    if edges.is_empty() {
        return None;
    }
    Some(edges)
}

/// Validate the staged Spec proposal and create its public issues: the
/// `needs-triage` top issue and one `ready-for-agent` Ticket per staged
/// Ticket, linked as native sub-issues with their staged blocking edges.
/// Every staged text is validated before any public issue exists, so a
/// rejection creates nothing and leaks no private write-up text.
fn publish_staged_spec(
    github: &GitHub,
    repo: &Repo,
    record: &SecurityRecord,
    url: &str,
    message: Option<&str>,
    started: DateTime<Utc>,
) -> Result<IssueUrl> {
    let Some(staged) = message.and_then(parse_staged_spec_proposal) else {
        bail!("the Security fix publishing session did not stage its proposed public spec text");
    };
    record.check_public_fix_text(&format!("{}\n\n{}", staged.title, staged.body), url)?;
    for ticket in &staged.tickets {
        record.check_public_fix_text(&format!("{}\n\n{}", ticket.title, ticket.body), url)?;
    }
    let spec = github.create_issue_in(repo, &staged.title, &staged.body, &[NEEDS_TRIAGE])?;
    let mut tickets = Vec::with_capacity(staged.tickets.len());
    for ticket in &staged.tickets {
        tickets.push(github.create_issue_in(
            repo,
            &ticket.title,
            &ticket.body,
            &[READY_FOR_AGENT],
        )?);
    }
    for ticket in &tickets {
        github.add_sub_issue(&spec, ticket)?;
    }
    for (ticket, staged) in tickets.iter().zip(&staged.tickets) {
        for edge in &staged.blocked_by {
            github.add_blocked_by(ticket, &tickets[*edge])?;
        }
    }
    // The session has no independent public-issue path, so anything in its
    // window that is neither the staged spec nor one of its Tickets is a
    // side-effect leak outside the validated staged path. The pre-creation
    // sweep runs before any staged issue exists; this one runs after
    // linking, so the staged issues themselves must not count.
    let unlinked = unlinked_issues(
        github.issues_created_since(&repo.slug(), started)?,
        &spec,
        &tickets,
    );
    if !unlinked.is_empty() {
        return fail_on_side_effects(github, &unlinked);
    }
    Ok(spec)
}

/// The issues in the publishing session's `window` that are neither the
/// staged `spec` nor one of its `tickets`: side-effect leaks outside the
/// validated staged path.
fn unlinked_issues(window: Vec<IssueUrl>, spec: &IssueUrl, tickets: &[IssueUrl]) -> Vec<IssueUrl> {
    window
        .into_iter()
        .filter(|issue| {
            issue.number != spec.number
                && !tickets.iter().any(|ticket| ticket.number == issue.number)
        })
        .collect()
}

/// Close `leaks`, public issues a Security fix publishing session created
/// outside the validated staged path, then fail loudly so the Day shift
/// triages the exposure.
fn fail_on_side_effects(github: &GitHub, leaks: &[IssueUrl]) -> Result<IssueUrl> {
    for leak in leaks {
        github.close_issue(leak, "Closed by thirdshift: the Security fix publishing session created this public issue outside the validated staged path.")?;
    }
    let numbers = leaks
        .iter()
        .map(|leak| format!("#{}", leak.number))
        .collect::<Vec<_>>()
        .join(", ");
    bail!(
        "the Security fix publishing session created public issue(s) outside the staged path: {numbers}"
    )
}

/// The single public fix the publishing session staged for validation:
/// its proposed `(title, body)`, instead of a session-created public issue.
fn parse_staged_proposal(message: &str) -> Option<(String, String)> {
    let (_, staged) = message.rsplit_once(prompt::STAGED_PROPOSAL_MARKER)?;
    let staged = staged.trim_start_matches(['\n', '\r']);
    let (title, body) = staged.split_once('\n')?;
    let title = title.strip_prefix(prompt::STAGED_TITLE_MARKER)?.trim();
    let body = body
        .strip_prefix(prompt::STAGED_BODY_MARKER)
        .map(|body| body.trim().to_string())?;
    if title.is_empty() || body.is_empty() {
        return None;
    }
    Some((title.to_string(), body))
}

fn check_issue_text(
    github: &GitHub,
    issue: &IssueUrl,
    record: &SecurityRecord,
    url: &str,
) -> Result<()> {
    let body = github.security_fix_text(issue)?;
    record.check_public_fix_text(&body, url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_proposal_parses_title_and_body() {
        let message = "Some reasoning\nProposed public issue:\nTitle: Bound accepted input\nBody:\nReject oversized input. Private record: https://example.invalid/advisory\n";
        let (title, body) = parse_staged_proposal(message).expect("staged proposal parses");
        assert_eq!(title, "Bound accepted input");
        assert!(body.contains("Reject oversized input."));
    }

    #[test]
    fn staged_proposal_rejects_a_bare_url_final_line() {
        let message = "Security fix Ticket: https://github.com/acme/widgets/issues/8\n";
        assert!(parse_staged_proposal(message).is_none());
    }

    #[test]
    fn staged_spec_parses_top_issue_and_tickets_with_blocking() {
        let message = "Reasoning\nProposed public spec:\nTitle: Bound accepted input\nBody:\nBound input across storage and transport. Private record: https://example.invalid/advisory\nTicket: Bound storage input\nBody:\nBound storage input. Private record: https://example.invalid/advisory\nBlocked by: none\nTicket: Bound transport input\nBody:\nBound transport input. Private record: https://example.invalid/advisory\nBlocked by: 1\n";
        let spec = parse_staged_spec_proposal(message).expect("staged spec parses");
        assert_eq!(spec.title, "Bound accepted input");
        assert!(spec.body.contains("storage and transport"));
        assert_eq!(spec.tickets.len(), 2);
        assert_eq!(spec.tickets[0].title, "Bound storage input");
        assert!(spec.tickets[0].blocked_by.is_empty());
        assert_eq!(spec.tickets[1].title, "Bound transport input");
        assert_eq!(spec.tickets[1].blocked_by, vec![0]);
    }

    #[test]
    fn staged_spec_rejects_a_bare_url_final_line() {
        let message = "Security fix Spec: https://github.com/acme/widgets/issues/8\n";
        assert!(parse_staged_spec_proposal(message).is_none());
    }

    #[test]
    fn unlinked_issues_keeps_only_issues_outside_the_staged_spec() {
        let issue = |number: u64| {
            IssueUrl::parse(&format!("https://github.com/acme/widgets/issues/{number}")).unwrap()
        };
        let spec = issue(8);
        let tickets = vec![issue(9), issue(10)];
        let window = vec![issue(8), issue(9), issue(10), issue(11)];
        assert_eq!(unlinked_issues(window, &spec, &tickets), vec![issue(11)]);
        let window = vec![issue(8), issue(9), issue(10)];
        assert!(unlinked_issues(window, &spec, &tickets).is_empty());
    }

    #[test]
    fn staged_spec_rejects_a_trailing_blocked_by_edge_on_its_own_line() {
        let message = "Proposed public spec:\nTitle: Bound accepted input\nBody:\nBound input. Private record: https://example.invalid/advisory\nTicket: First ticket\nBody:\nFirst ticket. Private record: https://example.invalid/advisory\nBlocked by: none\nTicket: Second ticket\nBody:\nSecond ticket. Private record: https://example.invalid/advisory\nBlocked by: none\nTicket: Third ticket\nBody:\nThird ticket. Private record: https://example.invalid/advisory\nBlocked by: 1\n2\n";
        assert!(
            parse_staged_spec_proposal(message).is_none(),
            "a second Blocked by edge on its own line was silently dropped instead of rejected"
        );
    }

    #[test]
    fn staged_spec_rejects_bad_blocking_edges() {
        for message in [
            "Proposed public spec:\nTitle: Bound accepted input\nBody:\nBound input. Private record: https://example.invalid/advisory\nTicket: Only ticket\nBody:\nOnly ticket. Private record: https://example.invalid/advisory\nBlocked by: 1\n",
            "Proposed public spec:\nTitle: Bound accepted input\nBody:\nBound input. Private record: https://example.invalid/advisory\nTicket: Only ticket\nBody:\nOnly ticket. Private record: https://example.invalid/advisory\nBlocked by: 2\n",
            "Proposed public spec:\nTitle: Bound accepted input\nBody:\nBound input. Private record: https://example.invalid/advisory\n",
        ] {
            assert!(
                parse_staged_spec_proposal(message).is_none(),
                "bad staged spec parses: {message}"
            );
        }
    }
}
