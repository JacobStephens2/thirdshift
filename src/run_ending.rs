//! A Run's ending, both ways: how a Run, a Spec run or an Architect run
//! shows how it ended, and how a Run reads back the ending of a child Run it
//! started. The lines a Run prints are all a child Run tells what started
//! it (ADR-0006), so which line carries what is known only here.

use std::io::Write;
use std::process::ExitCode;

use crate::architect;
use crate::base_fix::Advice;
use crate::failed_run::FailedRun;
use crate::progress::{self, ChildLine};
use crate::run::Ended;

/// What starts the line naming a Failed run's session log.
const SESSION_LOG: &str = "session log: ";

/// What starts the line saying what became of the Base fix a Run took.
const BASE_FIX: &str = "Base fix: ";

/// Show how a Run or a Spec run `ended`: on stderr, what became of its Base
/// fix, if it took one, then its pull request's outcome once it reached its
/// goal, or as a Failed run does (see [`show_failure`]); its pull request's
/// URL on stdout. Returns its exit code.
pub fn show(ended: &Ended) -> ExitCode {
    Shown::of(ended).print()
}

/// Show a Failed run, or a failed Architect run: its cause and its session
/// log on stderr, and its pull request's URL, if it left one, on stdout.
/// Returns its exit code.
pub fn show_failure(failed: &FailedRun) -> ExitCode {
    Shown::of_failure(failed, &[]).print()
}

/// Show an Architect run that dispatched no run, skipped or ended on an
/// issue: the line that says how it ended on stderr, and its URLs, if it has
/// any, on stdout. Returns its exit code.
pub fn show_architect(outcome: &architect::Outcome) -> ExitCode {
    Shown {
        // Also on stderr, so the outcome shows even when stdout is captured.
        steps: vec![outcome.to_string()],
        urls: outcome.urls().into_iter().map(String::from).collect(),
        success: true,
    }
    .print()
}

/// An ending as it shows.
struct Shown {
    /// The messages of its progress lines on stderr, in order.
    steps: Vec<String>,
    /// The URLs on stdout, a line each: the pull request's, or those of the
    /// issues an Architect run ended on.
    urls: Vec<String>,
    /// Whether it exits 0.
    success: bool,
}

impl Shown {
    fn of(ended: &Ended) -> Self {
        let mut shown = match &ended.outcome {
            Ok(reached) => Shown {
                // Also on stderr, so the outcome shows even when stdout is
                // captured.
                steps: vec![format!(
                    "PR {} is {}",
                    reached.pr_url,
                    reached.goal.outcome()
                )],
                urls: vec![reached.pr_url.clone()],
                success: true,
            },
            Err(failed) => Shown::of_failure(failed, &ended.advice),
        };
        if let Some(report) = &ended.base_fix {
            shown.steps.insert(0, format!("{BASE_FIX}{report}"));
        }
        shown
    }

    /// A Failed run's cause, its `advice`, if it has any, then its session
    /// log.
    fn of_failure(failed: &FailedRun, advice: &[Advice]) -> Self {
        let mut steps = vec![format!("{:#}", failed.error)];
        steps.extend(advice.iter().map(Advice::to_string));
        if let Some(log) = &failed.log {
            steps.push(format!("{SESSION_LOG}{}", log.display()));
        }
        Shown {
            steps,
            urls: failed.pr_url.iter().cloned().collect(),
            success: false,
        }
    }

    fn print(self) -> ExitCode {
        for step in self.steps {
            progress::step(step);
        }
        for url in self.urls {
            // A failed write, as once the terminal has closed, is ignored,
            // so the Run notification still goes.
            let _ = writeln!(std::io::stdout(), "{url}");
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
    use crate::run::{Goal, Reached};

    const PR: &str = "https://github.com/acme/widgets/pull/31";
    const LOG: &str = "/home/me/.thirdshift/logs/widgets-21-implement.jsonl";
    const BASE_FIX_ISSUE: &str = "https://github.com/acme/widgets/issues/8";

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
        let shown = Shown::of(ended);
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
        let cause = "git push failed: exit status: 1\n\
                     error: failed to push some refs\n\
                     hint: fetch first";
        for log in [Some(LOG), None] {
            assert_eq!(
                read_back(&failed(cause, log, None)),
                ChildEnding {
                    outcome: failure("git push failed: exit status: 1", log),
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
        let relayed: Vec<String> = stderr(&Shown::of(&base_fix))
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
        for line in stderr(&Shown::of(&with_base_fix(
            failed("claude exited 1", Some(LOG), None),
            "not merged",
        ))) {
            reader.read(ChildLine::of(&progress::relayed(8, &line)));
        }
        assert_eq!(reader.finish("", false), nothing);
    }
}
