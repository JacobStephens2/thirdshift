//! Prepare Harness environment and stop behavior for the shared execution owner.

use std::process::{ChildStdout, Output};
use std::time::Instant;

use anyhow::Result;

use super::Adapter;
use crate::process::{self, Control, Interruption, Invocation, Streamed};

pub(super) fn output(
    adapter: &dyn Adapter,
    invocation: &mut dyn Invocation,
    input: Option<&str>,
) -> Result<Output> {
    invocation
        .command()
        .envs(adapter.environment().iter().copied());
    process::output(
        invocation,
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
    invocation: &mut dyn Invocation,
    input: Option<&str>,
    deadline: Option<Instant>,
    state: T,
    consume: impl FnOnce(ChildStdout, &mut T) -> Result<()> + Send + 'static,
) -> Result<Streamed<T>> {
    invocation
        .command()
        .envs(adapter.environment().iter().copied());
    process::streaming(
        invocation,
        input.map(str::as_bytes),
        Control {
            name: adapter.name(),
            interruption: Interruption::Ordinary,
            stop: &|child| adapter.stop(child),
        },
        deadline,
        state,
        consume,
    )
}
