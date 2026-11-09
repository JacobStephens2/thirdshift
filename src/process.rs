//! Own a Command CLI child and all its I/O until completion or whole-tree cleanup.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use crate::interrupt;

pub(crate) mod session;
mod tree;
pub(crate) use tree::{is_alive, signal, stop};

#[cfg(test)]
mod faults;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;

/// The first executable file on PATH; callers decide whether to canonicalize
/// it and how a missing program affects their command.
pub(crate) fn find_executable(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|path| is_executable(path))
}

pub(crate) fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

/// Ordinary work stops for a recorded interrupt; completion work finishes
/// independently without clearing the Command's recorded request.
#[derive(Clone, Copy)]
pub(crate) enum Interruption {
    Ordinary,
    Completion,
}

impl Interruption {
    fn requested(self) -> bool {
        matches!(self, Self::Ordinary) && interrupt::requested()
    }

    pub(crate) fn check(self) -> Result<()> {
        if self.requested() {
            bail!("interrupted");
        }
        Ok(())
    }

    /// Wait `delay` before a retry. Ordinary waiting stops with
    /// `interrupted` as soon as the Command is interrupted; completion
    /// waiting goes on through it.
    pub(crate) fn pause(self, delay: Duration) -> Result<()> {
        match self {
            Self::Ordinary => crate::poll::sleep(delay),
            Self::Completion => {
                std::thread::sleep(delay);
                Ok(())
            }
        }
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

/// Configuration and execution borrow the same invocation, preserving its
/// optional memory observer through Harness and Session recording adapters.
pub(crate) trait Invocation {
    fn command(&mut self) -> &mut Command;
    fn parts(&mut self) -> (&mut Command, Option<&crate::scope::Scope>);
}

impl Invocation for Command {
    fn command(&mut self) -> &mut Command {
        self
    }

    fn parts(&mut self) -> (&mut Command, Option<&crate::scope::Scope>) {
        (self, None)
    }
}

/// Capture both output pipes without interpreting the child's exit status.
pub(crate) fn output(
    command: &mut dyn Invocation,
    input: Option<&[u8]>,
    control: Control<'_>,
) -> Result<Output> {
    captured(command, input, control, None, &|| Ok(()))
}

/// A Trial deadline includes pipe draining and uses the same whole-tree cleanup.
pub(crate) fn output_until(
    command: &mut dyn Invocation,
    input: Option<&[u8]>,
    control: Control<'_>,
    deadline: Instant,
    check: &dyn Fn() -> Result<()>,
) -> Result<Output> {
    captured(command, input, control, Some(deadline), check)
}

#[derive(Debug)]
pub(crate) struct TimedOut;

impl std::fmt::Display for TimedOut {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Trial timed out")
    }
}

impl std::error::Error for TimedOut {}

/// No child existed: callers can distinguish launcher failure from a child
/// that started and then failed its own work.
#[derive(Debug)]
pub(crate) struct NotStarted {
    name: String,
}

impl std::fmt::Display for NotStarted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "could not run {}", self.name)
    }
}

impl std::error::Error for NotStarted {}

fn captured(
    command: &mut dyn Invocation,
    input: Option<&[u8]>,
    control: Control<'_>,
    deadline: Option<Instant>,
    check: &dyn Fn() -> Result<()>,
) -> Result<Output> {
    let cli = control.name.to_owned();
    let (execution, _) = execute(
        command,
        input,
        control,
        true,
        deadline,
        check,
        move |pipe| read(pipe).with_context(|| format!("could not read {cli}'s stdout")),
    );
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

/// Consume stdout concurrently with prompt I/O and an optional deadline. Preserve
/// state after any normal consumer return, even when execution fails. Return
/// only after completion or whole-tree cleanup and every worker is joined.
/// A consumer panic or any startup failure returns no state. Recorded ordinary
/// interruption suppresses recovery even after a normal consumer return.
pub(crate) fn streaming<T: Send + 'static>(
    command: &mut dyn Invocation,
    input: Option<&[u8]>,
    control: Control<'_>,
    deadline: Option<Instant>,
    mut state: T,
    consume: impl FnOnce(ChildStdout, &mut T) -> Result<()> + Send + 'static,
) -> Result<Streamed<T>> {
    // One send fits without waiting for the caller, which must first finish
    // supervising and joining all workers, including on transport failure.
    let (completed, recovered) = mpsc::sync_channel(1);
    let (execution, started) = execute(
        command,
        input,
        control,
        false,
        deadline,
        &|| Ok(()),
        move |pipe| {
            let result = consume(pipe, &mut state);
            let _ = completed.send(state);
            result
        },
    );
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

fn execute<'a, T: Send + 'static>(
    invocation: &'a mut dyn Invocation,
    input: Option<&[u8]>,
    control: Control<'a>,
    capture_stderr: bool,
    deadline: Option<Instant>,
    check: &'a dyn Fn() -> Result<()>,
    consume: impl FnOnce(ChildStdout) -> Result<T> + Send + 'static,
) -> (Result<(ExitStatus, T, Vec<u8>)>, bool) {
    let (command, observer) = invocation.parts();
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
        check()?;
        let cli = name.to_owned();
        let (wake, awakened) = mpsc::channel();
        #[cfg(test)]
        let fault = faults::Fault::from(command);
        command
            .process_group(0)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped());
        // Streaming callers can retain private stderr in a file. Otherwise a
        // new Command's default already inherits it, as ordinary sessions do.
        if capture_stderr {
            command.stderr(Stdio::piped());
        }
        let child = command
            .spawn()
            .with_context(|| NotStarted { name: cli.clone() })?;
        // Install ownership immediately: every fallible step after spawn is
        // covered, including pipe extraction and starting workers.
        let mut owned = Execution {
            session: session::Session::of(command),
            observer,
            check,
            deadline,
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
    session: Option<session::Session>,
    observer: Option<&'a crate::scope::Scope>,
    check: &'a dyn Fn() -> Result<()>,
    deadline: Option<Instant>,
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
            self.observe();
            if self.control.interruption.requested() {
                bail!("interrupted");
            }
            (self.check)()?;
            if self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err(TimedOut.into());
            }
            if status.is_none() {
                status = self
                    .child
                    .try_wait()
                    .with_context(|| format!("could not wait for {}", self.control.name))?;
            }
            if status.is_some() {
                self.sweep_session();
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
                (self.check)()?;
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

    fn observe(&self) {
        if let Some(observer) = self.observer {
            observer.sample();
        }
    }

    fn cleanup(&mut self) {
        if self.completed {
            return;
        }
        (self.control.stop)(&mut self.child);
        self.sweep_session_after_stop();
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

    fn sweep_session(&mut self) {
        if self.session.is_some() {
            // Keep the tree snapshot first, even on normal completion. The
            // final sweep then catches descendants already reparented away.
            (self.control.stop)(&mut self.child);
            self.sweep_session_after_stop();
        }
    }

    fn sweep_session_after_stop(&mut self) {
        if let Some(session) = self.session.take() {
            session.sweep(self.observer);
        }
    }
}

impl<T> Drop for Execution<'_, T> {
    fn drop(&mut self) {
        self.cleanup();
        // Cleanup owns stopping, reaping and joining, including on unwind.
        // Keep the last observation after all of them; never rerun `check`.
        self.observe();
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
