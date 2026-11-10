//! Operation-local completion facts for the fixed panel's expected calls.
//! Projections never reread storage; only successful publication advances them.

use super::super::trial_results::files;
use super::*;

pub(super) struct History<'a> {
    root: &'a Path,
    seats: usize,
    pairs: Vec<Pair<'a>>,
}

struct Pair<'a> {
    task: &'a Task,
    trial: usize,
    key: String,
    calls: Vec<[Option<Call>; 2]>,
    executable: bool,
}

/// A missing expected call. Only the history can construct its identity.
pub(super) struct Work<'a> {
    root: &'a Path,
    pair: usize,
    pub task: &'a Task,
    pub trial: usize,
    pub seat: usize,
    pub order: usize,
}

impl Work<'_> {
    pub fn key(&self) -> String {
        trial_results::pair_key(&self.task.suite, &self.task.name, self.trial)
    }

    pub fn identities(&self) -> [trial_results::Identity<'_>; 2] {
        identities(self.task, self.trial)
    }
}

fn identities(task: &Task, trial: usize) -> [trial_results::Identity<'_>; 2] {
    [Arm::Baseline, Arm::Candidate].map(|arm| trial_results::Identity {
        suite: &task.suite,
        task: &task.name,
        arm,
        trial,
    })
}

fn call_path(root: &Path, pair: &str, seat: usize, order: usize) -> std::path::PathBuf {
    root.join("panel")
        .join(pair)
        .join(format!("seat-{}-order-{order}.json", seat + 1))
}

impl<'a> History<'a> {
    pub fn capture(
        files: &impl Files,
        root: &'a Path,
        description: &Description,
        tasks: &'a [Task],
    ) -> Result<Self> {
        let mut history = Self {
            root,
            seats: 0,
            pairs: Vec::new(),
        };
        let Some(panel) = &description.bar.panel else {
            return Ok(history);
        };
        history.seats = panel.seats.len();
        for task in tasks {
            if task.manifest.session != format::Session::Process {
                continue;
            }
            for trial in 0..description.bar.trials_per_task[&task.suite] {
                let key = trial_results::pair_key(&task.suite, &task.name, trial);
                let mut calls = Vec::new();
                for seat in 0..history.seats {
                    let mut orders = [None, None];
                    for (index, call) in orders.iter_mut().enumerate() {
                        let order = index + 1;
                        let path = call_path(root, &key, seat, order);
                        *call = files::metadata(files, &path).with_context(|| format!(
                            "can't read Grader panel pair {key} ({}/{} #{}), seat {}, order {order}",
                            task.suite, task.name, trial + 1, seat + 1
                        ))?;
                    }
                    calls.push(orders);
                }
                history.pairs.push(Pair {
                    task,
                    trial,
                    key,
                    calls,
                    executable: identities(task, trial)
                        .iter()
                        .all(|identity| trial_results::finished(root, identity)),
                });
            }
        }
        Ok(history)
    }

    /// All planned missing sessions and affected pairs, once per seat.
    pub fn remaining(&self) -> Vec<(usize, usize)> {
        let mut counts = vec![(0, 0); self.seats];
        for pair in &self.pairs {
            for (orders, (sessions, pairs)) in pair.calls.iter().zip(&mut counts) {
                let missing = orders.iter().filter(|call| call.is_none()).count();
                *sessions += missing;
                *pairs += usize::from(missing > 0);
            }
        }
        counts
    }

    /// Missing calls whose two Trials are terminal, in fixed execution order.
    pub fn executable(&self) -> Vec<Work<'a>> {
        let mut work = Vec::new();
        for (pair_index, pair) in self.pairs.iter().enumerate() {
            if !pair.executable {
                continue;
            }
            for (seat, orders) in pair.calls.iter().enumerate() {
                for (index, call) in orders.iter().enumerate() {
                    if call.is_none() {
                        work.push(Work {
                            root: self.root,
                            pair: pair_index,
                            task: pair.task,
                            trial: pair.trial,
                            seat,
                            order: index + 1,
                        });
                    }
                }
            }
        }
        work
    }

    /// Report inputs require both orders of every seat, independent of Trials.
    pub fn complete(&self) -> impl Iterator<Item = (&'a Task, usize, Vec<Vec<Call>>)> + '_ {
        self.pairs.iter().filter_map(|pair| {
            let calls: Option<Vec<Vec<Call>>> = pair
                .calls
                .iter()
                .map(|orders| orders.iter().cloned().collect())
                .collect();
            calls.map(|calls| (pair.task, pair.trial, calls))
        })
    }

    /// The existing Experiment writer lock owns publication. Save before
    /// advancing completion, so a failed write remains pending for this capture.
    pub fn publish(&mut self, work: &Work<'_>, call: Call) -> Result<()> {
        ensure!(
            self.root == work.root,
            "panel call does not belong to this history"
        );
        let pair = self
            .pairs
            .get_mut(work.pair)
            .context("unknown panel pair")?;
        ensure!(
            std::ptr::eq(pair.task, work.task) && pair.trial == work.trial,
            "panel call does not belong to this history"
        );
        let saved = pair
            .calls
            .get_mut(work.seat)
            .and_then(|orders| {
                work.order
                    .checked_sub(1)
                    .and_then(|index| orders.get_mut(index))
            })
            .context("unknown panel seat or order")?;
        ensure!(saved.is_none(), "Grader panel call is already complete");
        let path = call_path(self.root, &pair.key, work.seat, work.order);
        fs::create_dir_all(path.parent().context("panel call path")?)?;
        store::write(&path, &call)?;
        *saved = Some(call);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
