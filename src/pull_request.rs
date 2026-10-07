//! Pull request ownership for Spec Ticket accounting and Delivery. Numbered
//! observations, checklist edits, capture transitions, readiness and merge
//! reconciliation stay behind this interface.

use anyhow::{Context, Result, bail};

use crate::github::{GitHub, Mergeable, PrSnapshot, PrState};
use crate::harness::Choice;
use crate::issue::IssueUrl;
use crate::{poll, progress};

/// Immutable information for reporting and confirmed Self-merge finishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identified {
    pub number: u64,
    pub url: String,
}

/// A gate error is terminal (`Err`); only a refused request can return to
/// the Repair loop. A successful guarded command needs no additional read.
pub(crate) enum MergeAttempt {
    Merged,
    Refused(anyhow::Error),
}

/// Ticket accounting binds on resume or creation. Delivery binds lazily on
/// the first successful observation of the repository's own Issue branch. A wrong Base
/// branch still binds identity so Failed run salvage can draft that PR.
pub(crate) struct PullRequest<A = GitHub> {
    issue: IssueUrl,
    branch: String,
    base: String,
    identified: Option<Identified>,
    scope: CaptureScope,
    adapter: A,
}

#[derive(PartialEq, Eq)]
enum CaptureScope {
    TicketAccounting,
    Delivery,
}

impl<A: Adapter> PullRequest<A> {
    pub fn new(issue: &IssueUrl, branch: &str, base: &str, adapter: A) -> Self {
        Self {
            issue: issue.clone(),
            branch: branch.to_string(),
            base: base.to_string(),
            identified: None,
            scope: CaptureScope::Delivery,
            adapter,
        }
    }

    /// Resume accounting with an ordinary discovery, then draft the observed
    /// number only after a fresh numbered gate. Closed PRs allow a new landing.
    pub fn resume_spec(&mut self) -> Result<()> {
        self.scope = CaptureScope::TicketAccounting;
        let snapshot = self.adapter.observe(&self.issue, &self.branch, false)?;
        let Some(snapshot) = snapshot.filter(|snapshot| snapshot.pr.is_open()) else {
            return Ok(());
        };
        self.capture(&snapshot)?;
        self.validate(&snapshot, false)?;
        if !snapshot.pr.is_draft {
            let snapshot = self.accounting_snapshot(false)?;
            if !snapshot.pr.is_draft {
                self.adapter.draft(&self.issue, snapshot.pr.number, false)?;
            }
        }
        Ok(())
    }

    /// End Ticket accounting once, before Delivery's opening work. That
    /// session may replace the PR; the first later observation captures it.
    pub fn begin_delivery(&mut self) {
        if self.scope == CaptureScope::TicketAccounting {
            self.identified = None;
            self.scope = CaptureScope::Delivery;
        }
    }

    /// Progress text is optional. Invalid observations and failed writes warn
    /// without replacing identity or granting readiness/merge authority.
    pub fn show_checklist(&mut self, checklist: &str) {
        if self.identified.is_none() {
            return;
        }
        if let Err(error) = self.restore_checklist(checklist, true) {
            Self::warn_checklist(&error);
        }
    }

    /// A landing's identity gate is decisive; its body write is optional.
    /// Creation binds the returned URL before any numbered observation, so
    /// neither missing information nor a replacement can trigger discovery.
    pub fn land_spec(&mut self, checklist: &str) -> Result<String> {
        if self.identified.is_none() {
            progress::step(format!("opening the Spec PR into {} as a draft", self.base));
            let title = self.adapter.spec_title(&self.issue)?;
            let body = format!(
                "The work on Spec #{number}, gathered from its Tickets on {branch}.\n\n\
                 Closes #{number}\n\n{checklist}",
                number = self.issue.number,
                branch = self.branch,
                checklist = between_markers(checklist),
            );
            let url =
                self.adapter
                    .create_draft(&self.issue, &self.branch, &self.base, &title, &body)?;
            let url = url.trim().to_string();
            let number = self.number_from_url(&url)?;
            self.identified = Some(Identified { number, url });
            let snapshot = self.accounting_snapshot(true)?;
            if !snapshot.pr.is_draft {
                bail!("the Spec PR just opened is not a draft");
            }
        } else {
            let snapshot = self.accounting_snapshot(true)?;
            if let Err(error) = self.write_checklist(snapshot.pr.number, checklist, true) {
                Self::warn_checklist(&error);
            }
        }
        Ok(self
            .identified
            .as_ref()
            .expect("landing captured identity")
            .url
            .clone())
    }

    fn warn_checklist(error: &anyhow::Error) {
        progress::step(format!(
            "could not update the Spec PR's Tickets checklist: {error:#}"
        ));
    }

    fn accounting_snapshot(&mut self, completion: bool) -> Result<PrSnapshot> {
        let snapshot = self
            .observe(completion)?
            .context("the Spec PR is not found")?;
        self.validate(&snapshot, false)?;
        Ok(snapshot)
    }

    fn restore_checklist(&mut self, checklist: &str, completion: bool) -> Result<()> {
        let snapshot = self.accounting_snapshot(completion)?;
        self.write_checklist(snapshot.pr.number, checklist, completion)
    }

    fn write_checklist(&mut self, number: u64, checklist: &str, completion: bool) -> Result<()> {
        let body = self.adapter.body(&self.issue, number, completion)?;
        let updated = with_checklist(&body, checklist);
        if updated != body {
            progress::step("updating the Spec PR's Tickets checklist");
            self.adapter
                .set_body(&self.issue, number, &updated, completion)?;
        }
        Ok(())
    }

    /// Missing PRs are a no-op; the caller warns on errors, then readiness
    /// supplies the decisive gate. Body reads and edits always use the number.
    pub fn write_built_with(&mut self, choice: &Choice) -> Result<()> {
        let Some(snapshot) = self.observe(false)? else {
            return Ok(());
        };
        let body = self.adapter.body(&self.issue, snapshot.pr.number, false)?;
        let written = with_built_with(&body, choice);
        if written != body {
            progress::step(format!(
                "writing \"{}\" in the pull request's body",
                choice.built_with()
            ));
            self.adapter
                .set_body(&self.issue, snapshot.pr.number, &written, false)?;
        }
        Ok(())
    }

    /// Observe afresh and validate the expected branches and open state
    /// before marking this number ready. Already-ready PRs need no request.
    pub fn mark_ready(&mut self, checklist: Option<&str>) -> Result<Identified> {
        // Annotation normally captured identity already. If it was absent or
        // unreadable, discover before restoring any checklist.
        if self.identified.is_none() {
            self.observe(false)?.context("no PR found")?;
        }
        let identified = self.info().expect("observation captured identity");
        if let Some(checklist) = checklist {
            self.restore_checklist(checklist, false)?;
        }
        progress::step("checking the PR");
        let snapshot = self.observe(false)?.context("no PR found")?;
        self.validate(&snapshot, false)?;
        if snapshot.pr.is_draft {
            self.adapter.ready(&self.issue, snapshot.pr.number)?;
        }
        Ok(identified)
    }

    /// Immutable information for Delivery's final accounting, including
    /// identity captured only by Failed run completion discovery.
    pub fn info(&self) -> Option<Identified> {
        self.identified.clone()
    }

    /// Every unknown-mergeability poll repeats all readiness checks on one
    /// coherent numbered observation. Waiting remains interruptible.
    pub fn ensure_ready_and_mergeable(&mut self) -> Result<()> {
        progress::step("checking the PR is open, ready and mergeable");
        let deadline = std::time::Instant::now() + self.adapter.grace_period();
        loop {
            let snapshot = self.observe(false)?.context("no PR found")?;
            self.validate(&snapshot, true)?;
            match snapshot.mergeable {
                Mergeable::Yes => return Ok(()),
                Mergeable::No => bail!("PR {} is not mergeable", snapshot.pr.url),
                Mergeable::Unknown if std::time::Instant::now() >= deadline => {
                    bail!(
                        "GitHub has not worked out whether PR {} is mergeable",
                        snapshot.pr.url
                    );
                }
                Mergeable::Unknown => self.adapter.pause()?,
            }
        }
    }

    /// A fresh gate immediately precedes the numbered request. GitHub guards
    /// its head; a retarget between the gate and server merge remains a remote
    /// race because the request has no expected-base precondition.
    pub fn merge(&mut self, watched: &str) -> Result<MergeAttempt> {
        self.ensure_ready_and_mergeable()?;
        let number = self
            .identified
            .as_ref()
            .expect("gate captured identity")
            .number;
        let Err(error) = self.adapter.merge(&self.issue, number, watched) else {
            return Ok(MergeAttempt::Merged);
        };
        // Completion never clears or changes the recorded interruption. Only
        // this PR, merged at the watched head and expected branches, counts.
        if let Ok(Some(snapshot)) = self.observe(true)
            && snapshot.pr.state == PrState::Merged
            && snapshot.pr.base == self.base
            && snapshot.head_commit == watched
        {
            return Ok(MergeAttempt::Merged);
        }
        Ok(MergeAttempt::Refused(error))
    }

    /// After Worktree preservation, finish on the captured identity. Before
    /// any capture, make one completion discovery. A wrong target may be
    /// drafted for salvage; it never authorizes readiness or Self-merge.
    pub fn finish_failed_run(&mut self, keep_ready: bool) -> Result<Option<String>> {
        let Some(snapshot) = self.observe(true)? else {
            return Ok(None);
        };
        if !snapshot.pr.is_open() {
            return Ok(None);
        }
        if !snapshot.pr.is_draft && !keep_ready {
            self.adapter.draft(&self.issue, snapshot.pr.number, true)?;
        }
        Ok(Some(
            self.identified
                .as_ref()
                .expect("observation captured identity")
                .url
                .clone(),
        ))
    }

    fn observe(&mut self, completion: bool) -> Result<Option<PrSnapshot>> {
        let selector = self
            .identified
            .as_ref()
            .map(|pr| pr.number.to_string())
            .unwrap_or_else(|| self.branch.clone());
        let snapshot = self.adapter.observe(&self.issue, &selector, completion)?;
        if let Some(snapshot) = &snapshot {
            self.capture(snapshot)?;
        }
        Ok(snapshot)
    }

    fn capture(&mut self, snapshot: &PrSnapshot) -> Result<()> {
        if snapshot.from_fork {
            bail!("PR {} is from a fork", snapshot.pr.url);
        }
        if snapshot.pr.head != self.branch {
            bail!("PR head is {}, not {}", snapshot.pr.head, self.branch);
        }
        if let Some(pr) = &self.identified
            && snapshot.pr.number != pr.number
        {
            bail!(
                "PR identity changed from #{} to #{}",
                pr.number,
                snapshot.pr.number
            );
        }
        if self.number_from_url(&snapshot.pr.url)? != snapshot.pr.number {
            bail!(
                "PR URL does not match observed number #{}",
                snapshot.pr.number
            );
        }
        if self.identified.is_none() {
            self.identified = Some(Identified {
                number: snapshot.pr.number,
                url: snapshot.pr.url.clone(),
            });
        }
        Ok(())
    }

    fn number_from_url(&self, url: &str) -> Result<u64> {
        let parts = url
            .strip_prefix("https://github.com/")
            .map(|path| path.trim_end_matches('/').split('/').collect::<Vec<_>>());
        if let Some(parts) = parts
            && let [owner, repo, "pull", number] = parts.as_slice()
            && owner.eq_ignore_ascii_case(&self.issue.owner)
            && repo.eq_ignore_ascii_case(&self.issue.repo)
            && number.bytes().all(|byte| byte.is_ascii_digit())
            && let Ok(number) = number.parse::<u64>()
            && number > 0
        {
            return Ok(number);
        }
        bail!(
            "PR URL {url:?} does not identify a PR in {}",
            self.issue.repo_slug()
        )
    }

    fn validate(&self, snapshot: &PrSnapshot, ready: bool) -> Result<()> {
        let pr = &snapshot.pr;
        if !pr.is_open() {
            bail!("PR {} is {}, not open", pr.url, pr.state);
        }
        if pr.base != self.base {
            bail!("PR targets {}, not {}", pr.base, self.base);
        }
        if ready && pr.is_draft {
            bail!("PR {} is a draft", pr.url);
        }
        Ok(())
    }
}

/// The existing GitHub transport and a scripted test adapter vary here.
/// Selection and transition policy stay in PullRequest.
pub(crate) trait Adapter {
    fn observe(
        &mut self,
        issue: &IssueUrl,
        selector: &str,
        completion: bool,
    ) -> Result<Option<PrSnapshot>>;
    fn body(&mut self, issue: &IssueUrl, number: u64, completion: bool) -> Result<String>;
    fn set_body(
        &mut self,
        issue: &IssueUrl,
        number: u64,
        body: &str,
        completion: bool,
    ) -> Result<()>;
    fn ready(&mut self, issue: &IssueUrl, number: u64) -> Result<()>;
    fn draft(&mut self, issue: &IssueUrl, number: u64, completion: bool) -> Result<()>;
    fn spec_title(&mut self, issue: &IssueUrl) -> Result<String>;
    fn create_draft(
        &mut self,
        issue: &IssueUrl,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<String>;
    fn merge(&mut self, issue: &IssueUrl, number: u64, head: &str) -> Result<()>;
    fn pause(&mut self) -> Result<()> {
        poll::pause()
    }
    fn grace_period(&self) -> std::time::Duration {
        poll::grace_period()
    }
}

impl Adapter for GitHub {
    fn observe(
        &mut self,
        issue: &IssueUrl,
        selector: &str,
        completion: bool,
    ) -> Result<Option<PrSnapshot>> {
        if completion {
            self.completion().pr_snapshot(issue, selector)
        } else {
            self.pr_snapshot(issue, selector)
        }
    }
    fn body(&mut self, issue: &IssueUrl, number: u64, completion: bool) -> Result<String> {
        if completion {
            self.completion().pr_body(issue, number)
        } else {
            self.pr_body(issue, number)
        }
    }
    fn set_body(
        &mut self,
        issue: &IssueUrl,
        number: u64,
        body: &str,
        completion: bool,
    ) -> Result<()> {
        if completion {
            self.completion().set_pr_body(issue, number, body)
        } else {
            self.set_pr_body(issue, number, body)
        }
    }
    fn ready(&mut self, issue: &IssueUrl, number: u64) -> Result<()> {
        self.mark_ready(issue, &number.to_string())
    }
    fn draft(&mut self, issue: &IssueUrl, number: u64, completion: bool) -> Result<()> {
        if completion {
            self.completion()
                .convert_to_draft(issue, &number.to_string())
        } else {
            self.convert_to_draft(issue, &number.to_string())
        }
    }
    fn spec_title(&mut self, issue: &IssueUrl) -> Result<String> {
        self.completion().issue_title(issue)
    }
    fn create_draft(
        &mut self,
        issue: &IssueUrl,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<String> {
        self.completion()
            .create_draft_pr(issue, branch, base, title, body)
    }
    fn merge(&mut self, issue: &IssueUrl, number: u64, head: &str) -> Result<()> {
        GitHub::merge(self, issue, number, head)
    }
}

const CHECKLIST_START: &str = "<!-- thirdshift:tickets -->";
const CHECKLIST_END: &str = "<!-- /thirdshift:tickets -->";

/// `checklist` wrapped in the Tickets checklist markers.
fn between_markers(checklist: &str) -> String {
    format!("{CHECKLIST_START}\n{checklist}{CHECKLIST_END}\n")
}

/// `body` with its Tickets checklist, the text from its first start marker
/// to the next end marker, replaced by `checklist` between the markers, or
/// with that appended if it has no such pair of markers.
fn with_checklist(body: &str, checklist: &str) -> String {
    let checklist = between_markers(checklist);
    if let Some(start) = body.find(CHECKLIST_START)
        && let Some(end) = body[start..].find(CHECKLIST_END)
    {
        let end = start + end + CHECKLIST_END.len();
        let end = if body[end..].starts_with('\n') {
            end + 1
        } else {
            end
        };
        return format!("{}{checklist}{}", &body[..start], &body[end..]);
    }
    if body.is_empty() {
        return checklist;
    }
    format!("{}\n\n{checklist}", body.trim_end())
}

const BUILT_WITH_MARKER: &str = "<!-- thirdshift:built-with -->";

/// Preserve unrelated text and replace only thirdshift's annotation line.
fn with_built_with(body: &str, choice: &Choice) -> String {
    let line = format!("{} {BUILT_WITH_MARKER}", choice.built_with());
    if body.contains(BUILT_WITH_MARKER) {
        return body
            .split_inclusive('\n')
            .map(|old| {
                if old.trim_end().ends_with(BUILT_WITH_MARKER) {
                    let end = &old[old.trim_end_matches(['\r', '\n']).len()..];
                    format!("{line}{end}")
                } else {
                    old.to_string()
                }
            })
            .collect();
    }
    if body.trim().is_empty() {
        return format!("{line}\n");
    }
    format!("{}\n\n{line}\n", body.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::PullRequest as Record;
    use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

    const URL: &str = "https://github.com/acme/widgets/pull/12";
    type Answer = std::result::Result<Option<PrSnapshot>, &'static str>;

    fn snapshot() -> PrSnapshot {
        PrSnapshot {
            pr: Record {
                number: 12,
                url: URL.to_string(),
                state: PrState::Open,
                head: "issue-7".to_string(),
                base: "main".to_string(),
                is_draft: false,
            },
            mergeable: Mergeable::Yes,
            head_commit: "watched".to_string(),
            from_fork: false,
        }
    }

    /// Scripted GitHub observations and affected PRs. Outcomes are inspected
    /// at the external transport, never through the owner's private identity.
    struct World {
        observations: VecDeque<Answer>,
        completion: VecDeque<Answer>,
        selected: Vec<(String, bool)>,
        ready: Vec<u64>,
        draft: Vec<u64>,
        merges: Vec<(u64, String)>,
        edits: Vec<(u64, String)>,
        body: String,
        refused: bool,
        merge_signal: Option<i32>,
        body_error: bool,
        write_error: bool,
        draft_error: bool,
        title: std::result::Result<String, &'static str>,
        creation: std::result::Result<String, &'static str>,
        created: Vec<(String, String, String, String)>,
        effects: Vec<(String, bool)>,
        retarget_after_edit: bool,
        grace: Duration,
    }
    impl Default for World {
        fn default() -> Self {
            Self {
                observations: VecDeque::from([Ok(Some(snapshot()))]),
                completion: VecDeque::from([Ok(Some(snapshot()))]),
                selected: Vec::new(),
                ready: Vec::new(),
                draft: Vec::new(),
                merges: Vec::new(),
                edits: Vec::new(),
                body: "Closes #7\n".to_string(),
                refused: false,
                merge_signal: None,
                body_error: false,
                write_error: false,
                draft_error: false,
                title: Ok("Widgets".to_string()),
                creation: Ok(URL.to_string()),
                created: Vec::new(),
                effects: Vec::new(),
                retarget_after_edit: false,
                grace: Duration::ZERO,
            }
        }
    }

    #[derive(Clone)]
    struct Scripted(Rc<RefCell<World>>);
    impl Adapter for Scripted {
        fn observe(
            &mut self,
            issue: &IssueUrl,
            selector: &str,
            completion: bool,
        ) -> Result<Option<PrSnapshot>> {
            if !completion {
                crate::process::Interruption::Ordinary.check()?;
            }
            assert_eq!(issue.repo_slug(), "acme/widgets");
            let mut world = self.0.borrow_mut();
            world.selected.push((selector.to_string(), completion));
            let answers = if completion {
                &mut world.completion
            } else {
                &mut world.observations
            };
            let answer = if answers.len() > 1 {
                answers.pop_front().unwrap()
            } else {
                answers.front().expect("no snapshot scripted").clone()
            };
            answer.map_err(anyhow::Error::msg)
        }
        fn body(&mut self, _: &IssueUrl, number: u64, completion: bool) -> Result<String> {
            if !completion {
                crate::process::Interruption::Ordinary.check()?;
            }
            let mut world = self.0.borrow_mut();
            world.effects.push((format!("body {number}"), completion));
            if world.body_error {
                bail!("body unavailable");
            }
            Ok(world.body.clone())
        }
        fn set_body(
            &mut self,
            _: &IssueUrl,
            number: u64,
            body: &str,
            completion: bool,
        ) -> Result<()> {
            if !completion {
                crate::process::Interruption::Ordinary.check()?;
            }
            let mut world = self.0.borrow_mut();
            world.effects.push((format!("edit {number}"), completion));
            if world.write_error {
                bail!("write failed");
            }
            if world.retarget_after_edit {
                let mut retargeted = snapshot();
                retargeted.pr.base = "develop".to_string();
                world.observations = VecDeque::from([Ok(Some(retargeted))]);
            }
            world.edits.push((number, body.to_string()));
            world.body = body.to_string();
            Ok(())
        }
        fn ready(&mut self, _: &IssueUrl, number: u64) -> Result<()> {
            self.0.borrow_mut().ready.push(number);
            Ok(())
        }
        fn draft(&mut self, _: &IssueUrl, number: u64, completion: bool) -> Result<()> {
            if !completion {
                crate::process::Interruption::Ordinary.check()?;
            }
            let mut world = self.0.borrow_mut();
            world.effects.push((format!("draft {number}"), completion));
            if world.draft_error {
                bail!("draft failed");
            }
            world.draft.push(number);
            Ok(())
        }
        fn spec_title(&mut self, _: &IssueUrl) -> Result<String> {
            let mut world = self.0.borrow_mut();
            world.effects.push(("title".to_string(), true));
            world.title.clone().map_err(anyhow::Error::msg)
        }
        fn create_draft(
            &mut self,
            _: &IssueUrl,
            branch: &str,
            base: &str,
            title: &str,
            body: &str,
        ) -> Result<String> {
            let mut world = self.0.borrow_mut();
            world.effects.push(("create".to_string(), true));
            world.created.push((
                branch.to_string(),
                base.to_string(),
                title.to_string(),
                body.to_string(),
            ));
            world.creation.clone().map_err(anyhow::Error::msg)
        }
        fn merge(&mut self, _: &IssueUrl, number: u64, head: &str) -> Result<()> {
            let mut world = self.0.borrow_mut();
            world.merges.push((number, head.to_string()));
            if let Some(signal) = world.merge_signal {
                signal_hook::low_level::raise(signal).unwrap();
                bail!("interrupted");
            }
            if world.refused {
                bail!("merge request failed");
            }
            Ok(())
        }
        fn pause(&mut self) -> Result<()> {
            Ok(())
        }
        fn grace_period(&self) -> Duration {
            self.0.borrow().grace
        }
    }

    fn owner(world: World) -> (PullRequest<Scripted>, Rc<RefCell<World>>) {
        let world = Rc::new(RefCell::new(world));
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        (
            PullRequest::new(&issue, "issue-7", "main", Scripted(world.clone())),
            world,
        )
    }

    fn numbered(number: u64) -> PrSnapshot {
        let mut observed = snapshot();
        observed.pr.number = number;
        observed.pr.url = format!("https://github.com/acme/widgets/pull/{number}");
        observed
    }

    #[test]
    fn resume_drafts_the_observed_number_even_when_branch_discovery_would_find_a_replacement() {
        let (mut pr, world) = owner(World {
            observations: VecDeque::from([
                Ok(Some(snapshot())),
                Ok(Some(snapshot())),
                Ok(Some(numbered(13))),
            ]),
            ..World::default()
        });
        pr.resume_spec().unwrap();
        assert_eq!(world.borrow().draft, [12]);
        assert_eq!(
            world.borrow().selected,
            [("issue-7".to_string(), false), ("12".to_string(), false)]
        );
        assert_eq!(world.borrow().effects, [("draft 12".to_string(), false)]);
    }

    #[test]
    fn resume_revalidates_before_drafting_and_never_follows_a_replacement() {
        for change in [
            "base",
            "head",
            "fork",
            "repository",
            "number",
            "closed",
            "missing",
            "unreadable",
        ] {
            let mut changed = snapshot();
            match change {
                "base" => changed.pr.base = "develop".to_string(),
                "head" => changed.pr.head = "other".to_string(),
                "fork" => changed.from_fork = true,
                "repository" => {
                    changed.pr.url = "https://github.com/other/widgets/pull/12".to_string()
                }
                "number" => changed = numbered(13),
                "closed" => changed.pr.state = PrState::Closed,
                _ => (),
            }
            let next = match change {
                "missing" => Ok(None),
                "unreadable" => Err("read failed"),
                _ => Ok(Some(changed)),
            };
            let (mut pr, world) = owner(World {
                observations: VecDeque::from([Ok(Some(snapshot())), next]),
                ..World::default()
            });
            assert!(pr.resume_spec().is_err(), "{change}");
            assert!(world.borrow().draft.is_empty(), "{change}");
            assert_eq!(
                world.borrow().selected.last(),
                Some(&("12".to_string(), false))
            );
        }
    }

    #[test]
    fn resume_rejects_initial_wrong_branches_forks_and_repository_before_any_transition() {
        for change in ["base", "head", "fork", "repository"] {
            let mut changed = snapshot();
            match change {
                "base" => changed.pr.base = "develop".to_string(),
                "head" => changed.pr.head = "other".to_string(),
                "fork" => changed.from_fork = true,
                "repository" => {
                    changed.pr.url = "https://github.com/other/widgets/pull/12".to_string()
                }
                _ => unreachable!(),
            }
            let (mut pr, world) = owner(World {
                observations: VecDeque::from([Ok(Some(changed))]),
                ..World::default()
            });
            assert!(pr.resume_spec().is_err(), "{change}");
            assert!(world.borrow().draft.is_empty());
            assert!(world.borrow().edits.is_empty());
        }
    }

    #[test]
    fn absent_or_closed_resume_has_no_active_pr_and_first_landing_uses_the_creation_number() {
        let mut closed = snapshot();
        closed.pr.state = PrState::Closed;
        for resumed in [None, Some(closed)] {
            let mut created = numbered(13);
            created.pr.is_draft = true;
            let (mut pr, world) = owner(World {
                observations: VecDeque::from([Ok(resumed)]),
                completion: VecDeque::from([Ok(Some(created))]),
                creation: Ok("  https://github.com/ACME/Widgets/pull/13\n".to_string()),
                ..World::default()
            });
            pr.resume_spec().unwrap();
            pr.show_checklist("- [ ] #8 running\n");
            assert_eq!(pr.info(), None);
            assert_eq!(
                pr.land_spec("- [x] #8 landed\n").unwrap(),
                "https://github.com/ACME/Widgets/pull/13"
            );
            assert_eq!(
                world.borrow().selected,
                [("issue-7".to_string(), false), ("13".to_string(), true)]
            );
            assert_eq!(world.borrow().created, [(
                "issue-7".to_string(), "main".to_string(), "Widgets".to_string(),
                "The work on Spec #7, gathered from its Tickets on issue-7.\n\nCloses #7\n\n<!-- thirdshift:tickets -->\n- [x] #8 landed\n<!-- /thirdshift:tickets -->\n".to_string(),
            )]);
            assert!(world.borrow().draft.is_empty());
            assert!(world.borrow().ready.is_empty());
            assert!(world.borrow().merges.is_empty());
        }
    }

    #[test]
    fn unreadable_title_creation_or_invalid_creation_url_fails_without_discovery() {
        for problem in [
            "title",
            "creation",
            "",
            "garbage",
            "https://github.com/acme/other/pull/12",
            "https://github.com/acme/widgets/pull/0",
            "https://github.com/acme/widgets/pull/+12",
            "https://github.com/acme/widgets/issues/12",
            "https://github.com/acme/widgets/pull/12?x=1",
        ] {
            let (mut pr, world) = owner(World {
                observations: VecDeque::from([Ok(None)]),
                title: if problem == "title" {
                    Err("title unreadable")
                } else {
                    Ok("Widgets".to_string())
                },
                creation: if problem == "creation" {
                    Err("creation unreadable")
                } else {
                    Ok(problem.to_string())
                },
                ..World::default()
            });
            pr.resume_spec().unwrap();
            assert!(pr.land_spec("landed\n").is_err(), "{problem}");
            assert_eq!(world.borrow().selected, [("issue-7".to_string(), false)]);
            assert!(world.borrow().edits.is_empty());
        }
    }

    #[test]
    fn creation_requires_an_open_draft_with_the_returned_number_and_expected_ownership() {
        for change in [
            "missing",
            "unreadable",
            "number",
            "base",
            "head",
            "fork",
            "repository",
            "closed",
            "ready",
        ] {
            let mut created = snapshot();
            created.pr.is_draft = true;
            match change {
                "number" => created = numbered(13),
                "base" => created.pr.base = "develop".to_string(),
                "head" => created.pr.head = "other".to_string(),
                "fork" => created.from_fork = true,
                "repository" => {
                    created.pr.url = "https://github.com/other/widgets/pull/12".to_string()
                }
                "closed" => created.pr.state = PrState::Closed,
                "ready" => created.pr.is_draft = false,
                _ => (),
            }
            let observation = match change {
                "missing" => Ok(None),
                "unreadable" => Err("read failed"),
                _ => Ok(Some(created)),
            };
            let (mut pr, world) = owner(World {
                observations: VecDeque::from([Ok(None)]),
                completion: VecDeque::from([observation]),
                ..World::default()
            });
            pr.resume_spec().unwrap();
            assert!(pr.land_spec("landed\n").is_err(), "{change}");
            assert_eq!(
                world.borrow().selected.last(),
                Some(&("12".to_string(), true))
            );
            assert!(world.borrow().edits.is_empty());
        }
    }

    #[test]
    fn accounting_never_edits_or_lands_an_invalid_numbered_observation() {
        for change in [
            "base",
            "head",
            "fork",
            "repository",
            "number",
            "closed",
            "merged",
            "missing",
            "unreadable",
        ] {
            let mut changed = snapshot();
            match change {
                "base" => changed.pr.base = "develop".to_string(),
                "head" => changed.pr.head = "other".to_string(),
                "fork" => changed.from_fork = true,
                "repository" => {
                    changed.pr.url = "https://github.com/other/widgets/pull/12".to_string()
                }
                "number" => changed = numbered(13),
                "closed" => changed.pr.state = PrState::Closed,
                "merged" => changed.pr.state = PrState::Merged,
                _ => (),
            }
            let observation = match change {
                "missing" => Ok(None),
                "unreadable" => Err("read failed"),
                _ => Ok(Some(changed)),
            };
            let (mut pr, world) = owner(World {
                completion: VecDeque::from([observation]),
                ..World::default()
            });
            pr.resume_spec().unwrap();
            pr.show_checklist("new\n");
            assert!(pr.land_spec("new\n").is_err(), "{change}");
            assert_eq!(world.borrow().body, "Closes #7\n");
            assert!(world.borrow().edits.is_empty());
            assert!(world.borrow().created.is_empty());
            assert!(
                world.borrow().selected[1..]
                    .iter()
                    .all(|(selector, _)| selector == "12")
            );
        }
    }

    #[test]
    fn checklist_preserves_surrounding_text_is_idempotent_and_never_marks_ready_or_merges() {
        let (mut pr, world) = owner(World {
            body:
                "Summary\n\n<!-- thirdshift:tickets -->\nold\n<!-- /thirdshift:tickets -->\nTail\n"
                    .to_string(),
            ..World::default()
        });
        pr.resume_spec().unwrap();
        pr.show_checklist("- [x] #8 done\n");
        pr.show_checklist("- [x] #8 done\n");
        assert_eq!(pr.land_spec("- [x] #8 done\n").unwrap(), URL);
        assert_eq!(
            world.borrow().body,
            "Summary\n\n<!-- thirdshift:tickets -->\n- [x] #8 done\n<!-- /thirdshift:tickets -->\nTail\n"
        );
        assert_eq!(world.borrow().edits.len(), 1);
        assert_eq!(world.borrow().edits[0].0, 12);
        assert!(world.borrow().ready.is_empty());
        assert!(world.borrow().merges.is_empty());
    }

    #[test]
    fn checklist_read_and_write_failures_warn_during_accounting_but_fail_before_readiness() {
        for read in [false, true] {
            let (mut pr, world) = owner(World {
                body_error: read,
                write_error: !read,
                ..World::default()
            });
            pr.resume_spec().unwrap();
            pr.show_checklist("new\n");
            assert_eq!(pr.land_spec("new\n").unwrap(), URL);
            pr.begin_delivery();
            assert!(pr.mark_ready(Some("new\n")).is_err());
            assert!(world.borrow().ready.is_empty());
            assert_eq!(world.borrow().body, "Closes #7\n");
        }
    }

    #[test]
    fn delivery_capture_replaces_accounting_once_and_final_checklists_use_that_same_owner() {
        for opening_failed in [false, true] {
            let (mut pr, world) = owner(World::default());
            pr.resume_spec().unwrap();
            pr.begin_delivery();
            assert_eq!(pr.info(), None);
            let mut replacement = numbered(13);
            replacement.pr.is_draft = true;
            world.borrow_mut().observations = VecDeque::from([Ok(Some(replacement.clone()))]);
            world.borrow_mut().completion = VecDeque::from([Ok(Some(replacement))]);
            if opening_failed {
                assert_eq!(
                    pr.finish_failed_run(false).unwrap().as_deref(),
                    Some("https://github.com/acme/widgets/pull/13")
                );
            } else {
                assert_eq!(pr.mark_ready(Some("done\n")).unwrap().number, 13);
            }
            pr.begin_delivery(); // A later call cannot reset a Delivery capture.
            pr.show_checklist("final\n");
            assert_eq!(world.borrow().edits.last().unwrap().0, 13);
            assert_eq!(
                world.borrow().selected.last(),
                Some(&("13".to_string(), true))
            );
            assert_eq!(pr.info().unwrap().number, 13);
        }
    }

    #[test]
    fn checklist_appends_to_unmarked_empty_or_incomplete_bodies_without_removing_text() {
        for (body, expected) in [
            (
                "",
                "<!-- thirdshift:tickets -->\nnew\n<!-- /thirdshift:tickets -->\n",
            ),
            (
                "Closes #7\n",
                "Closes #7\n\n<!-- thirdshift:tickets -->\nnew\n<!-- /thirdshift:tickets -->\n",
            ),
            (
                "Intro\n<!-- thirdshift:tickets -->\nold\n",
                "Intro\n<!-- thirdshift:tickets -->\nold\n\n<!-- thirdshift:tickets -->\nnew\n<!-- /thirdshift:tickets -->\n",
            ),
        ] {
            let (mut pr, world) = owner(World {
                body: body.to_string(),
                ..World::default()
            });
            pr.resume_spec().unwrap();
            pr.show_checklist("new\n");
            assert_eq!(world.borrow().body, expected);
        }
    }

    #[test]
    fn accounting_finishes_after_interruption_without_enabling_ordinary_delivery() {
        crate::test_support::with_recorded_signal(
            "pull_request::tests::accounting_finishes_after_interruption_without_enabling_ordinary_delivery",
            |signal| {
                let mut created = snapshot();
                created.pr.is_draft = true;
                let (mut first_landing, first_world) = owner(World {
                    observations: VecDeque::from([Ok(None)]),
                    completion: VecDeque::from([Ok(Some(created))]),
                    ..World::default()
                });
                let (mut existing, existing_world) = owner(World::default());
                first_landing.resume_spec().unwrap();
                existing.resume_spec().unwrap();
                existing_world.borrow_mut().effects.clear();
                signal_hook::low_level::raise(signal).unwrap();
                for (pr, world) in [
                    (&mut first_landing, first_world),
                    (&mut existing, existing_world),
                ] {
                    assert_eq!(pr.land_spec("done\n").unwrap(), URL);
                    pr.show_checklist("final\n");
                    assert!(
                        world
                            .borrow()
                            .effects
                            .iter()
                            .all(|(_, completion)| *completion)
                    );
                    assert!(world.borrow().body.contains("final\n"));
                    pr.begin_delivery();
                    assert_eq!(
                        pr.mark_ready(Some("final\n")).unwrap_err().to_string(),
                        "interrupted"
                    );
                    assert!(crate::interrupt::requested());
                    assert!(world.borrow().ready.is_empty());
                }
                let (mut cancelled_resume, _) = owner(World::default());
                assert_eq!(
                    cancelled_resume.resume_spec().unwrap_err().to_string(),
                    "interrupted"
                );
            },
        );
    }

    #[test]
    fn draft_failure_on_resume_is_terminal() {
        let (mut pr, world) = owner(World {
            draft_error: true,
            ..World::default()
        });
        assert_eq!(pr.resume_spec().unwrap_err().to_string(), "draft failed");
        assert!(world.borrow().draft.is_empty());
    }

    #[test]
    fn annotation_preserves_text_is_idempotent_and_replaces_the_previous_choice() {
        let (mut pr, world) = owner(World::default());
        let choice = Choice {
            model: Some("opus".to_string()),
            effort: Some("high".to_string()),
            ..Choice::default()
        };
        pr.write_built_with(&Choice::default()).unwrap();
        pr.write_built_with(&Choice::default()).unwrap();
        pr.write_built_with(&choice).unwrap();
        let expected =
            "Closes #7\n\nBuilt with claude · opus · high <!-- thirdshift:built-with -->\n";
        assert_eq!(world.borrow().body, expected);
        assert_eq!(world.borrow().edits.len(), 2);
        assert!(world.borrow().edits.iter().all(|(number, _)| *number == 12));
    }

    #[test]
    fn annotation_replaces_its_line_in_place_preserving_unrelated_text_and_line_endings() {
        let body = "Summary\r\nBuilt with old <!-- thirdshift:built-with -->\r\n\r\nCloses #7\r\n";
        let (mut pr, world) = owner(World {
            body: body.to_string(),
            ..World::default()
        });
        pr.write_built_with(&Choice::default()).unwrap();
        assert_eq!(
            world.borrow().body,
            "Summary\r\nBuilt with claude · default model · default effort <!-- thirdshift:built-with -->\r\n\r\nCloses #7\r\n"
        );
    }

    #[test]
    fn missing_or_failed_initial_observations_allow_discovery_at_readiness() {
        for first in [Ok(None), Err("transport failed")] {
            let (mut pr, world) = owner(World {
                observations: VecDeque::from([first, Ok(Some(snapshot()))]),
                ..World::default()
            });
            let _ = pr.write_built_with(&Choice::default());
            assert_eq!(pr.mark_ready(None).unwrap().url, URL);
            assert_eq!(
                world.borrow().selected[..2],
                [
                    ("issue-7".to_string(), false),
                    ("issue-7".to_string(), false)
                ]
            );
            assert!(
                world.borrow().selected[2..]
                    .iter()
                    .all(|(selector, _)| selector == "12")
            );
        }
    }

    #[test]
    fn a_body_read_failure_still_binds_identity_for_readiness_and_failure_finishing() {
        let (mut pr, world) = owner(World {
            body_error: true,
            ..World::default()
        });
        assert!(pr.write_built_with(&Choice::default()).is_err());
        pr.mark_ready(None).unwrap();
        assert_eq!(pr.finish_failed_run(false).unwrap().as_deref(), Some(URL));
        assert_eq!(world.borrow().draft, [12]);
        assert_eq!(
            world.borrow().selected[1..],
            [("12".to_string(), false), ("12".to_string(), true)]
        );
    }

    #[test]
    fn initial_wrong_target_binds_identity_but_never_marks_ready_and_can_be_drafted() {
        let mut wrong = snapshot();
        wrong.pr.base = "develop".to_string();
        let (mut pr, world) = owner(World {
            observations: VecDeque::from([Ok(Some(wrong.clone()))]),
            completion: VecDeque::from([Ok(Some(wrong))]),
            ..World::default()
        });
        assert_eq!(
            pr.mark_ready(None).unwrap_err().to_string(),
            "PR targets develop, not main"
        );
        assert_eq!(pr.finish_failed_run(false).unwrap().as_deref(), Some(URL));
        assert!(world.borrow().ready.is_empty());
        assert_eq!(world.borrow().draft, [12]);
        assert_eq!(
            world.borrow().selected.last(),
            Some(&("12".to_string(), true))
        );
    }

    #[test]
    fn discovery_rejects_fork_and_unexpected_issue_branch() {
        for fork in [false, true] {
            let mut wrong = snapshot();
            if fork {
                wrong.from_fork = true;
            } else {
                wrong.pr.head = "other".to_string();
            }
            let (mut pr, world) = owner(World {
                observations: VecDeque::from([Ok(Some(wrong))]),
                ..World::default()
            });
            assert!(pr.mark_ready(None).is_err());
            assert!(world.borrow().ready.is_empty());
        }
    }

    #[test]
    fn readiness_marks_only_a_draft_and_returns_immutable_identity() {
        for draft in [false, true] {
            let mut observed = snapshot();
            observed.pr.is_draft = draft;
            let (mut pr, world) = owner(World {
                observations: VecDeque::from([Ok(Some(observed))]),
                ..World::default()
            });
            assert_eq!(
                pr.mark_ready(None).unwrap(),
                Identified {
                    number: 12,
                    url: URL.to_string()
                }
            );
            assert_eq!(
                world.borrow().ready,
                if draft { vec![12] } else { Vec::new() }
            );
        }
    }

    #[test]
    fn checklist_restoration_rechecks_readiness_after_the_body_edit() {
        let (mut pr, world) = owner(World {
            retarget_after_edit: true,
            ..World::default()
        });
        pr.write_built_with(&Choice::default()).unwrap();
        // Retarget on the checklist write, rather than the annotation write.
        world.borrow_mut().observations = VecDeque::from([Ok(Some(snapshot()))]);
        let error = pr.mark_ready(Some("- [x] #8 done\n")).unwrap_err();
        assert_eq!(error.to_string(), "PR targets develop, not main");
        assert!(world.borrow().ready.is_empty());
    }

    #[test]
    fn every_unknown_mergeability_snapshot_revalidates_target_state_and_ownership() {
        // First UNKNOWN is followed immediately by the changed snapshot;
        // no pause is needed to script a change during polling.
        for change in ["base", "closed", "draft", "head", "fork", "number"] {
            let mut unknown = snapshot();
            unknown.mergeable = Mergeable::Unknown;
            let mut changed = unknown.clone();
            match change {
                "base" => changed.pr.base = "develop".to_string(),
                "closed" => changed.pr.state = PrState::Closed,
                "draft" => changed.pr.is_draft = true,
                "head" => changed.pr.head = "other".to_string(),
                "fork" => changed.from_fork = true,
                "number" => changed.pr.number = 13,
                _ => unreachable!(),
            }
            let (mut pr, world) = owner(World {
                observations: VecDeque::from([
                    Ok(Some(snapshot())),
                    Ok(Some(snapshot())),
                    Ok(Some(unknown)),
                    Ok(Some(changed)),
                ]),
                grace: Duration::from_secs(60),
                ..World::default()
            });
            pr.mark_ready(None).unwrap();
            assert!(pr.ensure_ready_and_mergeable().is_err(), "{change}");
            assert!(
                world.borrow().selected[1..]
                    .iter()
                    .all(|(selector, _)| selector == "12")
            );
        }
    }

    #[test]
    fn missing_closed_draft_conflicting_and_unknown_prs_fail_final_readiness() {
        let mut closed = snapshot();
        closed.pr.state = PrState::Closed;
        let mut draft = snapshot();
        draft.pr.is_draft = true;
        let mut conflicting = snapshot();
        conflicting.mergeable = Mergeable::No;
        let mut unknown = snapshot();
        unknown.mergeable = Mergeable::Unknown;
        for (answer, message) in [
            (Ok(None), "no PR found"),
            (Ok(Some(closed)), "is closed, not open"),
            (Ok(Some(draft)), "is a draft"),
            (Ok(Some(conflicting)), "is not mergeable"),
            (Ok(Some(unknown)), "has not worked out"),
        ] {
            let (mut pr, _) = owner(World {
                observations: VecDeque::from([Ok(Some(snapshot())), Ok(Some(snapshot())), answer]),
                ..World::default()
            });
            pr.mark_ready(None).unwrap();
            assert!(
                pr.ensure_ready_and_mergeable()
                    .unwrap_err()
                    .to_string()
                    .contains(message)
            );
        }
    }

    #[test]
    fn disappearance_or_transport_failure_after_capture_never_rediscovers_by_branch() {
        for missing in [Ok(None), Err("gone")] {
            let (mut pr, world) = owner(World {
                observations: VecDeque::from([
                    Ok(Some(snapshot())),
                    Ok(Some(snapshot())),
                    missing.clone(),
                ]),
                completion: VecDeque::from([missing]),
                ..World::default()
            });
            pr.mark_ready(None).unwrap();
            assert!(pr.ensure_ready_and_mergeable().is_err());
            let _ = pr.finish_failed_run(false);
            assert_eq!(
                world.borrow().selected[2..],
                [("12".to_string(), false), ("12".to_string(), true)]
            );
            assert!(world.borrow().draft.is_empty());
        }
    }

    #[test]
    fn closed_original_or_unexpected_replacement_cannot_be_reported_or_modified() {
        for replaced in [false, true] {
            let mut original = snapshot();
            if replaced {
                original.pr.number = 13;
            } else {
                original.pr.state = PrState::Closed;
            }
            let (mut pr, world) = owner(World {
                completion: VecDeque::from([Ok(Some(original))]),
                ..World::default()
            });
            pr.mark_ready(None).unwrap();
            assert!(pr.finish_failed_run(false).ok().flatten().is_none());
            assert!(world.borrow().draft.is_empty());
        }
    }

    #[test]
    fn a_fresh_invalid_or_unreadable_merge_gate_issues_no_request() {
        let mut wrong = snapshot();
        wrong.pr.base = "develop".to_string();
        for gate in [Ok(Some(wrong)), Err("unreadable")] {
            let (mut pr, world) = owner(World {
                observations: VecDeque::from([Ok(Some(snapshot())), Ok(Some(snapshot())), gate]),
                ..World::default()
            });
            pr.mark_ready(None).unwrap();
            assert!(pr.merge("watched").is_err());
            assert!(world.borrow().merges.is_empty());
        }
    }

    #[test]
    fn successful_guarded_merge_requires_no_completion_observation() {
        let (mut pr, world) = owner(World {
            completion: VecDeque::from([Err("completion unavailable")]),
            ..World::default()
        });
        pr.mark_ready(None).unwrap();
        assert!(matches!(pr.merge("watched").unwrap(), MergeAttempt::Merged));
        assert_eq!(world.borrow().merges, [(12, "watched".to_string())]);
        assert!(
            world
                .borrow()
                .selected
                .iter()
                .all(|(_, completion)| !completion)
        );
    }

    #[test]
    fn failed_requests_are_confirmed_only_for_the_same_pr_head_and_expected_branches() {
        for change in [
            "none",
            "number",
            "head",
            "base",
            "branch",
            "fork",
            "open",
            "closed",
            "missing",
            "unreadable",
        ] {
            let mut merged = snapshot();
            merged.pr.state = PrState::Merged;
            match change {
                "number" => merged.pr.number = 13,
                "head" => merged.head_commit = "foreign".to_string(),
                "base" => merged.pr.base = "develop".to_string(),
                "branch" => merged.pr.head = "other".to_string(),
                "fork" => merged.from_fork = true,
                "open" => merged.pr.state = PrState::Open,
                "closed" => merged.pr.state = PrState::Closed,
                _ => (),
            }
            let completion = match change {
                "missing" => Ok(None),
                "unreadable" => Err("unreadable"),
                _ => Ok(Some(merged)),
            };
            let (mut pr, world) = owner(World {
                refused: true,
                completion: VecDeque::from([completion]),
                ..World::default()
            });
            pr.mark_ready(None).unwrap();
            let attempt = pr.merge("watched").unwrap();
            assert_eq!(
                matches!(attempt, MergeAttempt::Merged),
                change == "none",
                "{change}"
            );
            if let MergeAttempt::Refused(error) = attempt {
                assert_eq!(error.to_string(), "merge request failed");
            }
            assert_eq!(
                world.borrow().selected.last(),
                Some(&("12".to_string(), true))
            );
        }
    }

    #[test]
    fn failed_run_can_discover_before_capture_and_policy_refusal_keeps_ready() {
        for keep_ready in [false, true] {
            let (mut pr, world) = owner(World::default());
            assert_eq!(
                pr.finish_failed_run(keep_ready).unwrap().as_deref(),
                Some(URL)
            );
            assert_eq!(world.borrow().selected, [("issue-7".to_string(), true)]);
            assert_eq!(
                world.borrow().draft,
                if keep_ready { Vec::new() } else { vec![12] }
            );
        }
        let mut draft = snapshot();
        draft.pr.is_draft = true;
        let (mut pr, world) = owner(World {
            completion: VecDeque::from([Ok(Some(draft))]),
            ..World::default()
        });
        assert_eq!(pr.finish_failed_run(false).unwrap().as_deref(), Some(URL));
        assert!(world.borrow().draft.is_empty());
    }
    fn interrupted_merge(name: &str, confirmed: bool) {
        crate::test_support::with_recorded_signal(name, |signal| {
            let mut completion = snapshot();
            if confirmed {
                completion.pr.state = PrState::Merged;
            }
            let (mut pr, world) = owner(World {
                merge_signal: Some(signal),
                completion: VecDeque::from([Ok(Some(completion))]),
                ..World::default()
            });
            pr.mark_ready(None).unwrap();
            let attempt = pr.merge("watched").unwrap();
            assert_eq!(matches!(attempt, MergeAttempt::Merged), confirmed);
            if let MergeAttempt::Refused(error) = attempt {
                assert_eq!(error.to_string(), "interrupted");
            }
            assert!(crate::interrupt::requested());
            assert_eq!(
                pr.ensure_ready_and_mergeable().unwrap_err().to_string(),
                "interrupted"
            );
            assert!(world.borrow().selected.contains(&("12".to_string(), true)));
        });
    }

    #[test]
    fn an_interrupted_request_can_be_confirmed_without_clearing_interruption() {
        interrupted_merge(
            "pull_request::tests::an_interrupted_request_can_be_confirmed_without_clearing_interruption",
            true,
        );
    }

    #[test]
    fn an_unconfirmed_interrupted_request_keeps_its_failure_and_interruption() {
        interrupted_merge(
            "pull_request::tests::an_unconfirmed_interrupted_request_keeps_its_failure_and_interruption",
            false,
        );
    }
}
