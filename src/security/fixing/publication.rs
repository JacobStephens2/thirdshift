//! The complete Security fix publication protocol. The shell owns Sessions
//! and Worktrees; this operation owns the raw proposal through readiness.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

use super::SECURITY_FIX;
use crate::github::{GitHub, Issue, SecurityRecord, Ticket};
use crate::issue::{IssueUrl, Repo};
use crate::labels::{Edit, Label, Labels, NEEDS_TRIAGE, READY_FOR_AGENT};
use crate::prompt;
use crate::security::reproduction::FixSize;

/// Facts captured by the shell, without Session or clock ownership here.
pub(super) enum Source<'a> {
    ExistingFinding,
    Session {
        message: Option<&'a str>,
        started: DateTime<Utc>,
    },
}

/// GitHub effects and interruption observations, shared with the scripted adapter.
pub(super) trait Outside {
    fn issue(&mut self, issue: &IssueUrl) -> Result<Issue>;
    fn tickets(&mut self, issue: &IssueUrl) -> Result<Vec<Ticket>>;
    fn issue_labels(&mut self, issue: &IssueUrl) -> Result<Labels>;
    fn issue_text(&mut self, issue: &IssueUrl) -> Result<String>;
    fn issues_created_since(&mut self, repo: &str, since: DateTime<Utc>) -> Result<Vec<IssueUrl>>;
    fn create_issue(
        &mut self,
        repo: &Repo,
        title: &str,
        body: &str,
        labels: &[Label],
    ) -> Result<IssueUrl>;
    fn close_issue(&mut self, issue: &IssueUrl, comment: &str) -> Result<()>;
    fn add_sub_issue(&mut self, parent: &IssueUrl, child: &IssueUrl) -> Result<()>;
    fn add_blocked_by(&mut self, ticket: &IssueUrl, blocker: &IssueUrl) -> Result<()>;
    fn apply(&mut self, edit: &Edit) -> Result<()>;
    fn interrupted(&mut self) -> bool;
}

impl Outside for GitHub {
    fn issue(&mut self, issue: &IssueUrl) -> Result<Issue> {
        GitHub::issue(self, issue)
    }
    fn tickets(&mut self, issue: &IssueUrl) -> Result<Vec<Ticket>> {
        GitHub::tickets(self, issue)
    }
    fn issue_labels(&mut self, issue: &IssueUrl) -> Result<Labels> {
        GitHub::issue_labels(self, issue)
    }
    fn issue_text(&mut self, issue: &IssueUrl) -> Result<String> {
        self.security_fix_text(issue)
    }
    fn issues_created_since(&mut self, repo: &str, since: DateTime<Utc>) -> Result<Vec<IssueUrl>> {
        GitHub::issues_created_since(self, repo, since)
    }
    fn create_issue(
        &mut self,
        repo: &Repo,
        title: &str,
        body: &str,
        labels: &[Label],
    ) -> Result<IssueUrl> {
        self.create_issue_in(repo, title, body, labels)
    }
    fn close_issue(&mut self, issue: &IssueUrl, comment: &str) -> Result<()> {
        GitHub::close_issue(self, issue, comment)
    }
    fn add_sub_issue(&mut self, parent: &IssueUrl, child: &IssueUrl) -> Result<()> {
        GitHub::add_sub_issue(self, parent, child)
    }
    fn add_blocked_by(&mut self, ticket: &IssueUrl, blocker: &IssueUrl) -> Result<()> {
        GitHub::add_blocked_by(self, ticket, blocker)
    }
    fn apply(&mut self, edit: &Edit) -> Result<()> {
        edit.apply(self)
    }
    fn interrupted(&mut self) -> bool {
        crate::interrupt::requested()
    }
}

/// Publish or reuse the finding's fix through the complete validation protocol.
pub(super) fn publish(
    github: &mut impl Outside,
    repo: &Repo,
    record: &SecurityRecord,
    url: &str,
    size: FixSize,
    source: Source<'_>,
) -> Result<IssueUrl> {
    let (message, started) = match source {
        Source::ExistingFinding => {
            let number = record
                .private_issue_number()
                .context("the Security finding has no private issue")?;
            let issue = IssueUrl::parse(url)?;
            if issue.number != number || !issue.repo_slug().eq_ignore_ascii_case(&repo.slug()) {
                bail!("the private Security finding's issue does not match its record");
            }
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
            mark_ready(github, &issue, viewed.labels)?;
            return Ok(issue);
        }
        Source::Session { message, started } => (message, started),
    };
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
            return fail_on_side_effects(github, &leaked);
        }
        if size == FixSize::Single {
            let Some((title, body)) = message.and_then(parse_staged_proposal) else {
                bail!(
                    "the Security fix publishing session did not stage its proposed public issue text"
                );
            };
            // Validate the staged text before any public issue exists;
            // a rejection blocks creation, so no public issue leaks
            // private write-up text.
            record.check_public_fix_text(&format!("{title}\n\n{body}"), url)?;
            github.create_issue(repo, &title, &body, &[NEEDS_TRIAGE])?
        } else {
            publish_staged_spec(github, repo, record, url, message, started)?
        }
    } else {
        let line = message
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
        check_issue_text(github, &issue, record, url)?;
    }
    for child in &tickets {
        if !issue.in_same_repo(&child.issue) {
            bail!("a Security fix Spec's Ticket must be in the fix issue's repository");
        }
        let issue = &child.issue;
        let viewed = github.issue(issue)?;
        if !viewed.is_open || viewed.created.timestamp() < started.timestamp() {
            bail!("a Security fix Spec's Ticket must be new and open");
        }
        if child.has_sub_issues
            || !viewed.labels.has(READY_FOR_AGENT)
            || viewed.labels.unready().is_some()
        {
            bail!("a Security fix Spec's Ticket must be ready-for-agent with no sub-issues");
        }
        check_issue_text(github, issue, record, url)?;
    }
    for child in tickets {
        let issue = &child.issue;
        let labels = github.issue_labels(issue)?;
        github.apply(&Edit::of(issue, labels, &[], &[SECURITY_FIX]))?;
    }
    mark_ready(github, &issue, viewed.labels)?;
    Ok(issue)
}

/// Make the final readiness edit, after the flow's checks and child tagging.
fn mark_ready(github: &mut impl Outside, issue: &IssueUrl, labels: Labels) -> Result<()> {
    if github.interrupted() {
        bail!("interrupted");
    }
    github.apply(&Edit::of(
        issue,
        labels,
        &[NEEDS_TRIAGE],
        &[READY_FOR_AGENT, SECURITY_FIX],
    ))
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
/// positions of earlier Tickets on a single line, since each later Ticket
/// names the Tickets that block it. Anything but blank lines after that
/// first line is malformed staged text: ignoring it would silently drop a
/// stated ordering edge, so validation fails closed instead. `None` unless
/// every position names an earlier Ticket of the `count` staged: a forward
/// or cyclic edge would dispatch Tickets that can never unblock.
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
        if position == 0 || position > count || position > index {
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
    github: &mut impl Outside,
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
    let spec = github.create_issue(repo, &staged.title, &staged.body, &[NEEDS_TRIAGE])?;
    let mut tickets = Vec::with_capacity(staged.tickets.len());
    for ticket in &staged.tickets {
        tickets.push(github.create_issue(repo, &ticket.title, &ticket.body, &[READY_FOR_AGENT])?);
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
fn fail_on_side_effects(github: &mut impl Outside, leaks: &[IssueUrl]) -> Result<IssueUrl> {
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
    github: &mut impl Outside,
    issue: &IssueUrl,
    record: &SecurityRecord,
    url: &str,
) -> Result<()> {
    let body = github.issue_text(issue)?;
    record.check_public_fix_text(&body, url)
}

#[cfg(test)]
mod tests;
