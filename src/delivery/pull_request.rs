//! Delivery's captured pull request and its transitions. Callers request
//! annotation, readiness, Self-merge or Failed run finishing; discovery,
//! numbered observations, target validation and reconciliation stay here.

use anyhow::{Context, Result, bail};

use crate::github::{GitHub, Mergeable, PrSnapshot, PrState};
use crate::harness::Choice;
use crate::issue::IssueUrl;
use crate::{poll, progress};

/// Immutable information for reporting and confirmed Self-merge finishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Identified {
    pub number: u64,
    pub url: String,
}

/// A gate error is terminal (`Err`); only a refused request can return to
/// the Repair loop. A successful guarded command needs no additional read.
pub(super) enum MergeAttempt {
    Merged,
    Refused(anyhow::Error),
}

/// Created before the opening session; binds lazily on the first successful
/// observation of the expected repository's own Issue branch. A wrong Base
/// branch still binds identity so Failed run salvage can draft that PR.
pub(super) struct PullRequest<A = GitHub> {
    issue: IssueUrl,
    branch: String,
    base: String,
    identified: Option<Identified>,
    adapter: A,
}

impl<A: Adapter> PullRequest<A> {
    pub fn new(issue: &IssueUrl, branch: &str, base: &str, adapter: A) -> Self {
        Self {
            issue: issue.clone(),
            branch: branch.to_string(),
            base: base.to_string(),
            identified: None,
            adapter,
        }
    }

    /// Missing PRs are a no-op; the caller warns on errors, then readiness
    /// supplies the decisive gate. Body reads and edits always use the number.
    pub fn write_built_with(&mut self, choice: &Choice) -> Result<()> {
        let Some(snapshot) = self.observe(false)? else {
            return Ok(());
        };
        let body = self.adapter.body(&self.issue, snapshot.pr.number)?;
        let written = with_built_with(&body, choice);
        if written != body {
            progress::step(format!(
                "writing \"{}\" in the pull request's body",
                choice.built_with()
            ));
            self.adapter
                .set_body(&self.issue, snapshot.pr.number, &written)?;
        }
        Ok(())
    }

    /// Observe afresh and validate the expected branches and open state
    /// before marking this number ready. Already-ready PRs need no request.
    pub fn mark_ready(&mut self) -> Result<Identified> {
        progress::step("checking the PR");
        let snapshot = self.observe(false)?.context("no PR found")?;
        self.validate(&snapshot, false)?;
        if snapshot.pr.is_draft {
            self.adapter.ready(&self.issue, snapshot.pr.number)?;
        }
        Ok(self
            .identified
            .as_ref()
            .expect("observation captured identity")
            .clone())
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
            self.adapter.draft(&self.issue, snapshot.pr.number)?;
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
            if snapshot.from_fork {
                bail!("PR {} is from a fork", snapshot.pr.url);
            }
            if snapshot.pr.head != self.branch {
                bail!("PR head is {}, not {}", snapshot.pr.head, self.branch);
            }
            if let Some(pr) = &self.identified {
                if snapshot.pr.number != pr.number {
                    bail!(
                        "PR identity changed from #{} to #{}",
                        pr.number,
                        snapshot.pr.number
                    );
                }
            } else {
                self.identified = Some(Identified {
                    number: snapshot.pr.number,
                    url: snapshot.pr.url.clone(),
                });
            }
        }
        Ok(snapshot)
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
pub(super) trait Adapter {
    fn observe(
        &mut self,
        issue: &IssueUrl,
        selector: &str,
        completion: bool,
    ) -> Result<Option<PrSnapshot>>;
    fn body(&mut self, issue: &IssueUrl, number: u64) -> Result<String>;
    fn set_body(&mut self, issue: &IssueUrl, number: u64, body: &str) -> Result<()>;
    fn ready(&mut self, issue: &IssueUrl, number: u64) -> Result<()>;
    fn draft(&mut self, issue: &IssueUrl, number: u64) -> Result<()>;
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
    fn body(&mut self, issue: &IssueUrl, number: u64) -> Result<String> {
        self.pr_body(issue, number)
    }
    fn set_body(&mut self, issue: &IssueUrl, number: u64, body: &str) -> Result<()> {
        self.set_pr_body(issue, number, body)
    }
    fn ready(&mut self, issue: &IssueUrl, number: u64) -> Result<()> {
        self.mark_ready(issue, &number.to_string())
    }
    fn draft(&mut self, issue: &IssueUrl, number: u64) -> Result<()> {
        self.completion()
            .convert_to_draft(issue, &number.to_string())
    }
    fn merge(&mut self, issue: &IssueUrl, number: u64, head: &str) -> Result<()> {
        GitHub::merge(self, issue, number, head)
    }
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
        fn body(&mut self, _: &IssueUrl, number: u64) -> Result<String> {
            assert_eq!(number, 12);
            let world = self.0.borrow();
            if world.body_error {
                bail!("body unavailable");
            }
            Ok(world.body.clone())
        }
        fn set_body(&mut self, _: &IssueUrl, number: u64, body: &str) -> Result<()> {
            let mut world = self.0.borrow_mut();
            world.edits.push((number, body.to_string()));
            world.body = body.to_string();
            Ok(())
        }
        fn ready(&mut self, _: &IssueUrl, number: u64) -> Result<()> {
            self.0.borrow_mut().ready.push(number);
            Ok(())
        }
        fn draft(&mut self, _: &IssueUrl, number: u64) -> Result<()> {
            self.0.borrow_mut().draft.push(number);
            Ok(())
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
            assert_eq!(pr.mark_ready().unwrap().url, URL);
            assert_eq!(
                world.borrow().selected,
                [
                    ("issue-7".to_string(), false),
                    ("issue-7".to_string(), false)
                ]
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
        pr.mark_ready().unwrap();
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
            pr.mark_ready().unwrap_err().to_string(),
            "PR targets develop, not main"
        );
        assert_eq!(pr.finish_failed_run(false).unwrap().as_deref(), Some(URL));
        assert!(world.borrow().ready.is_empty());
        assert_eq!(world.borrow().draft, [12]);
        assert_eq!(world.borrow().selected[1], ("12".to_string(), true));
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
            assert!(pr.mark_ready().is_err());
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
                pr.mark_ready().unwrap(),
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
                    Ok(Some(unknown)),
                    Ok(Some(changed)),
                ]),
                grace: Duration::from_secs(60),
                ..World::default()
            });
            pr.mark_ready().unwrap();
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
                observations: VecDeque::from([Ok(Some(snapshot())), answer]),
                ..World::default()
            });
            pr.mark_ready().unwrap();
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
                observations: VecDeque::from([Ok(Some(snapshot())), missing.clone()]),
                completion: VecDeque::from([missing]),
                ..World::default()
            });
            pr.mark_ready().unwrap();
            assert!(pr.ensure_ready_and_mergeable().is_err());
            let _ = pr.finish_failed_run(false);
            assert_eq!(
                world.borrow().selected[1..],
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
            pr.mark_ready().unwrap();
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
                observations: VecDeque::from([Ok(Some(snapshot())), gate]),
                ..World::default()
            });
            pr.mark_ready().unwrap();
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
        pr.mark_ready().unwrap();
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
            pr.mark_ready().unwrap();
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
            pr.mark_ready().unwrap();
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
            "delivery::pull_request::tests::an_interrupted_request_can_be_confirmed_without_clearing_interruption",
            true,
        );
    }

    #[test]
    fn an_unconfirmed_interrupted_request_keeps_its_failure_and_interruption() {
        interrupted_merge(
            "delivery::pull_request::tests::an_unconfirmed_interrupted_request_keeps_its_failure_and_interruption",
            false,
        );
    }
}
