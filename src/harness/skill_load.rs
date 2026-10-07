//! Warning policy for Harnesses whose models load Factory skills with a tool.
#[derive(Default)]
pub struct SkillLoad {
    expected: Option<String>,
    loaded: bool,
}

impl SkillLoad {
    pub fn for_prompt(prompt: &str) -> Self {
        Self {
            expected: prompt
                .strip_prefix('/')
                .and_then(|rest| rest.split_whitespace().next())
                .filter(|name| name.starts_with("thirdshift-"))
                .map(String::from),
            loaded: false,
        }
    }

    /// Adapters decode their own event format and report successful loads only.
    pub fn observed(&mut self, name: &str) {
        if self.expected.as_deref() == Some(name) {
            self.loaded = true;
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        self.expected
            .as_ref()
            .filter(|_| !self.loaded)
            .map(|skill| {
                vec![format!(
                    "warning: the session never loaded {skill} with its skill tool"
                )]
            })
            .unwrap_or_default()
    }
}
