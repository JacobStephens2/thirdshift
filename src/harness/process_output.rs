//! Capture a Harness check or export while retaining session interruption rules.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use super::Adapter;

/// Isolate the CLI's process group and drain both pipes while watching for
/// interruption. Checks and exports use the same fixed environment and
/// whole-process-tree stop as the adapter's live sessions.
pub(super) fn output(
    adapter: &dyn Adapter,
    command: &mut Command,
    input: Option<&str>,
) -> Result<Output> {
    if crate::interrupt::requested() {
        bail!("interrupted");
    }
    let cli = adapter.name();
    let mut child = command
        .envs(adapter.environment().iter().copied())
        .process_group(0)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not run {cli}"))?;

    fn read(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<std::io::Result<Vec<u8>>> {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.read_to_end(&mut bytes)?;
            Ok(bytes)
        })
    }
    let stdout = read(child.stdout.take().context("no captured stdout")?);
    let stderr = read(child.stderr.take().context("no captured stderr")?);
    if let Some(input) = input
        && let Some(mut stdin) = child.stdin.take()
        && let Err(error) = stdin.write_all(input.as_bytes())
    {
        adapter.stop(&mut child);
        let _ = stdout.join();
        let _ = stderr.join();
        return Err(error).with_context(|| format!("could not write the prompt to {cli}"));
    }
    let status = loop {
        if crate::interrupt::requested() {
            adapter.stop(&mut child);
            let _ = stdout.join();
            let _ = stderr.join();
            bail!("interrupted");
        }
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("could not wait for {cli}"))?
            // A child command can retain either pipe after the CLI exits.
            // Keep watching interruptions until both readers can be joined.
            && stdout.is_finished()
            && stderr.is_finished()
        {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };
    Ok(Output {
        status,
        stdout: stdout
            .join()
            .map_err(|_| anyhow!("{cli}'s stdout reader panicked"))??,
        stderr: stderr
            .join()
            .map_err(|_| anyhow!("{cli}'s stderr reader panicked"))??,
    })
}
