//! Own a Command CLI child and all its I/O until completion or whole-tree cleanup.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use crate::interrupt;

mod tree;
pub(crate) use tree::stop;

#[cfg(test)]
mod faults;
#[cfg(test)]
mod tests;

/// Ordinary work stops for a recorded interrupt; completion work finishes
/// independently without clearing the Command's recorded request.
#[derive(Clone, Copy)]
pub(crate) enum Interruption {
    Ordinary,
    // The following Git/GitHub migration tickets supply production callers.
    #[cfg_attr(not(test), allow(dead_code))]
    Completion,
}

impl Interruption {
    fn requested(self) -> bool {
        matches!(self, Self::Ordinary) && interrupt::requested()
    }

    fn check(self) -> Result<()> {
        if self.requested() {
            bail!("interrupted");
        }
        Ok(())
    }
}

/// Caller-supplied diagnostics and interruption behavior for one execution.
/// This immutable value owns no child or worker and grants no other execution
/// permission to ignore the Command's recorded interruption.
#[derive(Clone, Copy)]
pub(crate) struct Control<'a> {
    pub name: &'a str,
    pub interruption: Interruption,
    pub stop: &'a dyn Fn(&mut Child),
}

/// Capture both output pipes without interpreting the child's exit status.
pub(crate) fn output(
    command: &mut Command,
    input: Option<&[u8]>,
    control: Control<'_>,
) -> Result<Output> {
    let cli = control.name.to_owned();
    let (execution, _) = execute(command, input, control, true, move |pipe| {
        read(pipe).with_context(|| format!("could not read {cli}'s stdout"))
    });
    let (status, stdout, stderr) = execution?;
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
/// A consumer panic or any startup failure returns no state. Recorded ordinary
/// interruption suppresses recovery even after a normal consumer return.
pub(crate) fn streaming<T: Send + 'static>(
    command: &mut Command,
    input: Option<&[u8]>,
    control: Control<'_>,
    mut state: T,
    consume: impl FnOnce(ChildStdout, &mut T) -> Result<()> + Send + 'static,
) -> Result<Streamed<T>> {
    // One send fits without waiting for the caller, which must first finish
    // supervising and joining all workers, including on transport failure.
    let (completed, recovered) = mpsc::sync_channel(1);
    let (execution, started) = execute(command, input, control, false, move |pipe| {
        let result = consume(pipe, &mut state);
        let _ = completed.send(state);
        result
    });
    let execution = execution.map(|(status, (), _)| status);
    control.interruption.check()?;
    if !started {
        return Err(execution.expect_err("startup failure cannot execute successfully"));
    }
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
    command: &mut Command,
    input: Option<&[u8]>,
    control: Control<'_>,
    capture_stderr: bool,
    consume: impl FnOnce(ChildStdout) -> Result<T> + Send + 'static,
) -> (Result<(ExitStatus, T, Vec<u8>)>, bool) {
    let Control {
        name,
        interruption,
        stop: _,
    } = control;
    let mut started = false;
    let result = (|| {
        if interruption.requested() {
            bail!("interrupted");
        }
        let cli = name.to_owned();
        let (wake, awakened) = mpsc::channel();
        #[cfg(test)]
        let fault = faults::Fault::from(command);
        let child = command
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
            control,
            child,
            awakened,
            stdout: None,
            stderr: None,
            stdin: None,
            completed: false,
        };
        let result = (|| {
            #[cfg(test)]
            fault.fire("stdout-pipe")?;
            let pipe = owned
                .child
                .stdout
                .take()
                .with_context(|| format!("no stdout from {name}"))?;
            let reader = if capture_stderr {
                format!("{cli}'s stdout reader")
            } else {
                "the session stream reader".to_string()
            };
            #[cfg(test)]
            fault.fire("stdout-start")?;
            #[cfg(test)]
            let stdout_fault = fault.clone();
            owned.stdout = Some(Worker::start(reader, wake.clone(), move || {
                #[cfg(test)]
                stdout_fault.fire("stdout-read")?;
                consume(pipe)
            })?);
            if capture_stderr {
                #[cfg(test)]
                fault.fire("stderr-pipe")?;
                let pipe = owned.child.stderr.take().context("no captured stderr")?;
                #[cfg(test)]
                fault.fire("stderr-start")?;
                #[cfg(test)]
                let stderr_fault = fault.clone();
                owned.stderr = Some(Worker::start(
                    format!("{cli}'s stderr reader"),
                    wake.clone(),
                    {
                        let cli = cli.clone();
                        move || {
                            #[cfg(test)]
                            stderr_fault.fire("stderr-read")?;
                            read(pipe).with_context(|| format!("could not read {cli}'s stderr"))
                        }
                    },
                )?);
            }
            if let Some(input) = input {
                #[cfg(test)]
                fault.fire("stdin-pipe")?;
                let mut pipe = owned.child.stdin.take().context("no prompt stdin")?;
                #[cfg(test)]
                fault.fire("stdin-start")?;
                let input = input.to_vec();
                #[cfg(test)]
                let stdin_fault = fault.clone();
                owned.stdin = Some(Worker::start(
                    format!("{cli}'s prompt writer"),
                    wake.clone(),
                    move || {
                        #[cfg(test)]
                        stdin_fault.fire("stdin-write")?;
                        pipe.write_all(&input)
                            .with_context(|| format!("could not write the prompt to {cli}"))
                        // Dropping the pipe closes stdin, including for an empty prompt.
                    },
                )?);
            }
            started = true;
            #[cfg(test)]
            fault.fire("wait")?;
            let status = owned.wait()?;
            let stdout = owned.stdout.as_mut().unwrap().value.take().unwrap();
            let stderr = owned
                .stderr
                .as_mut()
                .and_then(|worker| worker.value.take())
                .unwrap_or_default();
            Ok((status, stdout, stderr))
        })();
        if result.is_err() || interruption.requested() {
            owned.cleanup();
            // Interruption can also arrive during stopping or joining. Preserve
            // the original transport failure unless an interruption was observed.
            if interruption.requested() {
                bail!("interrupted");
            }
        } else {
            owned.completed = true;
        }
        result
    })();
    (interruption.check().and(result), started)
}

struct Execution<'a, T> {
    control: Control<'a>,
    child: Child,
    awakened: mpsc::Receiver<()>,
    stdout: Option<Worker<T>>,
    stderr: Option<Worker<Vec<u8>>>,
    stdin: Option<Worker<()>>,
    completed: bool,
}

impl<T> Execution<'_, T> {
    fn wait(&mut self) -> Result<ExitStatus> {
        let mut status = None;
        loop {
            if self.control.interruption.requested() {
                bail!("interrupted");
            }
            if status.is_none() {
                status = self
                    .child
                    .try_wait()
                    .with_context(|| format!("could not wait for {}", self.control.name))?;
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
                if self.control.interruption.requested() {
                    bail!("interrupted");
                }
                return Ok(status);
            }
            // Worker completion (including unwind) wakes us immediately.
            // EOF can precede child exit: when all I/O is joined, recheck
            // the child promptly without treating EOF as completion.
            let wait = if stdin_done && stdout_done && stderr_done {
                1
            } else {
                100
            };
            let _ = self.awakened.recv_timeout(Duration::from_millis(wait));
        }
    }

    fn cleanup(&mut self) {
        if self.completed {
            return;
        }
        (self.control.stop)(&mut self.child);
        let _ = self.child.wait();
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
    completed: Arc<AtomicBool>,
    value: Option<T>,
    name: String,
}

impl<T> Worker<T> {
    fn start(
        name: String,
        wake: mpsc::Sender<()>,
        work: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<Self>
    where
        T: Send + 'static,
    {
        let completed = Arc::new(AtomicBool::new(false));
        let notification = Completion {
            completed: Arc::clone(&completed),
            wake,
        };
        let handle = thread::Builder::new()
            .spawn(move || {
                let _notification = notification;
                work()
            })
            .with_context(|| format!("could not start {name}"))?;
        Ok(Self {
            handle: Some(handle),
            completed,
            value: None,
            name,
        })
    }

    /// Join only a completed worker while supervising the child. Its failure
    /// or panic must initiate cleanup even if the child has not exited yet.
    fn poll(&mut self) -> Result<bool> {
        if self.handle.is_some() && self.completed.load(Ordering::Acquire) {
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

/// Dropped both on a normal worker return and while unwinding. Set readiness
/// before waking so the supervisor can join without racing is_finished().
struct Completion {
    completed: Arc<AtomicBool>,
    wake: mpsc::Sender<()>,
}

impl Drop for Completion {
    fn drop(&mut self) {
        self.completed.store(true, Ordering::Release);
        let _ = self.wake.send(());
    }
}
