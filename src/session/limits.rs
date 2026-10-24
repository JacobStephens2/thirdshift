//! Opt-in plan-limit stop; ordinary Runs only record quota evidence.
#[derive(Debug)]
pub(crate) struct Reached;

impl std::fmt::Display for Reached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Scoring run paused at a plan-limit warning; unscored pull requests carry over")
    }
}

impl std::error::Error for Reached {}
