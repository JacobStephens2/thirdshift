//! Stop a Command by its durable record, retaining the request for its cleanup.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::command_record::{CommandRecord, CommandTree};
use super::effects::OnMachine;

#[derive(Clone, Copy)]
pub enum Ending {
    Triage,
    Requeue,
}

#[derive(Deserialize, Serialize)]
pub struct Request {
    pub who: String,
    pub requeue: bool,
}

impl Request {
    pub fn ending(&self) -> Ending {
        if self.requeue {
            Ending::Requeue
        } else {
            Ending::Triage
        }
    }
}

fn request_path(record: &Path) -> PathBuf {
    record.with_extension("stop.json")
}

pub(super) fn requested(record: &Path) -> Option<Request> {
    serde_json::from_slice(&std::fs::read(request_path(record)).ok()?).ok()
}

fn read(record: &Path) -> Result<CommandTree> {
    serde_json::from_slice(&std::fs::read(record)?)
        .with_context(|| format!("could not read Command record {}", record.display()))
}

fn require_live(record: &CommandRecord) -> Result<()> {
    if let Some(outcome) = &record.outcome {
        bail!("cannot stop {}: Command ended: {outcome}", record.name);
    }
    if !record.live(&mut OnMachine) {
        bail!(
            "cannot stop {}: Command died (its process is gone or its identity changed)",
            record.name
        );
    }
    Ok(())
}

pub fn stop(input: &str, requeue: bool) -> Result<()> {
    let path = super::status::inspect::stop_target(input)?;
    let tree = read(&path)?;
    require_live(&tree.command)?;
    let user = std::process::Command::new("id")
        .arg("-un")
        .output()
        .context("could not identify the Unix user asking for Stop")?;
    if !user.status.success() {
        bail!("could not identify the Unix user asking for Stop");
    }
    let request = Request {
        who: String::from_utf8(user.stdout)?.trim().to_owned(),
        requeue,
    };
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("Command record has no directory")?)?;
    file.write_all(&serde_json::to_vec(&request)?)?;
    file.persist_noclobber(request_path(&path))
        .context("could not write Stop request (a Stop may already have been requested)")?;
    // Recheck the durable outcome too: cleanup may have finished while the
    // requester was being identified. Signal only the original process.
    let signal = (|| {
        require_live(&read(&path)?.command)?;
        super::process_identity::terminate(tree.command.process_id, &tree.command.process_start)
    })();
    if let Err(error) = signal {
        let _ = std::fs::remove_file(request_path(&path));
        return Err(error);
    }
    wait(
        &path,
        &mut WallClock(Instant::now()),
        &mut std::io::stdout(),
    )
}

trait Clock {
    fn elapsed(&self) -> Duration;
    fn sleep(&mut self);
}

struct WallClock(Instant);

impl Clock for WallClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
    fn sleep(&mut self) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn wait(path: &Path, clock: &mut impl Clock, output: &mut impl Write) -> Result<()> {
    let mut reminder = clock.elapsed();
    loop {
        let tree = read(path)?;
        if tree.command.outcome.is_some() || !tree.command.live(&mut OnMachine) {
            print_ending(&tree, output)?;
            return Ok(());
        }
        if clock.elapsed().saturating_sub(reminder) >= Duration::from_secs(60) {
            writeln!(output, "{} still ending", tree.command.name)?;
            output.flush()?;
            reminder = clock.elapsed();
        }
        clock.sleep();
    }
}

fn print_ending(tree: &CommandTree, output: &mut impl Write) -> Result<()> {
    writeln!(
        output,
        "{}: {}",
        tree.command.name,
        tree.command
            .outcome
            .as_deref()
            .unwrap_or("died before recording an outcome")
    )?;
    let mut resources = BTreeSet::new();
    for record in std::iter::once(&tree.command).chain(&tree.dispatched) {
        if let Some(branch) = &record.issue_branch {
            resources.insert(format!("Issue branch left: {branch}"));
        }
        if let Some(pr) = &record.pull_request {
            resources.insert(format!("Pull request left: {pr}"));
        }
    }
    for resource in resources {
        writeln!(output, "{resource}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{Choice, ChosenBy, Harness};
    use crate::issue::IssueUrl;
    use crate::logs::Work;

    #[test]
    fn waiting_reports_still_ending_each_minute_then_the_outcome_and_resources() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("command.json");
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let mut command = CommandRecord::new(
            Work::Run(&issue),
            "stamp",
            &Choice {
                harness: Harness::Claude,
                model: None,
                effort: None,
                chosen_by: ChosenBy::Default,
            },
            chrono::Local::now().fixed_offset(),
        );
        command.process_start = super::super::process_identity::start(command.process_id).unwrap();
        let tree = CommandTree {
            command,
            dispatched: Vec::new(),
        };
        std::fs::write(&path, serde_json::to_vec(&tree).unwrap()).unwrap();
        struct Minutes {
            elapsed: u64,
            path: PathBuf,
        }
        impl Clock for Minutes {
            fn elapsed(&self) -> Duration {
                Duration::from_secs(self.elapsed)
            }
            fn sleep(&mut self) {
                self.elapsed += 1;
                if self.elapsed == 125 {
                    let mut tree = read(&self.path).unwrap();
                    tree.command.outcome = Some("interrupted".into());
                    tree.command.issue_branch = Some("issue-7".into());
                    tree.command.pull_request =
                        Some("https://github.com/acme/widgets/pull/1".into());
                    std::fs::write(&self.path, serde_json::to_vec(&tree).unwrap()).unwrap();
                }
            }
        }
        let mut output = Vec::new();
        wait(
            &path,
            &mut Minutes {
                elapsed: 0,
                path: path.clone(),
            },
            &mut output,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "acme/widgets#7 still ending\nacme/widgets#7 still ending\nacme/widgets#7: interrupted\nIssue branch left: issue-7\nPull request left: https://github.com/acme/widgets/pull/1\n"
        );
    }
}
