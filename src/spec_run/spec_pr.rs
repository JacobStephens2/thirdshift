//! The Spec PR in a Spec run: resumed as a draft while the Tickets run,
//! opened as a draft once the first Ticket lands, and its Tickets checklist
//! kept up to date between its markers and put back after the Spec review.
//! The Spec run's loop says what happened; this says what that does to the
//! Spec PR.

use anyhow::{Context, Result};

use crate::github::{self, PullRequest};
use crate::issue::IssueUrl;
use crate::progress;

/// The markers around the Tickets checklist in the Spec PR's body, so it can
/// be replaced without touching the text around it.
const CHECKLIST_START: &str = "<!-- thirdshift:tickets -->";
const CHECKLIST_END: &str = "<!-- /thirdshift:tickets -->";

/// A Spec run's Spec PR, from its Spec branch into its Base branch, if it is
/// open.
pub(super) struct SpecPr<'a> {
    spec: &'a IssueUrl,
    branch: String,
    base: &'a str,
    pr: Option<PullRequest>,
}

impl<'a> SpecPr<'a> {
    /// The Spec PR of `spec` from `branch` into `base`: the open one, if
    /// there is one, converted back to a draft while the Tickets run.
    pub(super) fn resume(spec: &'a IssueUrl, branch: &str, base: &'a str) -> Result<Self> {
        let pr = github::pull_request_for(spec, branch)?.filter(|pr| pr.is_open());
        if let Some(pr) = &pr
            && !pr.is_draft
        {
            github::convert_to_draft(spec, branch)?;
        }
        Ok(Self {
            spec,
            branch: branch.to_string(),
            base,
            pr,
        })
    }

    /// Show `checklist` as the Spec PR's Tickets checklist if it is open,
    /// only warning if that fails: the Spec run goes on without it.
    pub(super) fn show(&self, checklist: &str) {
        let Some(pr) = &self.pr else {
            return;
        };
        if let Err(error) = self.write(pr, checklist) {
            progress::step(format_args!(
                "could not update the Spec PR's Tickets checklist: {error:#}"
            ));
        }
    }

    /// A Ticket landed, and the Tickets checklist is now `checklist`: open
    /// the Spec PR as a draft with it if it is not open, else show it.
    /// Returns the Spec PR's URL.
    pub(super) fn landed(&mut self, checklist: &str) -> Result<&str> {
        if self.pr.is_some() {
            self.show(checklist);
        } else {
            self.pr = Some(self.open(checklist)?);
        }
        Ok(&self.pr.as_ref().expect("the Spec PR is open").url)
    }

    /// Put `checklist` back in the Spec PR's body, as the Spec review may
    /// have rewritten it without the checklist.
    pub(super) fn put_back(&self, checklist: &str) -> Result<()> {
        let pr = self.pr.as_ref().context("the Spec PR is not open")?;
        self.write(pr, checklist)
    }

    /// The Spec PR's URL, if it is open.
    pub(super) fn url(&self) -> Option<&str> {
        self.pr.as_ref().map(|pr| pr.url.as_str())
    }

    /// Open the Spec PR as a draft, titled from the Spec, closing it, with
    /// `checklist` as its Tickets checklist.
    fn open(&self, checklist: &str) -> Result<PullRequest> {
        let (spec, branch, base) = (self.spec, &self.branch, self.base);
        progress::step(format_args!("opening the Spec PR into {base} as a draft"));
        let title = github::issue_title(spec)?;
        let body = format!(
            "The work on Spec #{number}, gathered from its Tickets on {branch}.\n\n\
             Closes #{number}\n\n\
             {checklist}",
            number = spec.number,
            checklist = between_markers(checklist),
        );
        github::create_draft_pr(spec, branch, base, &title, &body)?;
        github::pull_request_for(spec, branch)?.context("the Spec PR just opened is not found")
    }

    /// Put `checklist` in the body of the Spec PR `pr`, in place of its
    /// Tickets checklist, leaving the rest of the body as it is.
    fn write(&self, pr: &PullRequest, checklist: &str) -> Result<()> {
        let body = github::pr_body(self.spec, pr.number)?;
        let updated = with_checklist(&body, &between_markers(checklist));
        if updated != body {
            progress::step("updating the Spec PR's Tickets checklist");
            github::set_pr_body(self.spec, pr.number, &updated)?;
        }
        Ok(())
    }
}

/// `checklist` wrapped in the Tickets checklist markers.
fn between_markers(checklist: &str) -> String {
    format!("{CHECKLIST_START}\n{checklist}{CHECKLIST_END}\n")
}

/// `body` with its Tickets checklist, the text from its first start marker
/// to the next end marker, replaced by `checklist`, or with `checklist`
/// appended if it has no such pair of markers.
fn with_checklist(body: &str, checklist: &str) -> String {
    if let Some(start) = body.find(CHECKLIST_START)
        && let Some(end) = body[start..].find(CHECKLIST_END)
    {
        let end = start + end + CHECKLIST_END.len();
        let end = if body[end..].starts_with('\n') {
            end + 1
        } else {
            end
        };
        return format!("{}{checklist}{}", &body[..start], &body[end..]);
    }
    if body.is_empty() {
        return checklist.to_string();
    }
    format!("{}\n\n{checklist}", body.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "<!-- thirdshift:tickets -->\nnew\n<!-- /thirdshift:tickets -->\n";

    #[test]
    fn the_checklist_is_wrapped_in_the_markers() {
        assert_eq!(between_markers("new\n"), LIST);
    }

    #[test]
    fn the_checklist_replaces_the_one_between_the_markers_and_leaves_the_rest() {
        let body = "Intro.\n\n<!-- thirdshift:tickets -->\nold\n<!-- /thirdshift:tickets -->\n\nCloses #20\n";

        assert_eq!(
            with_checklist(body, LIST),
            format!("Intro.\n\n{LIST}\nCloses #20\n")
        );
    }

    #[test]
    fn the_checklist_is_appended_when_the_markers_are_gone() {
        assert_eq!(
            with_checklist("A rewritten body.\n\nCloses #20\n", LIST),
            format!("A rewritten body.\n\nCloses #20\n\n{LIST}")
        );
        assert_eq!(with_checklist("", LIST), LIST);
    }

    #[test]
    fn the_checklist_is_appended_when_only_one_marker_is_left() {
        let body = "Text\n<!-- /thirdshift:tickets -->\n<!-- thirdshift:tickets -->\nmore";

        assert_eq!(with_checklist(body, LIST), format!("{body}\n\n{LIST}"));
    }
}
