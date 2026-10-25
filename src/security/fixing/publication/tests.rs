use std::collections::{BTreeMap, VecDeque};

use serde_json::json;

use super::*;
use crate::github::SecurityRecords;
use crate::issue::IssueIdentity;

const ADVISORY: &str = "https://github.com/acme/widgets/security/advisories/GHSA-abcd-1234-5678";
const SECRET: &str = "A private exploit bypasses the input boundary.";
const SINGLE: &str = "Reasoning\nProposed public issue:\nTitle: Bound accepted input\nBody:\nReject oversized input. Private record: ";

fn issue(number: u64) -> IssueUrl {
    IssueUrl::parse(&format!("https://github.com/acme/widgets/issues/{number}")).unwrap()
}

fn started() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-16T12:00:00.900Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn record(private: bool, size: FixSize) -> SecurityRecord {
    let description = format!(
        "{SECRET}\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced high {}\n",
        size.name()
    );
    if private {
        SecurityRecords::issues(Vec::new()).decode_created(json!({
            "number": 8, "title": "Private finding", "body": description,
            "html_url": issue(8).url, "state": "open", "labels": [{"name": "needs-triage"}]
        }))
    } else {
        SecurityRecords::advisories(Vec::new()).decode_created(json!({
            "ghsa_id": "GHSA-abcd-1234-5678", "summary": "Private finding",
            "description": description, "html_url": ADVISORY, "state": "draft"
        }))
    }
    .unwrap()
}

struct Stored {
    url: IssueUrl,
    view: Issue,
    text: String,
    children: Vec<IssueUrl>,
    blockers: Vec<IssueUrl>,
}

/// Native issue state plus scripted observations/failures at the effects boundary.
struct InMemory {
    issues: BTreeMap<IssueIdentity, Stored>,
    events: Vec<String>,
    reads: BTreeMap<IssueIdentity, VecDeque<Issue>>,
    label_reads: BTreeMap<IssueIdentity, Labels>,
    text_reads: BTreeMap<IssueIdentity, String>,
    fail: Option<String>,
    interrupted: bool,
    windows: VecDeque<Vec<IssueUrl>>,
    sweeps: usize,
    next: u64,
}

impl InMemory {
    fn new() -> Self {
        Self {
            issues: BTreeMap::new(),
            events: Vec::new(),
            reads: BTreeMap::new(),
            label_reads: BTreeMap::new(),
            text_reads: BTreeMap::new(),
            fail: None,
            interrupted: false,
            windows: VecDeque::new(),
            sweeps: 0,
            next: 8,
        }
    }

    fn observe(&mut self, event: String) -> Result<()> {
        self.events.push(event.clone());
        if self.fail.as_ref() == Some(&event) {
            bail!("failed {event}");
        }
        Ok(())
    }

    fn get(&self, url: &IssueUrl) -> &Stored {
        &self.issues[&url.identity()]
    }
    fn get_mut(&mut self, url: &IssueUrl) -> &mut Stored {
        self.issues.get_mut(&url.identity()).unwrap()
    }

    fn insert(&mut self, url: IssueUrl, labels: &[&str], text: &str) {
        self.issues.insert(
            url.identity(),
            Stored {
                url,
                view: Issue {
                    title: Some("Bound input".into()),
                    is_open: true,
                    labels: labels.iter().copied().collect(),
                    created: started(),
                },
                text: text.into(),
                children: Vec::new(),
                blockers: Vec::new(),
            },
        );
    }

    fn session(&mut self, private: bool, size: FixSize, message: Option<&str>) -> Result<IssueUrl> {
        let record = record(private, size);
        let url = if private {
            issue(8).url
        } else {
            ADVISORY.into()
        };
        publish(
            self,
            &issue(8).repo(),
            &record,
            &url,
            record.fix_size().unwrap(),
            Source::Session {
                message,
                started: started(),
            },
        )
    }

    fn has(&self, number: u64, label: Label) -> bool {
        self.get(&issue(number)).view.labels.has(label)
    }
}

impl Outside for InMemory {
    fn issue(&mut self, url: &IssueUrl) -> Result<Issue> {
        self.observe(format!("issue {}", url.number))?;
        if let Some(view) = self
            .reads
            .get_mut(&url.identity())
            .and_then(VecDeque::pop_front)
        {
            self.get_mut(url).view = view;
        }
        Ok(self.get(url).view.clone())
    }
    fn tickets(&mut self, url: &IssueUrl) -> Result<Vec<Ticket>> {
        self.observe(format!("tickets {}", url.number))?;
        Ok(self
            .get(url)
            .children
            .iter()
            .map(|child| {
                let stored = self.get(child);
                Ticket {
                    issue: child.clone(),
                    is_open: stored.view.is_open,
                    labels: stored.view.labels.clone(),
                    has_sub_issues: !stored.children.is_empty(),
                    blockers: stored.blockers.clone(),
                    open_blockers: stored
                        .blockers
                        .iter()
                        .filter(|url| self.get(url).view.is_open)
                        .cloned()
                        .collect(),
                }
            })
            .collect())
    }
    fn issue_labels(&mut self, url: &IssueUrl) -> Result<Labels> {
        self.observe(format!("labels {}", url.number))?;
        if let Some(labels) = self.label_reads.remove(&url.identity()) {
            self.get_mut(url).view.labels = labels;
        }
        Ok(self.get(url).view.labels.clone())
    }
    fn issue_text(&mut self, url: &IssueUrl) -> Result<String> {
        self.observe(format!("text {}", url.number))?;
        if let Some(text) = self.text_reads.remove(&url.identity()) {
            self.get_mut(url).text = text;
        }
        Ok(self.get(url).text.clone())
    }
    fn issues_created_since(&mut self, repo: &str, since: DateTime<Utc>) -> Result<Vec<IssueUrl>> {
        self.sweeps += 1;
        self.observe(format!("sweep {}", self.sweeps))?;
        Ok(self.windows.pop_front().unwrap_or_else(|| {
            self.issues
                .values()
                .filter(|stored| {
                    stored.url.repo_slug().eq_ignore_ascii_case(repo)
                        && stored.view.created.timestamp() >= since.timestamp()
                })
                .map(|stored| stored.url.clone())
                .collect()
        }))
    }
    fn create_issue(
        &mut self,
        repo: &Repo,
        title: &str,
        body: &str,
        labels: &[Label],
    ) -> Result<IssueUrl> {
        let number = self.next;
        self.observe(format!("create {number}"))?;
        self.next += 1;
        let url = IssueUrl::parse(&format!(
            "https://github.com/{}/issues/{number}",
            repo.slug()
        ))?;
        self.insert(
            url.clone(),
            &labels.iter().map(|label| label.name()).collect::<Vec<_>>(),
            &format!("{title}\n\n{body}"),
        );
        self.get_mut(&url).view.title = Some(title.into());
        Ok(url)
    }
    fn close_issue(&mut self, url: &IssueUrl, comment: &str) -> Result<()> {
        self.observe(format!("close {}", url.number))?;
        assert_eq!(
            comment,
            "Closed by thirdshift: the Security fix publishing session created this public issue outside the validated staged path."
        );
        self.get_mut(url).view.is_open = false;
        Ok(())
    }
    fn add_sub_issue(&mut self, parent: &IssueUrl, child: &IssueUrl) -> Result<()> {
        self.observe(format!("link {} {}", parent.number, child.number))?;
        self.get_mut(parent).children.push(child.clone());
        Ok(())
    }
    fn add_blocked_by(&mut self, ticket: &IssueUrl, blocker: &IssueUrl) -> Result<()> {
        self.observe(format!("block {} {}", ticket.number, blocker.number))?;
        self.get_mut(ticket).blockers.push(blocker.clone());
        Ok(())
    }
    fn apply(&mut self, edit: &Edit) -> Result<()> {
        self.observe(format!("edit {}", edit.issue().number))?;
        self.get_mut(edit.issue()).view.labels = edit.labels_after();
        Ok(())
    }
    fn interrupted(&mut self) -> bool {
        self.events.push("interrupted".into());
        self.interrupted
    }
}

#[test]
fn public_single_stages_checks_and_readies_the_created_ticket() {
    let mut github = InMemory::new();
    let result = github
        .session(
            false,
            FixSize::Single,
            Some(&format!("{SINGLE}{ADVISORY}\n")),
        )
        .unwrap();
    assert_eq!(result, issue(8));
    let stored = github.get(&result);
    assert_eq!(stored.view.title.as_deref(), Some("Bound accepted input"));
    assert_eq!(
        stored.text,
        format!("Bound accepted input\n\nReject oversized input. Private record: {ADVISORY}")
    );
    assert!(github.has(8, READY_FOR_AGENT));
    assert!(github.has(8, SECURITY_FIX));
    assert!(!github.has(8, NEEDS_TRIAGE));
    assert_eq!(
        github.events,
        [
            "sweep 1",
            "create 8",
            "issue 8",
            "tickets 8",
            "text 8",
            "interrupted",
            "edit 8"
        ]
    );
}

fn staged_spec() -> String {
    format!(
        "Reasoning\nProposed public spec:\nTitle: Bound accepted input\nBody:\nBound input across storage and transport. Private record: {ADVISORY}\nTicket: Bound storage input\nBody:\nBound storage input. Private record: {ADVISORY}\nBlocked by: none\nTicket: Bound transport input\nBody:\nBound transport input. Private record: {ADVISORY}\nBlocked by: 1, 1\n"
    )
}

fn private_spec() -> InMemory {
    let mut github = InMemory::new();
    github.insert(issue(8), &["Security-Finding", "bug"], SECRET);
    github.get_mut(&issue(8)).view.created = started() - chrono::Duration::days(1);
    for number in [9, 10] {
        github.insert(
            issue(number),
            &["READY-FOR-AGENT", "bug"],
            &format!("Bound input. Private record: {}", issue(8).url),
        );
    }
    github.get_mut(&issue(8)).children = vec![issue(9), issue(10)];
    github.get_mut(&issue(10)).blockers = vec![issue(9)];
    github
}

#[test]
fn public_spec_creates_links_and_deduplicates_blockers_before_validation_and_readiness() {
    let mut github = InMemory::new();
    assert_eq!(
        github
            .session(false, FixSize::Spec, Some(&staged_spec()))
            .unwrap(),
        issue(8)
    );
    assert_eq!(github.get(&issue(8)).children, [issue(9), issue(10)]);
    assert_eq!(github.get(&issue(10)).blockers, [issue(9)]);
    assert_eq!(
        github.get(&issue(9)).view.title.as_deref(),
        Some("Bound storage input")
    );
    assert_eq!(
        github.get(&issue(10)).view.title.as_deref(),
        Some("Bound transport input")
    );
    for number in [8, 9, 10] {
        assert!(github.has(number, READY_FOR_AGENT));
        assert!(github.has(number, SECURITY_FIX));
        assert!(github.get(&issue(number)).view.is_open);
        assert!(!github.has(number, NEEDS_TRIAGE));
    }
    assert_eq!(
        github.events,
        [
            "sweep 1",
            "create 8",
            "create 9",
            "create 10",
            "link 8 9",
            "link 8 10",
            "block 10 9",
            "sweep 2",
            "issue 8",
            "tickets 8",
            "text 8",
            "issue 9",
            "text 9",
            "issue 10",
            "text 10",
            "labels 9",
            "edit 9",
            "labels 10",
            "edit 10",
            "interrupted",
            "edit 8",
        ]
    );
}

#[test]
fn private_single_reuses_the_matching_old_finding_without_public_text_or_freshness_checks() {
    for labels in [vec!["Needs-Triage", "Security-Finding", "bug"], vec!["bug"]] {
        let mut github = InMemory::new();
        github.insert(issue(8), &labels, SECRET);
        github.get_mut(&issue(8)).view.created = started() - chrono::Duration::days(1);
        let record = record(true, FixSize::Single);
        let url = "https://github.com/ACME/Widgets/issues/8/";
        let result = publish(
            &mut github,
            &issue(8).repo(),
            &record,
            url,
            record.fix_size().unwrap(),
            Source::ExistingFinding,
        )
        .unwrap();
        assert_eq!(result.url, url);
        assert!(github.has(8, READY_FOR_AGENT));
        assert!(github.has(8, SECURITY_FIX));
        assert!(!github.has(8, NEEDS_TRIAGE));
        assert!(
            github
                .get(&issue(8))
                .view
                .labels
                .names()
                .any(|label| label == "bug")
        );
        assert_eq!(github.get(&issue(8)).text, SECRET);
        assert_eq!(
            github.events,
            ["issue 8", "tickets 8", "interrupted", "edit 8"]
        );
    }
}

#[test]
fn private_spec_reuses_the_old_parent_and_checks_new_children_before_tagging_them() {
    let mut github = private_spec();
    let child = IssueUrl::parse("https://github.com/ACME/Widgets/issues/9/").unwrap();
    github.get_mut(&issue(8)).children[0] = child;
    assert_eq!(
        github
            .session(
                true,
                FixSize::Spec,
                Some("Reasoning\nSecurity fix Spec: https://github.com/ACME/Widgets/issues/8/\n\n")
            )
            .unwrap()
            .number,
        8
    );
    assert_eq!(github.get(&issue(8)).text, SECRET);
    assert_eq!(github.get(&issue(10)).blockers, [issue(9)]);
    for number in [8, 9, 10] {
        assert!(github.has(number, READY_FOR_AGENT));
        assert!(github.has(number, SECURITY_FIX));
        assert!(
            github
                .get(&issue(number))
                .view
                .labels
                .names()
                .any(|label| label == "bug")
        );
    }
    assert_eq!(
        github.events,
        [
            "issue 8",
            "tickets 8",
            "issue 9",
            "text 9",
            "issue 10",
            "text 10",
            "labels 9",
            "edit 9",
            "labels 10",
            "edit 10",
            "interrupted",
            "edit 8"
        ]
    );
}

#[test]
fn malformed_public_messages_fail_before_creating_any_issue() {
    let single = format!("{SINGLE}{ADVISORY}");
    let spec = staged_spec();
    for (size, message) in [
        (FixSize::Single, None),
        (
            FixSize::Single,
            Some("Security fix Ticket: https://github.com/acme/widgets/issues/8".into()),
        ),
        (
            FixSize::Single,
            Some(single.replace("Title: Bound accepted input", "Title: ")),
        ),
        (
            FixSize::Single,
            Some("Proposed public issue:\nTitle: Bound input\nBody:\n \n".into()),
        ),
        (FixSize::Spec, None),
        (
            FixSize::Spec,
            Some("Security fix Spec: https://github.com/acme/widgets/issues/8".into()),
        ),
        (
            FixSize::Spec,
            Some(spec.replace("Title: Bound accepted input", "Title: ")),
        ),
        (
            FixSize::Spec,
            Some(spec.replace(
                &format!("Bound input across storage and transport. Private record: {ADVISORY}"),
                " ",
            )),
        ),
        (
            FixSize::Spec,
            Some(spec.replace("Ticket: Bound storage input", "Ticket: ")),
        ),
        (
            FixSize::Spec,
            Some(spec.replace(
                &format!("Bound transport input. Private record: {ADVISORY}"),
                " ",
            )),
        ),
        (
            FixSize::Spec,
            Some(spec.split("Ticket:").next().unwrap().into()),
        ),
    ] {
        let mut github = InMemory::new();
        let expected = if size == FixSize::Single {
            "issue"
        } else {
            "spec"
        };
        assert_eq!(
            github
                .session(false, size, message.as_deref())
                .unwrap_err()
                .to_string(),
            format!(
                "the Security fix publishing session did not stage its proposed public {expected} text"
            ),
            "{message:?}"
        );
        assert!(github.issues.is_empty());
        assert_eq!(github.events, ["sweep 1"]);
    }
}

#[test]
fn malformed_spec_blockers_fail_before_creating_any_issue() {
    for message in [
        staged_spec().replace("Blocked by: 1, 1", "Blocked by: 1\n2"),
        staged_spec().replace("Blocked by: 1, 1", "Blocked by: 2"),
        staged_spec().replace("Blocked by: 1, 1", "Blocked by: 3"),
        staged_spec().replace("Blocked by: 1, 1", "Blocked by: 0"),
        staged_spec().replace("Blocked by: 1, 1", "Blocked by: earlier"),
        staged_spec().replace("Blocked by: 1, 1", "Blocked by: "),
        staged_spec().replace("Blocked by: none", "Blocked by: 2"),
        staged_spec().replace("Blocked by: none", "Blocked by: 1"),
        staged_spec()
            .replace("Blocked by: none", "Blocked by: 2")
            .replace("Blocked by: 1, 1", "Blocked by: 1"),
    ] {
        let mut github = InMemory::new();
        assert_eq!(
            github
                .session(false, FixSize::Spec, Some(&message))
                .unwrap_err()
                .to_string(),
            "the Security fix publishing session did not stage its proposed public spec text",
            "{message}"
        );
        assert!(github.issues.is_empty());
        assert_eq!(github.events, ["sweep 1"]);
    }
}

#[test]
fn every_staged_title_and_body_is_checked_before_any_creation() {
    for (size, proposal, phrases) in [
        (
            FixSize::Single,
            format!("{SINGLE}{ADVISORY}"),
            vec!["Title: Bound accepted input", "Reject oversized input."],
        ),
        (
            FixSize::Spec,
            staged_spec(),
            vec![
                "Title: Bound accepted input",
                "Bound input across storage and transport.",
                "Ticket: Bound storage input",
                "Bound storage input.",
                "Ticket: Bound transport input",
                "Bound transport input.",
            ],
        ),
    ] {
        for phrase in phrases {
            let replacement = if phrase.starts_with("Title: ") {
                format!("Title: {SECRET}")
            } else if phrase.starts_with("Ticket: ") {
                format!("Ticket: {SECRET}")
            } else {
                SECRET.into()
            };
            let message = proposal.replace(phrase, &replacement);
            let mut github = InMemory::new();
            assert_eq!(
                github
                    .session(false, size, Some(&message))
                    .unwrap_err()
                    .to_string(),
                "the Security fix Ticket includes private write-up text",
                "{phrase}"
            );
            assert!(github.issues.is_empty(), "{phrase}");
            assert_eq!(github.events, ["sweep 1"]);
        }
        let mut github = InMemory::new();
        assert_eq!(
            github
                .session(false, size, Some(&proposal.replace(ADVISORY, "no link")))
                .unwrap_err()
                .to_string(),
            "the Security fix Ticket does not link the private record"
        );
        assert!(github.issues.is_empty());
    }
}

#[test]
fn a_private_single_rejects_mismatched_identity_before_reading_github() {
    for url in [
        "https://github.com/other/widgets/issues/8",
        "https://github.com/acme/other/issues/8",
        "https://github.com/acme/widgets/issues/9",
    ] {
        let mut github = InMemory::new();
        let result = publish(
            &mut github,
            &issue(8).repo(),
            &record(true, FixSize::Single),
            url,
            FixSize::Single,
            Source::ExistingFinding,
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "the private Security finding's issue does not match its record"
        );
        assert!(github.events.is_empty());
    }
}

#[test]
fn a_private_single_must_be_open_without_children_or_other_unready_labels() {
    for rejection in [
        "closed",
        "children",
        "READY-FOR-HUMAN",
        "Needs-Info",
        "WONTFIX",
    ] {
        let mut github = private_spec();
        github.get_mut(&issue(8)).children.clear();
        match rejection {
            "closed" => github.get_mut(&issue(8)).view.is_open = false,
            "children" => github.get_mut(&issue(8)).children.push(issue(9)),
            label => {
                github.get_mut(&issue(8)).view.labels =
                    ["Needs-Triage", label].into_iter().collect()
            }
        }
        let result = publish(
            &mut github,
            &issue(8).repo(),
            &record(true, FixSize::Single),
            &issue(8).url,
            FixSize::Single,
            Source::ExistingFinding,
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "the private Security finding must be an open, ready single Ticket"
        );
        assert!(!github.has(8, SECURITY_FIX));
        assert!(!github.events.iter().any(|event| event.starts_with("edit")));
    }
}

#[test]
fn private_spec_requires_its_own_parent_and_the_final_line() {
    for (message, expected) in [
        (
            None,
            "the Security fix publishing session ended without the final line its prompt asks for",
        ),
        (
            Some("Security fix Spec: https://github.com/acme/widgets/issues/8\nMore text"),
            "the Security fix publishing session ended without the final line its prompt asks for",
        ),
        (
            Some("Security fix Ticket: https://github.com/acme/widgets/issues/8"),
            "the Security fix publishing session ended without the final line its prompt asks for",
        ),
        (
            Some("Security fix Spec: https://github.com/other/widgets/issues/8"),
            "the Security fix Ticket is not in this repository",
        ),
        (
            Some("Security fix Spec: https://github.com/acme/widgets/issues/9"),
            "a private Security fix must reuse the finding's own issue",
        ),
    ] {
        let mut github = private_spec();
        assert_eq!(
            github
                .session(true, FixSize::Spec, message)
                .unwrap_err()
                .to_string(),
            expected
        );
        assert!(github.events.is_empty());
    }
}

const PRIVATE_SPEC: &str = "Security fix Spec: https://github.com/acme/widgets/issues/8";

#[test]
fn every_child_passes_all_checks_before_any_child_is_tagged() {
    for rejection in [
        "foreign owner",
        "foreign repository",
        "closed",
        "old",
        "not ready",
        "nested",
        "READY-FOR-HUMAN",
        "Needs-Info",
        "WONTFIX",
        "Needs-Triage",
        "missing link",
        "copied text",
    ] {
        let mut github = private_spec();
        let expected = match rejection {
            "foreign owner" | "foreign repository" => {
                let url = if rejection == "foreign owner" {
                    "https://github.com/other/widgets/issues/10"
                } else {
                    "https://github.com/acme/other/issues/10"
                };
                let foreign = IssueUrl::parse(url).unwrap();
                github.insert(foreign.clone(), &["ready-for-agent"], "Bound input.");
                github.get_mut(&issue(8)).children[1] = foreign;
                "a Security fix Spec's Ticket must be in the fix issue's repository"
            }
            "closed" | "old" => {
                if rejection == "closed" {
                    github.get_mut(&issue(10)).view.is_open = false;
                } else {
                    github.get_mut(&issue(10)).view.created =
                        started() - chrono::Duration::seconds(1);
                }
                "a Security fix Spec's Ticket must be new and open"
            }
            "missing link" => {
                github.get_mut(&issue(10)).text = "Bound input.".into();
                "the Security fix Ticket does not link the private record"
            }
            "copied text" => {
                github.get_mut(&issue(10)).text = format!("{SECRET} {}", issue(8).url);
                "the Security fix Ticket includes private write-up text"
            }
            label => {
                match label {
                    "not ready" => github.get_mut(&issue(10)).view.labels = Labels::default(),
                    "nested" => github.get_mut(&issue(10)).children.push(issue(9)),
                    label => {
                        github.get_mut(&issue(10)).view.labels =
                            ["ready-for-agent", label].into_iter().collect()
                    }
                }
                "a Security fix Spec's Ticket must be ready-for-agent with no sub-issues"
            }
        };
        assert_eq!(
            github
                .session(true, FixSize::Spec, Some(PRIVATE_SPEC))
                .unwrap_err()
                .to_string(),
            expected,
            "{rejection}"
        );
        for number in [8, 9, 10] {
            assert!(!github.has(number, SECURITY_FIX), "{rejection}: #{number}");
        }
        assert!(
            !github.events.iter().any(|event| event.starts_with("edit")),
            "{rejection}"
        );
        if rejection.starts_with("foreign") {
            assert!(!github.events.iter().any(|event| event == "issue 10"));
        }
    }
}

#[test]
fn failed_effects_stop_immediately_and_keep_the_completed_spec_effects() {
    // Each row describes the externally visible state immediately before the
    // failing request: created issues, native children, blockers and tagged children.
    for (event, created, children, blockers, tagged) in [
        ("sweep 1", 0, 0, 0, 0),
        ("create 8", 0, 0, 0, 0),
        ("create 9", 1, 0, 0, 0),
        ("create 10", 2, 0, 0, 0),
        ("link 8 9", 3, 0, 0, 0),
        ("link 8 10", 3, 1, 0, 0),
        ("block 10 9", 3, 2, 0, 0),
        ("sweep 2", 3, 2, 1, 0),
        ("issue 8", 3, 2, 1, 0),
        ("tickets 8", 3, 2, 1, 0),
        ("text 8", 3, 2, 1, 0),
        ("issue 9", 3, 2, 1, 0),
        ("text 9", 3, 2, 1, 0),
        ("issue 10", 3, 2, 1, 0),
        ("text 10", 3, 2, 1, 0),
        ("labels 9", 3, 2, 1, 0),
        ("edit 9", 3, 2, 1, 0),
        ("labels 10", 3, 2, 1, 1),
        ("edit 10", 3, 2, 1, 1),
        ("edit 8", 3, 2, 1, 2),
    ] {
        let mut github = InMemory::new();
        github.fail = Some(event.into());
        assert_eq!(
            github
                .session(false, FixSize::Spec, Some(&staged_spec()))
                .unwrap_err()
                .to_string(),
            format!("failed {event}")
        );
        assert_eq!(github.events.last().map(String::as_str), Some(event));
        assert_eq!(github.issues.len(), created, "{event}");
        assert!(
            github.issues.values().all(|stored| stored.view.is_open),
            "{event}: no rollback closes"
        );
        if created > 0 {
            assert!(github.has(8, NEEDS_TRIAGE), "{event}");
            assert!(!github.has(8, SECURITY_FIX), "{event}");
            assert!(!github.has(8, READY_FOR_AGENT), "{event}");
            assert_eq!(github.get(&issue(8)).children.len(), children, "{event}");
        }
        if created == 3 {
            assert_eq!(github.get(&issue(10)).blockers.len(), blockers, "{event}");
        }
        assert_eq!(
            github
                .issues
                .values()
                .filter(|stored| stored.view.labels.has(SECURITY_FIX))
                .count(),
            tagged,
            "{event}"
        );
    }
}

#[test]
fn interruption_keeps_child_tags_and_leaves_parent_readiness_untouched() {
    let mut github = InMemory::new();
    github.interrupted = true;
    assert_eq!(
        github
            .session(false, FixSize::Spec, Some(&staged_spec()))
            .unwrap_err()
            .to_string(),
        "interrupted"
    );
    assert!(github.has(9, SECURITY_FIX));
    assert!(github.has(10, SECURITY_FIX));
    assert!(github.has(8, NEEDS_TRIAGE));
    assert!(!github.has(8, READY_FOR_AGENT));
    assert!(!github.has(8, SECURITY_FIX));
    assert_eq!(
        github.events.last().map(String::as_str),
        Some("interrupted")
    );

    let mut github = private_spec();
    github.get_mut(&issue(8)).children.clear();
    github.interrupted = true;
    let result = publish(
        &mut github,
        &issue(8).repo(),
        &record(true, FixSize::Single),
        &issue(8).url,
        FixSize::Single,
        Source::ExistingFinding,
    );
    assert_eq!(result.unwrap_err().to_string(), "interrupted");
    assert!(!github.has(8, SECURITY_FIX));
    assert!(!github.events.iter().any(|event| event.starts_with("edit")));
}

#[test]
fn public_text_passing_window_issues_stay_open_in_both_sweeps() {
    for (size, postlinking) in [
        (FixSize::Single, false),
        (FixSize::Spec, false),
        (FixSize::Spec, true),
    ] {
        let mut github = InMemory::new();
        github.insert(issue(20), &[], &format!("Other work. {ADVISORY}"));
        if postlinking {
            github.windows.push_back(Vec::new());
        }
        github.windows.push_back(vec![issue(20)]);
        let message = match size {
            FixSize::Single => format!("{SINGLE}{ADVISORY}"),
            FixSize::Spec => staged_spec(),
        };
        assert_eq!(
            github.session(false, size, Some(&message)).unwrap(),
            issue(8)
        );
        assert!(github.get(&issue(20)).view.is_open);
        assert!(github.has(8, READY_FOR_AGENT));
    }
}

#[test]
fn rejected_window_issue_is_closed_before_a_later_text_read_fails() {
    let mut github = InMemory::new();
    github.insert(issue(20), &[], &format!("{SECRET} {ADVISORY}"));
    github.insert(issue(21), &[], &format!("Other work. {ADVISORY}"));
    github.windows.push_back(vec![issue(20), issue(21)]);
    github.fail = Some("text 21".into());

    let error = github
        .session(false, FixSize::Single, Some(&format!("{SINGLE}{ADVISORY}")))
        .unwrap_err();

    assert_eq!(error.to_string(), "failed text 21");
    assert!(
        !github.get(&issue(20)).view.is_open,
        "already rejected public leak #20 remained open after #21 could not be read"
    );
}

#[test]
fn side_effect_sweeps_close_in_window_order_and_stop_at_the_first_close_failure() {
    for postlinking in [false, true] {
        for failed_close in [None, Some(20)] {
            let mut github = InMemory::new();
            for number in [20, 21, 22] {
                github.insert(issue(number), &[], &format!("{SECRET} {ADVISORY}"));
            }
            github.insert(issue(23), &[], &format!("Separate change. {ADVISORY}"));
            let mut window = vec![issue(22), issue(23), issue(20), issue(21)];
            if postlinking {
                github.windows.push_back(Vec::new());
                window.extend([issue(8), issue(9), issue(10)]);
            }
            github.windows.push_back(window);
            github.fail = failed_close.map(|number| format!("close {number}"));
            let expected = if failed_close.is_some() {
                "failed close 20"
            } else {
                "the Security fix publishing session created public issue(s) outside the staged path: #22, #20, #21"
            };
            assert_eq!(
                github
                    .session(false, FixSize::Spec, Some(&staged_spec()))
                    .unwrap_err()
                    .to_string(),
                expected
            );
            let closes: Vec<&str> = github
                .events
                .iter()
                .map(String::as_str)
                .filter(|event| event.starts_with("close"))
                .collect();
            assert_eq!(
                closes,
                if failed_close.is_some() {
                    vec!["close 22", "close 20"]
                } else {
                    vec!["close 22", "close 20", "close 21"]
                }
            );
            assert!(!github.get(&issue(22)).view.is_open);
            assert_eq!(github.get(&issue(20)).view.is_open, failed_close.is_some());
            assert_eq!(github.get(&issue(21)).view.is_open, failed_close.is_some());
            assert!(github.get(&issue(23)).view.is_open);
            assert!(!github.events.iter().any(|event| event.starts_with("edit")));
            if postlinking {
                assert_eq!(github.get(&issue(8)).children, [issue(9), issue(10)]);
                assert_eq!(github.get(&issue(10)).blockers, [issue(9)]);
                for number in [8, 9, 10] {
                    assert!(github.get(&issue(number)).view.is_open);
                }
                assert!(github.has(8, NEEDS_TRIAGE));
            } else {
                assert!(
                    !github
                        .events
                        .iter()
                        .any(|event| event.starts_with("create"))
                );
            }
        }
    }
}

#[test]
fn public_single_sweeps_before_creation_and_has_no_postcreation_sweep() {
    let mut github = InMemory::new();
    github.windows = [Vec::new(), vec![issue(20)]].into();
    assert_eq!(
        github
            .session(false, FixSize::Single, Some(&format!("{SINGLE}{ADVISORY}")))
            .unwrap(),
        issue(8)
    );
    assert_eq!(github.sweeps, 1);
    assert_eq!(github.windows.len(), 1);
}

#[test]
fn child_label_edits_use_fresh_labels_and_keep_unrelated_labels() {
    let mut github = private_spec();
    github.label_reads.insert(
        issue(9).identity(),
        ["READY-FOR-AGENT", "bug", "added-later"]
            .into_iter()
            .collect(),
    );
    github
        .session(true, FixSize::Spec, Some(PRIVATE_SPEC))
        .unwrap();
    assert_eq!(
        github
            .get(&issue(9))
            .view
            .labels
            .names()
            .collect::<Vec<_>>(),
        ["READY-FOR-AGENT", "bug", "added-later", "security-fix"]
    );
}

fn public_view(labels: &[&str]) -> Issue {
    Issue {
        title: Some("Bound accepted input".into()),
        is_open: true,
        labels: labels.iter().copied().collect(),
        created: started(),
    }
}

#[test]
fn public_parent_observations_must_still_be_new_open_and_needs_triage() {
    for rejection in [
        "closed",
        "old",
        "missing triage",
        "READY-FOR-HUMAN",
        "Needs-Info",
        "WONTFIX",
    ] {
        let mut github = InMemory::new();
        let mut viewed = public_view(&["Needs-Triage"]);
        let expected = match rejection {
            "closed" => {
                viewed.is_open = false;
                "the Security fix Ticket is closed"
            }
            "old" => {
                viewed.created = started() - chrono::Duration::seconds(1);
                "the Security fix Ticket was created before this session"
            }
            label => {
                viewed.labels = if label == "missing triage" {
                    Labels::default()
                } else {
                    ["Needs-Triage", label].into_iter().collect()
                };
                "the Security fix Ticket must be labelled needs-triage with no other Unready Ticket label"
            }
        };
        github.reads.insert(issue(8).identity(), [viewed].into());
        assert_eq!(
            github
                .session(false, FixSize::Single, Some(&format!("{SINGLE}{ADVISORY}")))
                .unwrap_err()
                .to_string(),
            expected
        );
        assert!(!github.has(8, SECURITY_FIX));
        assert!(!github.events.iter().any(|event| event.starts_with("edit")));
    }
}

#[test]
fn freshness_comparisons_keep_second_resolution_for_parent_and_children() {
    let mut github = InMemory::new();
    for (number, labels) in [
        (8, vec!["Needs-Triage", "bug"]),
        (9, vec!["READY-FOR-AGENT"]),
        (10, vec!["ready-for-agent"]),
    ] {
        let mut view = public_view(&labels);
        view.created = started() - chrono::Duration::milliseconds(800);
        github.reads.insert(issue(number).identity(), [view].into());
    }
    assert_eq!(
        github
            .session(false, FixSize::Spec, Some(&staged_spec()))
            .unwrap(),
        issue(8)
    );
    for number in [8, 9, 10] {
        assert!(github.has(number, SECURITY_FIX));
    }
    assert!(
        github
            .get(&issue(8))
            .view
            .labels
            .names()
            .any(|name| name == "bug")
    );
}

#[test]
fn reread_public_issue_text_is_checked_after_creation_and_linking() {
    for number in [8, 9, 10] {
        let mut github = InMemory::new();
        github
            .text_reads
            .insert(issue(number).identity(), format!("{SECRET} {ADVISORY}"));
        assert_eq!(
            github
                .session(false, FixSize::Spec, Some(&staged_spec()))
                .unwrap_err()
                .to_string(),
            "the Security fix Ticket includes private write-up text"
        );
        assert_eq!(github.get(&issue(8)).children, [issue(9), issue(10)]);
        for number in [8, 9, 10] {
            assert!(!github.has(number, SECURITY_FIX));
            assert!(github.get(&issue(number)).view.is_open);
        }
    }
}

#[test]
fn private_spec_parent_must_be_open_ready_and_have_tickets() {
    for rejection in [
        "closed",
        "no children",
        "READY-FOR-HUMAN",
        "Needs-Info",
        "WONTFIX",
    ] {
        let mut github = private_spec();
        let expected = match rejection {
            "closed" => {
                github.get_mut(&issue(8)).view.is_open = false;
                "the Security fix Ticket is closed"
            }
            "no children" => {
                github.get_mut(&issue(8)).children.clear();
                "the Security fix's sub-issues do not match the reproduced fix size"
            }
            label => {
                github.get_mut(&issue(8)).view.labels =
                    ["Needs-Triage", label].into_iter().collect();
                "the Security fix Ticket must be labelled needs-triage with no other Unready Ticket label"
            }
        };
        assert_eq!(
            github
                .session(true, FixSize::Spec, Some(PRIVATE_SPEC))
                .unwrap_err()
                .to_string(),
            expected
        );
        assert!(!github.events.iter().any(|event| event.starts_with("edit")));
    }
}
