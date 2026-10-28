//! The conclusion's GitHub effects, shared by an Architect run and a Process
//! Step. Pass gates and dispatch are outside this interface.

use anyhow::Result;

use crate::github::{GitHub, Issue};
use crate::issue::IssueUrl;
use crate::labels::Edit;

pub(crate) trait Outside {
    fn issue(&mut self, issue: &IssueUrl) -> Result<Issue>;
    fn apply(&mut self, edit: &Edit) -> Result<()>;
    fn interrupted(&mut self) -> bool;
    fn step(&mut self, line: String);
}

impl<T: crate::pass::Outside> Outside for T {
    fn issue(&mut self, issue: &IssueUrl) -> Result<Issue> {
        crate::pass::Outside::issue(self, issue)
    }

    fn apply(&mut self, edit: &Edit) -> Result<()> {
        crate::pass::Outside::apply(self, edit)
    }

    fn interrupted(&mut self) -> bool {
        crate::pass::Outside::interrupted(self)
    }

    fn step(&mut self, line: String) {
        crate::pass::Outside::step(self, line);
    }
}

impl Outside for GitHub {
    fn issue(&mut self, issue: &IssueUrl) -> Result<Issue> {
        GitHub::issue(self, issue)
    }

    fn apply(&mut self, edit: &Edit) -> Result<()> {
        edit.apply(self)
    }

    fn interrupted(&mut self) -> bool {
        crate::interrupt::requested()
    }

    fn step(&mut self, line: String) {
        crate::progress::step(line);
    }
}
