//! A Run's ending, both ways: how a command shows how it ended, and how a
//! Run reads back the ending of a child Run it started. A command's ending
//! is read once, into an [`Account`], which its terminal display, its
//! Activity log end line and its Run notification each lay out. The lines a
//! Run prints are all a child Run tells what started it (ADR-0006,
//! ADR-0008), so which line carries what, and in what order, is known only
//! here. What a line of advice looks like is the Base fix module's to say,
//! and which lines are a child's own the progress module's.

use std::path::Path;
use std::process::ExitCode;

use crate::architect::Reviewed;
use crate::base_fix::Advice;
use crate::command::{Ending, Skip};
use crate::failed_run::FailedRun;
use crate::logs;
use crate::progress::{self, ChildLine};
use crate::run::Ended;

/// What starts the line naming a Failed run's session log.
const SESSION_LOG: &str = "session log: ";

/// What starts the line naming a Failed run's Command log.
const COMMAND_LOG: &str = "command log: ";

/// What starts the line saying what became of the Base fix a Run took.
const BASE_FIX: &str = "Base fix: ";

/// Read how a command ended as `ending`, once, into the account its display,
/// its Activity log end line and its Run notification lay out; or, for a
/// skipped pass, which reads no further, its skip.
pub fn read(ending: &Ending) -> Result<Account<'_>, &Skip> {
    match ending {
        Ending::Run(ended) => Ok(Account::of_run(ended)),
        Ending::SecurityFix { ended, findings } => Ok(Account {
            security_findings: Some(findings),
            ..Account::of_run(ended)
        }),
        Ending::Architect { review, dispatched } => {
            Ok(Account::of_architect(review, dispatched.as_ref()))
        }
        Ending::Security(audited) => {
            let account = match &audited.outcome {
                Ok(recorded) => Account {
                    outcome: "findings recorded",
                    ended: Ok(recorded.to_string()),
                    interrupted: false,
                    pr_url: None,
                    advice: &[],
                    base_fix: None,
                    log: None,
                    ticket_lines: &[],
                    review: None,
                    security_findings: None,
                    security_fix_offer: None,
                    urls: Vec::new(),
                },
                Err(failed) => Account::of_failure(failed, "audit failed"),
            };
            Ok(Account {
                security_findings: Some(&audited.findings),
                security_fix_offer: audited.offer.as_ref(),
                ..account
            })
        }
        Ending::Skipped(skip) => Err(skip),
    }
}

/// Show a skipped pass: the line that says why on stderr, and the URLs it
/// was skipped for on stdout. Returns its exit code, 0.
pub fn show_skip(skip: &Skip) -> ExitCode {
    Shown::of_skip(skip).print()
}

/// How a command that got past its skip checks ended: a Run or a Spec run
/// as it ended; an Architect run as the run it dispatched its plan as ended,
/// if it dispatched it, else as its Architecture review did.
#[derive(Debug, PartialEq, Eq)]
pub struct Account<'a> {
    /// In a word or a few, as a Run notification's subject gives it: its
    /// pull request's outcome, `failed` or `interrupted`; or, for an
    /// Architect run that dispatched nothing, how its review ended, or
    /// `review failed`.
    pub outcome: &'static str,
    /// The line that says how it ended, once it succeeded: its pull
    /// request's outcome, or how its Architecture review ended, naming the
    /// issue it ended on; else its cause.
    pub ended: Result<String, Cause>,
    /// Whether it was interrupted, as it was when it failed.
    pub interrupted: bool,
    /// Its pull request's URL, if it has one.
    pub pr_url: Option<&'a str>,
    /// What a failed Run says after its cause, if Inherited failures failed
    /// it with no Base fix taken.
    pub advice: &'a [Advice],
    /// What became of the Base fix the Run started or waited on, if any.
    pub base_fix: Option<&'a str>,
    /// The most recent session log, if a session was started.
    pub log: Option<&'a Path>,
    /// In a Spec run, a line on each Ticket, as in its summary on stderr.
    pub ticket_lines: &'a [String],
    /// In an Architect run, how its Architecture review ended.
    pub review: Option<Review<'a>>,
    /// Safe metadata from a Security run's private records, even if the audit failed.
    pub security_findings: Option<&'a [crate::security::RecordedFinding]>,
    /// An offer to allow fixing when no command or setting decided against it.
    pub security_fix_offer: Option<&'a crate::security::FixOffer>,
    /// The URLs on stdout, a line each: the pull request's, or that of the
    /// issue an Architect run's review ended on.
    pub urls: Vec<&'a str>,
}

/// How an Architect run's Architecture review ended.
#[derive(Debug, PartialEq, Eq)]
pub struct Review<'a> {
    /// How it ended, with the issue it ended on, as in `plan published:
    /// <url>`; else `failed` or `interrupted`.
    pub line: String,
    /// The URL of the plan it dispatched, if it dispatched it.
    pub dispatched: Option<&'a str>,
}

/// A failure's cause, as stderr gives it, down to what a session left
/// running.
#[derive(Debug, PartialEq, Eq)]
pub struct Cause {
    full: String,
    safeguard_refusal: Option<crate::harness::interpretation::SafeguardRefusal>,
}

impl Cause {
    /// The cause of a failure that failed with `error`.
    pub fn of(error: &anyhow::Error) -> Self {
        Self {
            full: format!("{error:#}"),
            safeguard_refusal: error.downcast_ref().copied(),
        }
    }

    /// The whole cause, of as many lines as it has.
    pub fn full(&self) -> &str {
        &self.full
    }

    /// Its first line, all of it that a short account gives.
    pub fn first_line(&self) -> &str {
        self.full.lines().next().unwrap_or_default()
    }

    /// A fixed refusal category safe to send without the Harness's private diagnostic.
    pub fn safeguard_refusal(&self) -> Option<&'static str> {
        self.safeguard_refusal.map(|refusal| refusal.description())
    }
}

impl<'a> Account<'a> {
    /// A Run or a Spec run that `ended`.
    fn of_run(ended: &'a Ended) -> Self {
        let base_fix = ended.base_fix.as_deref();
        match &ended.outcome {
            Ok(reached) => Account {
                outcome: reached.goal.outcome(),
                ended: Ok(format!(
                    "PR {} is {}",
                    reached.pr_url,
                    reached.goal.outcome()
                )),
                interrupted: false,
                pr_url: Some(&reached.pr_url),
                advice: &[],
                base_fix,
                log: reached.log.as_deref(),
                ticket_lines: &reached.ticket_lines,
                review: None,
                security_findings: None,
                security_fix_offer: None,
                urls: vec![&reached.pr_url],
            },
            Err(failed) => Account {
                advice: &ended.advice,
                base_fix,
                ..Account::of_failure(failed, "failed")
            },
        }
    }

    /// A Failed run, or a failed Architecture review, whose outcome is
    /// `failure` unless it was interrupted.
    fn of_failure(failed: &'a FailedRun, failure: &'static str) -> Self {
        Account {
            outcome: failure_outcome(failed, failure),
            ended: Err(Cause::of(&failed.error)),
            interrupted: failed.interrupted,
            pr_url: failed.pr_url.as_deref(),
            advice: &[],
            base_fix: None,
            log: failed.log.as_deref(),
            ticket_lines: &failed.ticket_lines,
            review: None,
            security_findings: None,
            security_fix_offer: None,
            urls: failed.pr_url.as_deref().into_iter().collect(),
        }
    }

    /// An Architect run whose Architecture review ended as `review`, and
    /// which dispatched its plan as the run that ended as `dispatched`, if
    /// it dispatched it.
    fn of_architect(
        review: &'a Result<Reviewed, FailedRun>,
        dispatched: Option<&'a Ended>,
    ) -> Self {
        let line = match review {
            Ok(reviewed) => format!("{}: {}", reviewed.review(), reviewed.url()),
            Err(failed) => failure_outcome(failed, "failed").to_string(),
        };
        let account = match (review, dispatched) {
            (_, Some(dispatched)) => Account::of_run(dispatched),
            (Ok(reviewed), None) => Account {
                outcome: reviewed.review(),
                ended: Ok(reviewed.to_string()),
                interrupted: false,
                pr_url: None,
                advice: &[],
                base_fix: None,
                log: None,
                ticket_lines: &[],
                review: None,
                security_findings: None,
                security_fix_offer: None,
                urls: vec![reviewed.url()],
            },
            (Err(failed), None) => Account::of_failure(failed, "review failed"),
        };
        let dispatched = dispatched.and(review.as_ref().ok()).map(Reviewed::url);
        Account {
            review: Some(Review { line, dispatched }),
            ..account
        }
    }

    /// Whether the command succeeded, and so exits 0.
    fn success(&self) -> bool {
        self.ended.is_ok()
    }

    /// Show how the command ended: on stderr, what became of its Base fix,
    /// if it took one, then the line that says how it ended once it
    /// succeeded; or its cause, its advice, if it has any, then its session
    /// log and the command's Command log; its URLs on stdout. Returns its
    /// exit code.
    pub fn show(&self) -> ExitCode {
        Shown::of(self).print()
    }

    /// How the command ended, in short, as its Activity log end line gives
    /// it: the line that says how it ended once it succeeded, or `failed: `
    /// and the first line of its cause; after the plan it dispatched, in an
    /// Architect run that dispatched one.
    pub fn summary(&self) -> String {
        let ended = match &self.ended {
            Ok(line) => line.clone(),
            Err(cause) => format!("failed: {}", cause.first_line()),
        };
        match self.review.as_ref().and_then(|review| review.dispatched) {
            Some(plan) => format!("plan {plan} dispatched: {ended}"),
            None => ended,
        }
    }
}

/// `interrupted`, or `failure` for a failure that was not an interrupt.
fn failure_outcome(failed: &FailedRun, failure: &'static str) -> &'static str {
    if failed.interrupted {
        "interrupted"
    } else {
        failure
    }
}

/// An ending as it shows.
struct Shown {
    /// The messages of its progress lines on stderr, in order.
    steps: Vec<String>,
    /// The URLs on stdout, a line each.
    urls: Vec<String>,
    /// Whether it exits 0.
    success: bool,
}

impl Shown {
    /// How a command that ended as `account` shows: see [`Account::show`].
    fn of(account: &Account) -> Self {
        let mut steps = Vec::new();
        if let Some(report) = account.base_fix {
            steps.push(format!("{BASE_FIX}{report}"));
        }
        match &account.ended {
            // Also on stderr, so the outcome shows even when stdout is
            // captured.
            Ok(line) => steps.push(line.clone()),
            Err(cause) => {
                steps.push(cause.full().to_string());
                steps.extend(account.advice.iter().map(Advice::to_string));
                if let Some(log) = account.log {
                    steps.push(format!("{SESSION_LOG}{}", log.display()));
                }
                if let Some(log) = logs::command_log_path() {
                    steps.push(format!("{COMMAND_LOG}{}", log.display()));
                }
            }
        }
        if let Some(offer) = account.security_fix_offer {
            steps.extend(offer.lines());
        }
        Shown {
            steps,
            urls: account.urls.iter().map(|url| url.to_string()).collect(),
            success: account.success(),
        }
    }

    /// How a skipped pass shows: see [`show_skip`].
    fn of_skip(skip: &Skip) -> Self {
        Shown {
            steps: vec![skip.reason.clone()],
            urls: skip.urls.clone(),
            success: true,
        }
    }

    fn print(self) -> ExitCode {
        for step in self.steps {
            progress::step(step);
        }
        for url in self.urls {
            logs::print(&url);
        }
        if self.success {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }
}

/// The ending of a child Run, as read back from what it showed.
#[derive(Debug, PartialEq, Eq)]
pub struct ChildEnding {
    /// Its pull request's URL, if it showed one, once it reached its goal,
    /// or how it failed.
    pub outcome: Result<Option<String>, ChildFailure>,
    /// What became of the Base fix it took, if any.
    pub base_fix: Option<String>,
}

/// How a child Run failed, as read back from what it showed.
#[derive(Debug, PartialEq, Eq)]
pub struct ChildFailure {
    /// The first line of its cause, or none if it showed nothing.
    pub cause: Option<String>,
    pub log: Option<String>,
}

/// Reads a child Run's ending back from what it shows: each line of its
/// stderr, as it is relayed, then how it exited. Only the child's own
/// progress lines count: a line that continues one, as the later lines of a
/// cause do, tells nothing, nor does a line the child relayed from a Run it
/// started itself. Its advice, between its cause and its session log, is
/// neither.
#[derive(Default)]
pub struct Reader {
    /// The last progress line that is no advice and names neither a session
    /// log nor a Base fix: once a failed child has shown its ending, its
    /// cause.
    cause: Option<String>,
    /// The session log named since that line.
    log: Option<String>,
    base_fix: Option<String>,
}

impl Reader {
    /// Read the next line of the child's stderr.
    pub fn read(&mut self, line: ChildLine<'_>) {
        let ChildLine::Own(message) = line else {
            return;
        };
        if let Some(report) = message.strip_prefix(BASE_FIX) {
            self.base_fix = Some(report.to_string());
        } else if let Some(log) = message.strip_prefix(SESSION_LOG) {
            self.log = Some(log.to_string());
        } else if !Advice::is_line(message) {
            self.cause = Some(message.to_string());
            self.log = None;
        }
    }

    /// How the child ended, given its `stdout` and whether it `exited_zero`.
    pub fn finish(self, stdout: &str, exited_zero: bool) -> ChildEnding {
        let outcome = if exited_zero {
            Ok(stdout.lines().last().map(String::from))
        } else {
            Err(ChildFailure {
                cause: self.cause,
                log: self.log,
            })
        };
        ChildEnding {
            outcome,
            base_fix: self.base_fix,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::anyhow;

    use super::*;
    use crate::issue::IssueUrl;
    use crate::run::{Goal, Reached};

    const PR: &str = "https://github.com/acme/widgets/pull/31";
    const LOG: &str = "/home/me/.thirdshift/logs/widgets-21-implement.jsonl";
    const BASE_FIX_ISSUE: &str = "https://github.com/acme/widgets/issues/8";
    const PLAN: &str = "https://github.com/acme/widgets/issues/40";

    fn plan() -> Reviewed {
        Reviewed::PlanReady(IssueUrl::parse(PLAN).unwrap())
    }

    fn failed_run(cause: &str) -> FailedRun {
        FailedRun {
            error: anyhow!("{cause}"),
            pr_url: None,
            log: Some(PathBuf::from(LOG)),
            interrupted: false,
            ticket_lines: Vec::new(),
        }
    }

    fn interrupted(failed: FailedRun) -> FailedRun {
        FailedRun {
            error: anyhow!("interrupted"),
            interrupted: true,
            ..failed
        }
    }

    fn cause(cause: &str) -> Cause {
        Cause::of(&anyhow!("{cause}"))
    }

    fn strings(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| line.to_string()).collect()
    }

    fn advice() -> Vec<Advice> {
        vec![
            Advice {
                label: "Base check",
                value: "test: https://ci.example/main/test".to_string(),
            },
            Advice {
                label: "Or set",
                value: "base.fix = true".to_string(),
            },
        ]
    }

    /// How a Run that reached its goal reads, with neither a Base fix nor
    /// Ticket lines.
    fn reached_account(goal: Goal) -> Account<'static> {
        Account {
            outcome: goal.outcome(),
            ended: Ok(format!("PR {PR} is {}", goal.outcome())),
            interrupted: false,
            pr_url: Some(PR),
            advice: &[],
            base_fix: None,
            log: Some(Path::new(LOG)),
            ticket_lines: &[],
            review: None,
            security_findings: None,
            security_fix_offer: None,
            urls: vec![PR],
        }
    }

    /// How a Run that failed as `cause`, with its session log and no pull
    /// request, reads.
    fn failed_account(cause: &str) -> Account<'static> {
        Account {
            outcome: "failed",
            ended: Err(Cause::of(&anyhow!("{cause}"))),
            interrupted: false,
            pr_url: None,
            advice: &[],
            base_fix: None,
            log: Some(Path::new(LOG)),
            ticket_lines: &[],
            review: None,
            security_findings: None,
            security_fix_offer: None,
            urls: Vec::new(),
        }
    }

    /// How a command that got past its skip checks, ending as `ending`,
    /// reads.
    fn read_account(ending: &Ending) -> Account<'_> {
        let Ok(account) = read(ending) else {
            panic!("a command that got past its skip checks read as skipped");
        };
        account
    }

    #[test]
    fn a_run_that_reached_its_goal_reads_as_its_pull_requests_outcome() {
        for goal in [Goal::ReadyForReview, Goal::Merged] {
            let ending = Ending::Run(reached(goal));
            assert_eq!(read_account(&ending), reached_account(goal));
        }
        let base_fix = format!("{BASE_FIX_ISSUE} merged");
        let ended = with_base_fix(reached(Goal::Merged), "merged");
        assert_eq!(
            Account::of_run(&ended),
            Account {
                base_fix: Some(&base_fix),
                ..reached_account(Goal::Merged)
            }
        );
    }

    #[test]
    fn a_failed_run_reads_as_its_cause_its_advice_and_its_base_fix() {
        let advice = advice();
        let base_fix = format!("{BASE_FIX_ISSUE} not merged");
        let ended = Ended {
            advice: advice.clone(),
            ..with_base_fix(
                failed("claude exited 1\nno such model", Some(LOG), Some(PR)),
                "not merged",
            )
        };
        assert_eq!(
            Account::of_run(&ended),
            Account {
                pr_url: Some(PR),
                advice: &advice,
                base_fix: Some(&base_fix),
                urls: vec![PR],
                ..failed_account("claude exited 1\nno such model")
            }
        );
    }

    #[test]
    fn an_interrupted_run_reads_as_interrupted() {
        let ended = Ended {
            outcome: Err(interrupted(failed_run("claude exited 1"))),
            base_fix: None,
            advice: Vec::new(),
        };
        assert_eq!(
            Account::of_run(&ended),
            Account {
                outcome: "interrupted",
                interrupted: true,
                ..failed_account("interrupted")
            }
        );
    }

    #[test]
    fn a_spec_run_reads_with_its_ticket_lines() {
        let tickets = strings(&["#21 landed in #31", "#22 failed: claude exited 1"]);
        let ended = Ended {
            outcome: Err(FailedRun {
                ticket_lines: tickets.clone(),
                ..failed_run("Tickets not done: #22")
            }),
            base_fix: None,
            advice: Vec::new(),
        };
        assert_eq!(
            Account::of_run(&ended),
            Account {
                ticket_lines: &tickets,
                ..failed_account("Tickets not done: #22")
            }
        );
        let mut ended = reached(Goal::ReadyForReview);
        if let Ok(reached) = &mut ended.outcome {
            reached.ticket_lines = tickets.clone();
        }
        assert_eq!(
            Account::of_run(&ended),
            Account {
                ticket_lines: &tickets,
                ..reached_account(Goal::ReadyForReview)
            }
        );
    }

    #[test]
    fn an_architect_run_that_dispatched_its_plan_reads_as_that_run_after_its_review() {
        let ending = Ending::Architect {
            review: Ok(plan()),
            dispatched: Some(reached(Goal::ReadyForReview)),
        };
        assert_eq!(
            read_account(&ending),
            Account {
                review: Some(Review {
                    line: format!("plan published: {PLAN}"),
                    dispatched: Some(PLAN),
                }),
                ..reached_account(Goal::ReadyForReview)
            }
        );
        let ending = Ending::Architect {
            review: Ok(plan()),
            dispatched: Some(failed("CI red on test", Some(LOG), None)),
        };
        assert_eq!(
            read_account(&ending),
            Account {
                review: Some(Review {
                    line: format!("plan published: {PLAN}"),
                    dispatched: Some(PLAN),
                }),
                ..failed_account("CI red on test")
            }
        );
    }

    #[test]
    fn an_architect_run_that_dispatched_nothing_reads_as_its_review() {
        let issue = || IssueUrl::parse(PLAN).unwrap();
        for (reviewed, outcome, line) in [
            (
                plan(),
                "plan published",
                format!("plan {PLAN} is ready for an agent"),
            ),
            (
                Reviewed::IdeaFiled(issue()),
                "idea filed",
                format!("no Strong candidate: the Architecture review filed the idea {PLAN}"),
            ),
            (
                Reviewed::AlreadyFiled(issue()),
                "idea already filed",
                format!(
                    "no Strong candidate: {PLAN} already covers the Architecture review's \
                     top recommendation, so it filed nothing"
                ),
            ),
        ] {
            let ending = Ending::Architect {
                review: Ok(reviewed),
                dispatched: None,
            };
            assert_eq!(
                read_account(&ending),
                Account {
                    outcome,
                    ended: Ok(line),
                    interrupted: false,
                    pr_url: None,
                    advice: &[],
                    base_fix: None,
                    log: None,
                    ticket_lines: &[],
                    review: Some(Review {
                        line: format!("{outcome}: {PLAN}"),
                        dispatched: None,
                    }),
                    security_findings: None,
                    security_fix_offer: None,
                    urls: vec![PLAN],
                }
            );
        }
    }

    #[test]
    fn an_architect_run_whose_review_failed_reads_as_the_failure() {
        let ending = Ending::Architect {
            review: Err(failed_run("claude exited 1\nno such model")),
            dispatched: None,
        };
        assert_eq!(
            read_account(&ending),
            Account {
                outcome: "review failed",
                review: Some(Review {
                    line: "failed".to_string(),
                    dispatched: None,
                }),
                ..failed_account("claude exited 1\nno such model")
            }
        );
        let ending = Ending::Architect {
            review: Err(interrupted(failed_run("claude exited 1"))),
            dispatched: None,
        };
        assert_eq!(
            read_account(&ending),
            Account {
                outcome: "interrupted",
                interrupted: true,
                review: Some(Review {
                    line: "interrupted".to_string(),
                    dispatched: None,
                }),
                ..failed_account("interrupted")
            }
        );
    }

    #[test]
    fn a_skipped_pass_reads_no_further_than_its_skip() {
        let ending = Ending::Skipped(Skip {
            reason: "no Ready issue on acme/widgets".to_string(),
            urls: Vec::new(),
        });
        let Err(skip) = read(&ending) else {
            panic!("a skipped pass read into an account");
        };
        assert_eq!(skip.reason, "no Ready issue on acme/widgets");
    }

    #[test]
    fn a_cause_cuts_to_its_first_line() {
        assert_eq!(
            cause("claude exited 1\nno such model").first_line(),
            "claude exited 1"
        );
        assert_eq!(cause("origin mismatch").first_line(), "origin mismatch");
        assert_eq!(cause("").first_line(), "");
    }

    /// The lines `account` shows on stderr and stdout, and whether it exits
    /// 0.
    fn shown(account: &Account) -> (Vec<String>, Vec<String>, bool) {
        let shown = Shown::of(account);
        (shown.steps, shown.urls, shown.success)
    }

    #[test]
    fn a_success_shows_its_base_fix_then_how_it_ended_and_sums_up_as_that_line() {
        let base_fix = format!("{BASE_FIX_ISSUE} merged");
        let account = Account {
            base_fix: Some(&base_fix),
            ..reached_account(Goal::Merged)
        };
        assert_eq!(
            shown(&account),
            (
                vec![
                    format!("Base fix: {BASE_FIX_ISSUE} merged"),
                    format!("PR {PR} is merged"),
                ],
                strings(&[PR]),
                true
            )
        );
        assert_eq!(account.summary(), format!("PR {PR} is merged"));
    }

    #[test]
    fn a_failure_shows_its_cause_its_advice_and_its_session_log_and_sums_up_its_first_line() {
        let advice = advice();
        let account = Account {
            pr_url: Some(PR),
            advice: &advice,
            urls: vec![PR],
            ..failed_account("claude exited 1\nno such model")
        };
        assert_eq!(
            shown(&account),
            (
                vec![
                    "claude exited 1\nno such model".to_string(),
                    "Base check: test: https://ci.example/main/test".to_string(),
                    "Or set: base.fix = true".to_string(),
                    format!("session log: {LOG}"),
                ],
                strings(&[PR]),
                false
            )
        );
        assert_eq!(account.summary(), "failed: claude exited 1");
        let account = Account {
            log: None,
            ..failed_account("interrupted")
        };
        assert_eq!(
            shown(&account),
            (strings(&["interrupted"]), Vec::new(), false)
        );
        assert_eq!(account.summary(), "failed: interrupted");
    }

    #[test]
    fn an_architect_run_sums_up_the_plan_it_dispatched_before_how_that_run_ended() {
        let review = |dispatched| {
            Some(Review {
                line: format!("plan published: {PLAN}"),
                dispatched,
            })
        };
        let account = Account {
            review: review(Some(PLAN)),
            ..reached_account(Goal::ReadyForReview)
        };
        assert_eq!(
            account.summary(),
            format!("plan {PLAN} dispatched: PR {PR} is ready for review")
        );
        assert_eq!(
            shown(&account),
            (
                vec![format!("PR {PR} is ready for review")],
                strings(&[PR]),
                true
            )
        );
        let account = Account {
            review: review(Some(PLAN)),
            ..failed_account("CI red on test")
        };
        assert_eq!(
            account.summary(),
            format!("plan {PLAN} dispatched: failed: CI red on test")
        );
        let line = format!("plan {PLAN} is ready for an agent");
        let account = Account {
            outcome: "plan published",
            ended: Ok(line.clone()),
            pr_url: None,
            log: None,
            review: review(None),
            urls: vec![PLAN],
            ..reached_account(Goal::ReadyForReview)
        };
        assert_eq!(account.summary(), line);
        assert_eq!(shown(&account), (vec![line], strings(&[PLAN]), true));
    }

    #[test]
    fn a_skipped_pass_shows_its_reason_and_its_urls_and_exits_0() {
        let reason = format!("Ready issue #40 \"Export\" goes first: {PLAN}");
        let skip = Skip {
            reason: reason.clone(),
            urls: strings(&[PLAN]),
        };
        let shown = Shown::of_skip(&skip);
        assert_eq!(
            (shown.steps, shown.urls, shown.success),
            (vec![reason], strings(&[PLAN]), true)
        );
    }

    fn reached(goal: Goal) -> Ended {
        Ended {
            outcome: Ok(Reached {
                pr_url: PR.to_string(),
                goal,
                log: Some(PathBuf::from(LOG)),
                ticket_lines: Vec::new(),
            }),
            base_fix: None,
            advice: Vec::new(),
        }
    }

    fn failed(cause: &str, log: Option<&str>, pr_url: Option<&str>) -> Ended {
        Ended {
            outcome: Err(FailedRun {
                error: anyhow!("{cause}"),
                pr_url: pr_url.map(String::from),
                log: log.map(PathBuf::from),
                interrupted: false,
                ticket_lines: Vec::new(),
            }),
            base_fix: None,
            advice: Vec::new(),
        }
    }

    fn with_base_fix(ended: Ended, became: &str) -> Ended {
        Ended {
            base_fix: Some(format!("{BASE_FIX_ISSUE} {became}")),
            ..ended
        }
    }

    /// The lines on the stderr of a Run that ends as `shown`.
    fn stderr(shown: &Shown) -> Vec<String> {
        shown
            .steps
            .iter()
            .flat_map(|step| {
                progress::stamped(step)
                    .lines()
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// What a Run reads back from a child Run that `ended`, having first
    /// put the lines `before` on its stderr.
    fn read_back_after(before: &[String], ended: &Ended) -> ChildEnding {
        let shown = Shown::of(&Account::of_run(ended));
        let mut reader = Reader::default();
        for line in before.iter().chain(&stderr(&shown)) {
            reader.read(ChildLine::of(line));
        }
        let stdout = shown
            .urls
            .iter()
            .map(|url| format!("{url}\n"))
            .collect::<String>();
        reader.finish(&stdout, shown.success)
    }

    fn read_back(ended: &Ended) -> ChildEnding {
        read_back_after(&[], ended)
    }

    fn failure(cause: &str, log: Option<&str>) -> Result<Option<String>, ChildFailure> {
        Err(ChildFailure {
            cause: Some(cause.to_string()),
            log: log.map(String::from),
        })
    }

    #[test]
    fn a_run_that_reached_its_goal_reads_back_as_its_pull_request() {
        for goal in [Goal::ReadyForReview, Goal::Merged] {
            assert_eq!(
                read_back(&reached(goal)),
                ChildEnding {
                    outcome: Ok(Some(PR.to_string())),
                    base_fix: None,
                }
            );
        }
    }

    #[test]
    fn a_run_that_reached_its_goal_after_a_base_fix_reads_back_with_what_became_of_it() {
        assert_eq!(
            read_back(&with_base_fix(reached(Goal::Merged), "merged")),
            ChildEnding {
                outcome: Ok(Some(PR.to_string())),
                base_fix: Some(format!("{BASE_FIX_ISSUE} merged")),
            }
        );
    }

    #[test]
    fn a_failed_run_reads_back_as_its_cause_and_its_session_log() {
        assert_eq!(
            read_back(&failed("claude exited 1", Some(LOG), None)),
            ChildEnding {
                outcome: failure("claude exited 1", Some(LOG)),
                base_fix: None,
            }
        );
    }

    #[test]
    fn a_failed_run_with_no_session_log_reads_back_as_its_cause() {
        assert_eq!(
            read_back(&failed("origin mismatch", None, None)),
            ChildEnding {
                outcome: failure("origin mismatch", None),
                base_fix: None,
            }
        );
    }

    #[test]
    fn a_failed_run_that_left_a_pull_request_open_reads_back_as_a_failure() {
        assert_eq!(
            read_back(&failed("CI red on test", Some(LOG), Some(PR))),
            ChildEnding {
                outcome: failure("CI red on test", Some(LOG)),
                base_fix: None,
            }
        );
    }

    #[test]
    fn a_cause_of_several_lines_reads_back_as_its_first_line() {
        let cause = "git fetch origin issue-21 failed: \
                     error: fetching ref refs/remotes/origin/issue-21 failed\n\
                     From https://github.com/acme/widgets\n\
                     * branch            issue-21   -> FETCH_HEAD";
        for log in [Some(LOG), None] {
            assert_eq!(
                read_back(&failed(cause, log, None)),
                ChildEnding {
                    outcome: failure(
                        "git fetch origin issue-21 failed: \
                         error: fetching ref refs/remotes/origin/issue-21 failed",
                        log
                    ),
                    base_fix: None,
                }
            );
        }
    }

    #[test]
    fn the_advice_a_failed_run_gives_is_neither_its_cause_nor_its_session_log() {
        let advice = vec![
            Advice {
                label: "Base check",
                value: "test: https://ci.example/main/test".to_string(),
            },
            Advice {
                label: "Retry with",
                value: "thirdshift base-fix https://github.com/acme/widgets/issues/21".to_string(),
            },
            Advice {
                label: "Or set",
                value: "base.fix = true".to_string(),
            },
        ];
        let cause = "CI red on test, which fails on main too: fix main first";
        for log in [Some(LOG), None] {
            let ended = Ended {
                advice: advice.clone(),
                ..failed(cause, log, Some(PR))
            };
            assert_eq!(
                read_back(&ended),
                ChildEnding {
                    outcome: failure(cause, log),
                    base_fix: None,
                }
            );
        }
    }

    #[test]
    fn a_run_that_failed_after_a_base_fix_reads_back_with_what_became_of_it() {
        let cause = format!("Base fix {BASE_FIX_ISSUE} failed: claude exited 1");
        assert_eq!(
            read_back(&with_base_fix(
                failed(&cause, Some(LOG), Some(PR)),
                "not merged"
            )),
            ChildEnding {
                outcome: failure(&cause, Some(LOG)),
                base_fix: Some(format!("{BASE_FIX_ISSUE} not merged")),
            }
        );
    }

    #[test]
    fn lines_relayed_from_a_child_of_its_own_are_not_its_ending() {
        // A Ticket's Run relays its Base fix, which took a Base fix of its
        // own for the sake of the test, then ends with none of what it
        // relayed.
        let base_fix = with_base_fix(
            failed(
                "claude exited 1\nno such model",
                Some("/logs/8.jsonl"),
                None,
            ),
            "not merged",
        );
        let relayed: Vec<String> = stderr(&Shown::of(&Account::of_run(&base_fix)))
            .iter()
            .map(|line| progress::relayed(8, line))
            .collect();
        assert_eq!(
            read_back_after(&relayed, &failed("interrupted", None, None)),
            ChildEnding {
                outcome: failure("interrupted", None),
                base_fix: None,
            }
        );
        assert_eq!(
            read_back_after(&relayed, &reached(Goal::Merged)),
            ChildEnding {
                outcome: Ok(Some(PR.to_string())),
                base_fix: None,
            }
        );
    }

    #[test]
    fn a_child_that_showed_nothing_reads_back_with_no_cause() {
        let nothing = ChildEnding {
            outcome: Err(ChildFailure {
                cause: None,
                log: None,
            }),
            base_fix: None,
        };
        assert_eq!(Reader::default().finish("", false), nothing);

        // Nor is what it relayed from a child of its own what it showed.
        let mut reader = Reader::default();
        let base_fix = with_base_fix(failed("claude exited 1", Some(LOG), None), "not merged");
        for line in stderr(&Shown::of(&Account::of_run(&base_fix))) {
            reader.read(ChildLine::of(&progress::relayed(8, &line)));
        }
        assert_eq!(reader.finish("", false), nothing);
    }
}
