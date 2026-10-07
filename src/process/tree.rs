//! The shared CLI stop path, including escaped process groups.

use std::collections::{BTreeSet, HashMap};
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::progress;

const POLL: Duration = Duration::from_millis(100);
const STOP_GRACE: Duration = Duration::from_secs(10);

/// Give each of the Harness's stop signals the existing grace period. Once
/// the CLI exits, skip the remaining grace periods but still kill its work.
pub(crate) fn stop(child: &mut Child, signals: &[libc::c_int]) {
    let pid = child.id() as libc::pid_t;
    let mut tree = ProcessTree {
        pids: BTreeSet::from([pid]),
        groups: BTreeSet::from([pid]),
    };
    tree.refresh();
    'signals: for &signal in signals {
        tree.signal(signal);
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                break 'signals;
            }
            thread::sleep(POLL);
        }
    }
    // Retain the first snapshot: the CLI may already have exited, leaving
    // its descendants reparented. Known descendants also seed this lookup
    // so their new children are found even when the CLI is gone.
    tree.refresh();
    tree.signal(libc::SIGKILL);
    let _ = child.wait();
}

struct ProcessTree {
    pids: BTreeSet<libc::pid_t>,
    groups: BTreeSet<libc::pid_t>,
}

impl ProcessTree {
    fn refresh(&mut self) {
        if let Err(error) = self.snapshot() {
            progress::step(format_args!(
                "could not snapshot the session's process tree: {error:#}; \
                 stopping its known process groups"
            ));
        }
    }

    fn snapshot(&mut self) -> Result<()> {
        // Separate -o flags and empty headers work on Linux and macOS.
        let output = Command::new("ps")
            .args(["-A", "-o", "pid=", "-o", "ppid=", "-o", "pgid="])
            .output()
            .context("could not run ps")?;
        if !output.status.success() {
            bail!(
                "ps exited {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let mut children = HashMap::<_, Vec<_>>::new();
        let mut groups = HashMap::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let mut fields = line.split_whitespace();
            let mut next = || -> Result<libc::pid_t> {
                fields
                    .next()
                    .context("missing ps field")?
                    .parse()
                    .context("invalid ps field")
            };
            let (pid, parent, group) = (next()?, next()?, next()?);
            children.entry(parent).or_default().push(pid);
            groups.insert(pid, group);
        }
        let mut pending: Vec<_> = self.pids.iter().copied().collect();
        let mut seen = BTreeSet::new();
        while let Some(pid) = pending.pop() {
            if !seen.insert(pid) {
                continue;
            }
            self.pids.insert(pid);
            if let Some(&group) = groups.get(&pid)
                && group > 0
            {
                self.groups.insert(group);
            }
            if let Some(children) = children.get(&pid) {
                pending.extend(children);
            }
        }
        Ok(())
    }

    fn signal(&self, signal: libc::c_int) {
        for &group in &self.groups {
            // SAFETY: kill has no memory-safety preconditions; groups are positive.
            unsafe { libc::kill(-group, signal) };
        }
    }
}
