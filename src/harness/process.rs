//! Prepare Harness environment and stop behavior for the shared execution owner.

use std::process::{ChildStdout, Command, Output};

use anyhow::Result;

use super::Adapter;
use crate::process::{self, Control, Interruption, Streamed};

pub(super) fn output(
    adapter: &dyn Adapter,
    command: &mut Command,
    input: Option<&str>,
) -> Result<Output> {
    process::output(
        command.envs(adapter.environment().iter().copied()),
        input.map(str::as_bytes),
        Control {
            name: adapter.name(),
            interruption: Interruption::Ordinary,
            stop: &|child| adapter.stop(child),
        },
    )
}

pub(crate) fn streaming<T: Send + 'static>(
    adapter: &dyn Adapter,
    command: &mut Command,
    input: Option<&str>,
    state: T,
    consume: impl FnOnce(ChildStdout, &mut T) -> Result<()> + Send + 'static,
) -> Result<Streamed<T>> {
    process::streaming(
        command.envs(adapter.environment().iter().copied()),
        input.map(str::as_bytes),
        Control {
            name: adapter.name(),
            interruption: Interruption::Ordinary,
            stop: &|child| adapter.stop(child),
        },
        state,
        consume,
    )
}
