//! Terminal decisions and optional release Experiments.
use std::io::{self, IsTerminal, Write};

use anyhow::{Context, Result, ensure};

use super::*;

pub(super) fn experiment(git: &Git, cut: &recovery::Cut, requested: bool) -> Result<()> {
    if !requested {
        progress::step("skipping the release Experiment (use --with-experiment to run it)");
        return Ok(());
    }
    let previous = cut
        .previous
        .as_deref()
        .context("release Experiment requires a previous release tag")?;
    progress::step(format_args!(
        "checking release Experiment at {} against {previous}",
        cut.base
    ));
    let Some(mut view) = crate::evals::experiment::for_release(git.dir(), &cut.base, previous)?
    else {
        return Ok(());
    };
    let mut line = poll::WaitingLine::new();
    loop {
        let verdict = view
            .verdict
            .as_ref()
            .context("release Experiment has no Verdict")?;
        match verdict.verdict {
            crate::evals::experiment::Conclusion::Pass => return Ok(()),
            crate::evals::experiment::Conclusion::Fail
            | crate::evals::experiment::Conclusion::Inconclusive => {
                ensure!(
                    io::stdin().is_terminal(),
                    "Experiment {}; unattended release has no terminal override",
                    verdict.outcome()
                );
                progress::step(format_args!(
                    "Experiment {}; release anyway? [y/N]",
                    verdict.outcome()
                ));
                ensure!(
                    matches!(answer()?.as_str(), "y" | "yes"),
                    "Experiment {}; release declined",
                    verdict.outcome()
                );
                return Ok(());
            }
            crate::evals::experiment::Conclusion::WaitingOnRechecks
            | crate::evals::experiment::Conclusion::WaitingOnLeaks
            | crate::evals::experiment::Conclusion::WaitingOnPeople => {
                if line.should_report(()) {
                    progress::step(format_args!(
                        "Experiment {} {}; resolve pending rechecks, leak decisions or pairwise calls with thirdshift experiments {}",
                        view.description.id,
                        verdict.outcome(),
                        view.description.id
                    ));
                }
                poll::pause()?;
                view = crate::evals::experiment::read(Some(&view.description.id))?
                    .pop()
                    .context("release Experiment disappeared")?;
            }
        }
    }
}

pub(super) fn review(summary: String, requested: bool) -> Result<String> {
    if !requested {
        return Ok(summary);
    }
    ensure!(
        io::stdin().is_terminal(),
        "release --review requires a terminal on standard input"
    );
    eprintln!("\n{summary}\n");
    loop {
        eprint!("accept summary? [y]es / [e]dit / [n]o ");
        io::stderr().flush()?;
        match answer()?.as_str() {
            "y" | "yes" => return Ok(summary),
            "e" | "edit" => {
                let mut file = tempfile::Builder::new().suffix(".md").tempfile()?;
                file.write_all(summary.as_bytes())?;
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
                let mut command = std::process::Command::new("sh");
                command
                    .args(["-c", &format!("{editor} \"$1\""), "release-editor"])
                    .arg(file.path());
                let mut child = Editor(
                    command
                        .spawn()
                        .context("could not start release summary editor")?,
                );
                let status = loop {
                    if let Some(status) = child.0.try_wait()? {
                        break status;
                    }
                    interrupt::check()?;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                };
                ensure!(
                    status.success(),
                    "release summary editor failed; nothing was pushed"
                );
                return fs::read_to_string(file.path())
                    .context("could not read edited release summary");
            }
            "n" | "no" | "" => anyhow::bail!("release stopped at the review; nothing was pushed"),
            _ => {}
        }
    }
}

fn answer() -> Result<String> {
    let mut line = String::new();
    ensure!(
        io::stdin().read_line(&mut line)? > 0,
        "release cancelled at end of input"
    );
    Ok(line.trim().to_ascii_lowercase())
}

struct Editor(std::process::Child);
impl Drop for Editor {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
