//! One action's actor and origin, retaining the existing Stop-request format.
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Actor {
    who: String,
    #[serde(default)]
    from_console: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    console_holder: Option<String>,
}

impl Actor {
    pub(crate) const START_ENV: &str = "THIRDSHIFT_STARTED_BY";

    pub(super) fn starting() -> Option<Self> {
        std::env::var(Self::START_ENV)
            .ok()
            .and_then(|value| serde_json::from_str(&value).ok())
    }

    pub fn cli(action: &str) -> anyhow::Result<Self> {
        Ok(Self {
            who: super::unix_user(action)?,
            from_console: false,
            console_holder: None,
        })
    }

    pub fn console(who: String, holder: Option<String>) -> Self {
        Self {
            who,
            from_console: true,
            console_holder: holder,
        }
    }

    pub fn who(&self) -> &str {
        &self.who
    }

    pub fn is_console(&self) -> bool {
        self.from_console
    }
}

impl fmt::Display for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.from_console, &self.console_holder) {
            (true, Some(holder)) => write!(f, "by {} from the Console on {holder}", self.who),
            (true, None) => write!(f, "from the Console by {}", self.who),
            (false, _) => write!(f, "from the CLI by {}", self.who),
        }
    }
}
