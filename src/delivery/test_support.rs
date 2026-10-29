use super::stage::Recorded;
pub(crate) use super::stage::Stage;
use super::*;
use crate::failed_run::PolicyRefusal;
use anyhow::{anyhow, bail};

pub(crate) const PR_URL: &str = "https://github.com/acme/widgets/pull/12";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fails {
    CatchUp,
    Session,
    Push,
    Annotation,
    Checklist,
    Ready,
    Goal,
    Preserve,
    Finish,
    Delete,
    Close,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Call {
    CatchUp,
    Session(Kind),
    Push,
    Annotation,
    Checklist,
    Ready,
    Goal(Goal),
    Preserve(String),
    Finish(bool),
    Delete,
    Close(String),
    Warning(String),
    Step(String),
}

/// Orchestration adapter: PR policy is tested through PullRequest's
/// interface, so this only scripts the outcomes Delivery consumes.
#[derive(Default)]
pub(crate) struct Scripted {
    pub(crate) calls: Vec<Call>,
    /// The stages entered, apart from the calls.
    pub(crate) stages: Vec<Stage>,
    /// Whether the Delivery runs a Security review, and whether that
    /// review ends incomplete, a Security hold.
    pub(crate) reviews_security: bool,
    pub(crate) security_held: bool,
    /// Whether taking the PR to its goal runs a Repair session first.
    pub(crate) repairs: bool,
    pub(crate) failing: Option<Fails>,
    pub(crate) interrupted: bool,
    pub(crate) refused: bool,
    pub(crate) issue_closed: bool,
    pub(crate) log: Option<PathBuf>,
}

impl Scripted {
    fn check(&self, at: Fails) -> Result<()> {
        if self.failing == Some(at) {
            bail!("{at:?} failed");
        }
        Ok(())
    }
}

impl Outside for Scripted {
    fn record_security_review(&mut self, _: &str) -> Result<()> {
        Ok(())
    }
    fn stage(&mut self, stage: Stage) {
        self.stages.push(stage);
    }
    fn security_review(&mut self, _: &str) -> Result<SecurityOutcome> {
        use crate::security::review::Review;
        let log = PathBuf::from("/logs/security-review.jsonl");
        self.log = Some(log.clone());
        let review = if self.security_held {
            Review::from_final_message(Some("Security review: incomplete: timed out"))
        } else {
            Review::from_final_message(Some(
                "Security review: {\"unaddressed_count\":0,\"findings\":[],\"pre_existing_count\":0}",
            ))
        };
        Ok(SecurityOutcome::of(review, Some(log)))
    }
    fn catch_up(&mut self) -> Result<()> {
        self.calls.push(Call::CatchUp);
        self.check(Fails::CatchUp)
    }
    fn session(&mut self, kind: Kind, _: &str) -> Result<Option<String>> {
        self.calls.push(Call::Session(kind));
        self.log = Some(PathBuf::from(format!("/logs/{}.jsonl", kind.token())));
        self.check(Fails::Session).map(|()| None)
    }
    fn confirm_needs_info(&mut self) -> Result<()> {
        Ok(())
    }
    fn push(&mut self) -> Result<()> {
        self.calls.push(Call::Push);
        self.check(Fails::Push)
    }
    fn write_built_with(&mut self, _: &Choice) -> Result<()> {
        self.calls.push(Call::Annotation);
        self.check(Fails::Annotation)
    }
    fn mark_pr_ready(&mut self, checklist: Option<&str>) -> Result<Identified> {
        if checklist.is_some() {
            self.calls.push(Call::Checklist);
            self.check(Fails::Checklist)?;
        }
        self.calls.push(Call::Ready);
        self.check(Fails::Ready)?;
        Ok(Identified {
            number: 12,
            url: PR_URL.to_string(),
        })
    }
    fn take_to_goal(&mut self, _: &str, goal: Goal) -> Result<()> {
        self.calls.push(Call::Goal(goal));
        if self.repairs {
            self.log = Some(PathBuf::from("/logs/repair-1.jsonl"));
        }
        if self.refused {
            return Err(anyhow!("merge refused").context(PolicyRefusal::NothingToFix));
        }
        self.check(Fails::Goal)
    }
    fn delete_branch(&mut self) -> Result<()> {
        self.calls.push(Call::Delete);
        self.check(Fails::Delete)
    }
    fn issue_is_open(&mut self) -> Result<bool> {
        Ok(!self.issue_closed)
    }
    fn close_issue(&mut self, comment: &str) -> Result<()> {
        self.calls.push(Call::Close(comment.to_string()));
        self.check(Fails::Close)
    }
    fn interrupted(&mut self) -> bool {
        self.interrupted
    }
    fn step(&mut self, line: String) {
        self.calls.push(Call::Step(line));
    }
    fn warn(&mut self, _: &anyhow::Error, warning: String) {
        self.calls.push(Call::Warning(warning));
    }
}

impl FailedOutside for Scripted {
    fn preserve_failed_run(&mut self, reason: &str) -> Result<Preserved> {
        self.calls.push(Call::Preserve(reason.to_string()));
        self.check(Fails::Preserve)?;
        Ok(Preserved::Pushed)
    }
    fn finish_pr(&mut self, keep_ready: bool) -> Result<Option<String>> {
        self.calls.push(Call::Finish(keep_ready));
        self.check(Fails::Finish)?;
        Ok(Some(PR_URL.to_string()))
    }
    fn interrupted(&mut self) -> bool {
        self.interrupted
    }
    fn step(&mut self, line: String) {
        self.calls.push(Call::Step(line));
    }
}

fn deliver(outside: &mut Scripted, goal: Goal, spec: bool) -> Result<String, FailedRun> {
    let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
    let route = Route {
        security_review: outside.reviews_security,
        issue: &issue,
        base: "main",
        branch: "issue-7",
        goal,
        harness: &Choice::default(),
    };
    let opening = Opening {
        session: if spec {
            OpeningSession::SpecReview
        } else {
            OpeningSession::Implementation
        },
        prompt: String::new(),
    };
    let outcome = if spec {
        route.process_steps(
            outside,
            &opening,
            crate::processes::spec::sequence().unwrap().delivery,
            Some("Tickets"),
        )
    } else {
        route.process_steps(
            outside,
            &opening,
            crate::processes::ticket::steps().unwrap(),
            None,
        )
    };
    outcome
        .map(|resolution| resolution.url().to_string())
        .map_err(|error| {
            // The real adapters enter each stage the steps name.
            let tracker = Tracker::new(opening.session.stage(), Recorded::default());
            for stage in &outside.stages {
                tracker.enter(*stage);
            }
            let stopped_at = stopped_at(&tracker, &error);
            fail(
                outside,
                stopped_at,
                PathBuf::from("/worktree"),
                outside.log.clone(),
                error,
            )
        })
}

/// The stage `failed` stopped at.
pub(crate) fn stopped(failed: &FailedRun) -> recovery::Stage {
    failed.recovery.as_ref().expect("no recovery").stage()
}

pub(crate) fn spec_delivery(outside: &mut Scripted, goal: Goal) -> Result<String, FailedRun> {
    deliver(outside, goal, true)
}

pub(crate) fn ticket_run(outside: &mut Scripted, goal: Goal) -> Result<String, FailedRun> {
    deliver(outside, goal, false)
}
