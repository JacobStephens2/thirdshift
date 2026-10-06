//! Own a Harness child and all its I/O until completion or whole-tree cleanup.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use super::Adapter;
use crate::interrupt;

#[cfg(test)]
mod tests;

/// Capture both output pipes without interpreting the child's exit status.
pub(super) fn output(
    adapter: &dyn Adapter,
    command: &mut Command,
    input: Option<&str>,
) -> Result<Output> {
    let cli = adapter.name();
    let (status, stdout, stderr) = execute(adapter, command, input, true, move |pipe| {
        read(pipe).with_context(|| format!("could not read {cli}'s stdout"))
    })?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Recovered consumer state accompanies an independent execution result.
#[derive(Debug)]
pub(crate) struct Streamed<T> {
    pub execution: Result<ExitStatus>,
    pub state: T,
}

/// Consume stdout concurrently with prompt I/O, inheriting stderr. Preserve
/// state after any normal consumer return, even when execution fails. Return
/// only after completion or whole-tree cleanup and every worker is joined.
/// A consumer panic or failure before it starts returns no state. Recorded
/// interruption suppresses recovery even after a normal consumer return.
pub(crate) fn streaming<T: Send + 'static>(
    adapter: &dyn Adapter,
    command: &mut Command,
    input: Option<&str>,
    mut state: T,
    consume: impl FnOnce(ChildStdout, &mut T) -> Result<()> + Send + 'static,
) -> Result<Streamed<T>> {
    // One send fits without waiting for the caller, which must first finish
    // supervising and joining all workers, including on transport failure.
    let (completed, recovered) = mpsc::sync_channel(1);
    let execution = execute(adapter, command, input, false, move |pipe| {
        let result = consume(pipe, &mut state);
        let _ = completed.send(state);
        result
    })
    .map(|(status, (), _)| status);
    interrupt::check()?;
    match recovered.try_recv() {
        Ok(state) => Ok(Streamed { execution, state }),
        Err(_) => Err(execution.expect_err("a successful consumer always returns its state")),
    }
}

fn read(mut pipe: impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn execute<T: Send + 'static>(
    adapter: &dyn Adapter,
    command: &mut Command,
    input: Option<&str>,
    capture_stderr: bool,
    consume: impl FnOnce(ChildStdout) -> Result<T> + Send + 'static,
) -> Result<(ExitStatus, T, Vec<u8>)> {
    if interrupt::requested() {
        bail!("interrupted");
    }
    let cli = adapter.name();
    let child = command
        .envs(adapter.environment().iter().copied())
        .process_group(0)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(if capture_stderr {
            Stdio::piped()
        } else {
            Stdio::inherit()
        })
        .spawn()
        .with_context(|| format!("could not run {cli}"))?;
    // Install ownership immediately: every fallible step after spawn is
    // covered, including pipe extraction and starting workers.
    let mut owned = Execution {
        adapter,
        child,
        stdout: None,
        stderr: None,
        stdin: None,
        completed: false,
    };
    let result = (|| {
        let pipe = owned
            .child
            .stdout
            .take()
            .context("no stdout from Harness")?;
        let reader = if capture_stderr {
            format!("{cli}'s stdout reader")
        } else {
            "the session stream reader".to_string()
        };
        owned.stdout = Some(Worker::start(reader, move || consume(pipe))?);
        if capture_stderr {
            let pipe = owned.child.stderr.take().context("no captured stderr")?;
            owned.stderr = Some(Worker::start(
                format!("{cli}'s stderr reader"),
                move || read(pipe).with_context(|| format!("could not read {cli}'s stderr")),
            )?);
        }
        if let Some(input) = input {
            let mut pipe = owned.child.stdin.take().context("no prompt stdin")?;
            let input = input.to_string();
            owned.stdin = Some(Worker::start(
                format!("{cli}'s prompt writer"),
                move || {
                    pipe.write_all(input.as_bytes())
                        .with_context(|| format!("could not write the prompt to {cli}"))
                    // Dropping the pipe closes stdin, including for an empty prompt.
                },
            )?);
        }
        let status = owned.wait()?;
        let stdout = owned.stdout.as_mut().unwrap().value.take().unwrap();
        let stderr = owned
            .stderr
            .as_mut()
            .and_then(|worker| worker.value.take())
            .unwrap_or_default();
        Ok((status, stdout, stderr))
    })();
    if result.is_err() || interrupt::requested() {
        owned.cleanup();
        // Interruption can also arrive during stopping or joining. Preserve
        // the original transport failure unless an interruption was observed.
        if interrupt::requested() {
            bail!("interrupted");
        }
    } else {
        owned.completed = true;
    }
    result
}

struct Execution<'a, T> {
    adapter: &'a dyn Adapter,
    child: Child,
    stdout: Option<Worker<T>>,
    stderr: Option<Worker<Vec<u8>>>,
    stdin: Option<Worker<()>>,
    completed: bool,
}

impl<T> Execution<'_, T> {
    fn wait(&mut self) -> Result<ExitStatus> {
        let mut status = None;
        loop {
            if interrupt::requested() {
                bail!("interrupted");
            }
            if status.is_none() {
                status = self
                    .child
                    .try_wait()
                    .with_context(|| format!("could not wait for {}", self.adapter.name()))?;
            }
            let stdin_done = self
                .stdin
                .as_mut()
                .map(Worker::poll)
                .transpose()?
                .unwrap_or(true);
            let stdout_done = self.stdout.as_mut().unwrap().poll()?;
            let stderr_done = self
                .stderr
                .as_mut()
                .map(Worker::poll)
                .transpose()?
                .unwrap_or(true);
            if let Some(status) = status
                && stdin_done
                && stdout_done
                && stderr_done
            {
                if interrupt::requested() {
                    bail!("interrupted");
                }
                return Ok(status);
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn cleanup(&mut self) {
        if self.completed {
            return;
        }
        self.adapter.stop(&mut self.child);
        // Pipes may still be attached if worker startup failed.
        drop(self.child.stdin.take());
        drop(self.child.stdout.take());
        drop(self.child.stderr.take());
        if let Some(worker) = &mut self.stdin {
            worker.join();
        }
        if let Some(worker) = &mut self.stdout {
            worker.join();
        }
        if let Some(worker) = &mut self.stderr {
            worker.join();
        }
        self.completed = true;
    }
}

impl<T> Drop for Execution<'_, T> {
    fn drop(&mut self) {
        self.cleanup();
    }
}

struct Worker<T> {
    handle: Option<JoinHandle<Result<T>>>,
    value: Option<T>,
    name: String,
}

impl<T> Worker<T> {
    fn start(name: String, work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<Self>
    where
        T: Send + 'static,
    {
        let handle = thread::Builder::new()
            .spawn(work)
            .with_context(|| format!("could not start {name}"))?;
        Ok(Self {
            handle: Some(handle),
            value: None,
            name,
        })
    }

    /// Join only a completed worker while supervising the child. Its failure
    /// or panic must initiate cleanup even if the child has not exited yet.
    fn poll(&mut self) -> Result<bool> {
        if self.handle.as_ref().is_some_and(JoinHandle::is_finished) {
            self.value = Some(self.handle.take().unwrap().join().map_err(|panic| {
                let cause = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                anyhow!("{} panicked: {cause}", self.name)
            })??);
        }
        Ok(self.handle.is_none())
    }

    fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
