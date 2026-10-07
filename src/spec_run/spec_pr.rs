//! The Spec PR in a Spec run: resumed as a draft while the Tickets run,
//! opened as a draft once the first Ticket lands, and its Tickets checklist
//! kept up to date between its markers and put back after the Spec review.
//! The Spec run's loop says what happened; this says what that does to the
//! Spec PR.
//!
//! Its rules reach GitHub and the progress lines only through [`Outside`]:
//! [`OnGitHub`] does that for real; `InMemory`, in tests, holds the pull
//! request and records each call.

use anyhow::{Context, Result};

use crate::github::{GitHub, PullRequest};
use crate::issue::IssueUrl;
use crate::progress;

/// The markers around the Tickets checklist in the Spec PR's body, so it can
/// be replaced without touching the text around it.
const CHECKLIST_START: &str = "<!-- thirdshift:tickets -->";
const CHECKLIST_END: &str = "<!-- /thirdshift:tickets -->";

/// A Spec run's Spec PR, from its Spec branch into its Base branch, if it is
/// open, reached through `O`.
pub(super) struct SpecPr<'a, O = OnGitHub<'a>> {
    spec: &'a IssueUrl,
    /// Owned: the worktree it is borrowed from moves into the Delivery,
    /// while the Spec PR is still needed after it.
    branch: String,
    base: &'a str,
    pr: Option<PullRequest>,
    outside: O,
}

impl<'a> SpecPr<'a> {
    /// The Spec PR of `spec` from `branch` into `base`, on GitHub: the open
    /// one, if there is one, converted back to a draft while the Tickets run.
    pub(super) fn resume(spec: &'a IssueUrl, branch: &str, base: &'a str) -> Result<Self> {
        let on_github = OnGitHub {
            spec,
            branch: branch.to_string(),
            base,
            github: GitHub::new(),
        };
        let mut resumed = Self::resume_through(on_github, spec, branch, base)?;
        // Resume is ordinary; subsequent writes account for child endings,
        // even when interruption prevents more implementation work.
        resumed.outside.github = resumed.outside.github.completion();
        Ok(resumed)
    }
}

impl<'a, O: Outside> SpecPr<'a, O> {
    /// [`SpecPr::resume`], reaching the Spec PR through `outside`.
    fn resume_through(
        mut outside: O,
        spec: &'a IssueUrl,
        branch: &str,
        base: &'a str,
    ) -> Result<Self> {
        let pr = outside.pull_request()?.filter(|pr| pr.is_open());
        if let Some(pr) = &pr
            && !pr.is_draft
        {
            outside.convert_to_draft()?;
        }
        Ok(Self {
            spec,
            branch: branch.to_string(),
            base,
            pr,
            outside,
        })
    }

    /// Show `checklist` as the Spec PR's Tickets checklist if it is open,
    /// only warning if that fails: the Spec run goes on without it.
    pub(super) fn show(&mut self, checklist: &str) {
        let Some(number) = self.number() else {
            return;
        };
        if let Err(error) = self.write(number, checklist) {
            self.outside.step(format!(
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
    pub(super) fn put_back(&mut self, checklist: &str) -> Result<()> {
        let number = self.number().context("the Spec PR is not open")?;
        self.write(number, checklist)
    }

    /// The Spec PR's URL, if it is open.
    pub(super) fn url(&self) -> Option<&str> {
        self.pr.as_ref().map(|pr| pr.url.as_str())
    }

    /// The Spec PR's number, if it is open.
    fn number(&self) -> Option<u64> {
        self.pr.as_ref().map(|pr| pr.number)
    }

    /// Open the Spec PR as a draft, titled from the Spec, closing it, with
    /// `checklist` as its Tickets checklist.
    fn open(&mut self, checklist: &str) -> Result<PullRequest> {
        let (spec, branch, base) = (self.spec, &self.branch, self.base);
        self.outside
            .step(format!("opening the Spec PR into {base} as a draft"));
        let title = self.outside.spec_title()?;
        let body = format!(
            "The work on Spec #{number}, gathered from its Tickets on {branch}.\n\n\
             Closes #{number}\n\n\
             {checklist}",
            number = spec.number,
            checklist = between_markers(checklist),
        );
        self.outside.create_draft(&title, &body)?;
        self.outside
            .pull_request()?
            .context("the Spec PR just opened is not found")
    }

    /// Put `checklist` in the body of the Spec PR `number`, in place of its
    /// Tickets checklist, leaving the rest of the body as it is.
    fn write(&mut self, number: u64, checklist: &str) -> Result<()> {
        let body = self.outside.pr_body(number)?;
        let updated = with_checklist(&body, checklist);
        if updated != body {
            self.outside
                .step("updating the Spec PR's Tickets checklist".to_string());
            self.outside.set_pr_body(number, &updated)?;
        }
        Ok(())
    }
}

/// What the Spec PR's rules do or read outside the Spec PR: the pull request
/// from the Spec branch, the Spec's title, and the progress lines. Visible
/// to the Spec run only because [`SpecPr`]'s type parameter names it; the
/// Spec run never calls it.
pub(super) trait Outside {
    /// The pull request from the Spec branch, if it has one.
    fn pull_request(&mut self) -> Result<Option<PullRequest>>;
    /// Convert the pull request from the Spec branch back to a draft.
    fn convert_to_draft(&mut self) -> Result<()>;
    /// The Spec's title.
    fn spec_title(&mut self) -> Result<String>;
    /// Open a draft pull request from the Spec branch into the Base branch,
    /// titled `title`, with `body`.
    fn create_draft(&mut self, title: &str, body: &str) -> Result<()>;
    /// The body of pull request `number`.
    fn pr_body(&mut self, number: u64) -> Result<String>;
    /// Set the body of pull request `number` to `body`.
    fn set_pr_body(&mut self, number: u64, body: &str) -> Result<()>;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
}

/// The Spec PR of `spec`, from `branch` into `base`, on GitHub, and progress
/// lines printed on stderr.
pub(super) struct OnGitHub<'a> {
    github: GitHub,
    spec: &'a IssueUrl,
    branch: String,
    base: &'a str,
}

impl Outside for OnGitHub<'_> {
    fn pull_request(&mut self) -> Result<Option<PullRequest>> {
        self.github.pull_request_for(self.spec, &self.branch)
    }

    fn convert_to_draft(&mut self) -> Result<()> {
        self.github.convert_to_draft(self.spec, &self.branch)
    }

    fn spec_title(&mut self) -> Result<String> {
        self.github.issue_title(self.spec)
    }

    fn create_draft(&mut self, title: &str, body: &str) -> Result<()> {
        self.github
            .create_draft_pr(self.spec, &self.branch, self.base, title, body)
            .map(|_| ())
    }

    fn pr_body(&mut self, number: u64) -> Result<String> {
        self.github.pr_body(self.spec, number)
    }

    fn set_pr_body(&mut self, number: u64, body: &str) -> Result<()> {
        self.github.set_pr_body(self.spec, number, body)
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }
}

/// `checklist` wrapped in the Tickets checklist markers.
fn between_markers(checklist: &str) -> String {
    format!("{CHECKLIST_START}\n{checklist}{CHECKLIST_END}\n")
}

/// `body` with its Tickets checklist, the text from its first start marker
/// to the next end marker, replaced by `checklist` between the markers, or
/// with that appended if it has no such pair of markers.
fn with_checklist(body: &str, checklist: &str) -> String {
    let checklist = between_markers(checklist);
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
        return checklist;
    }
    format!("{}\n\n{checklist}", body.trim_end())
}

#[cfg(test)]
mod in_memory {
    use anyhow::{Result, bail};

    use super::Outside;
    use crate::github::{PrState, PullRequest};

    /// A call of [`Outside`] that can fail.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Fails {
        PullRequest,
        ConvertToDraft,
        SpecTitle,
        CreateDraft,
        PrBody,
        SetPrBody,
    }

    /// A call the Spec PR made, in the order it made it.
    #[derive(Debug, PartialEq, Eq)]
    pub enum Call {
        /// It read the pull request from the Spec branch.
        PullRequest,
        ConvertToDraft,
        SpecTitle,
        /// It opened a draft pull request with this title and body.
        CreateDraft {
            title: String,
            body: String,
        },
        /// It read the body of this pull request.
        PrBody(u64),
        /// It set the body of this pull request to this.
        SetPrBody(u64, String),
        /// It handed on this progress line.
        Step(String),
    }

    /// The pull request from the Spec branch, as the adapter holds it.
    #[derive(Clone, Copy)]
    pub struct Pr {
        pub number: u64,
        pub draft: bool,
        pub open: bool,
    }

    impl Pr {
        /// An open draft, numbered `number`.
        pub fn draft(number: u64) -> Self {
            Pr {
                number,
                draft: true,
                open: true,
            }
        }
    }

    /// The pull request from the Spec branch held in memory, if there is
    /// one, with its body, and the Spec's title. A draft it opens is
    /// numbered 30.
    pub struct InMemory {
        pub pr: Option<Pr>,
        pub body: String,
        pub title: String,
        /// Whether the pull request it opens is then not found.
        pub opened_is_lost: bool,
        /// The calls that fail, every time.
        pub failing: Vec<Fails>,
        /// Every call made, in order.
        pub calls: Vec<Call>,
    }

    impl Default for InMemory {
        /// No pull request, and every call succeeding.
        fn default() -> Self {
            InMemory {
                pr: None,
                body: String::new(),
                title: "Widgets".to_string(),
                opened_is_lost: false,
                failing: Vec::new(),
                calls: Vec::new(),
            }
        }
    }

    impl InMemory {
        /// Record `call`, then fail if `what` is to fail.
        fn call(&mut self, call: Call, what: Fails) -> Result<()> {
            self.calls.push(call);
            if self.failing.contains(&what) {
                bail!("{what:?} failed");
            }
            Ok(())
        }
    }

    /// The URL of pull request `number`.
    pub fn url(number: u64) -> String {
        format!("https://github.com/acme/widgets/pull/{number}")
    }

    impl Outside for InMemory {
        fn pull_request(&mut self) -> Result<Option<PullRequest>> {
            self.call(Call::PullRequest, Fails::PullRequest)?;
            Ok(self.pr.map(|pr| PullRequest {
                number: pr.number,
                url: url(pr.number),
                state: if pr.open {
                    PrState::Open
                } else {
                    PrState::Closed
                },
                head: "issue-20".to_string(),
                base: "main".to_string(),
                is_draft: pr.draft,
            }))
        }

        fn convert_to_draft(&mut self) -> Result<()> {
            self.call(Call::ConvertToDraft, Fails::ConvertToDraft)?;
            if let Some(pr) = &mut self.pr {
                pr.draft = true;
            }
            Ok(())
        }

        fn spec_title(&mut self) -> Result<String> {
            self.call(Call::SpecTitle, Fails::SpecTitle)?;
            Ok(self.title.clone())
        }

        fn create_draft(&mut self, title: &str, body: &str) -> Result<()> {
            let call = Call::CreateDraft {
                title: title.to_string(),
                body: body.to_string(),
            };
            self.call(call, Fails::CreateDraft)?;
            if !self.opened_is_lost {
                self.pr = Some(Pr::draft(30));
            }
            self.body = body.to_string();
            Ok(())
        }

        fn pr_body(&mut self, number: u64) -> Result<String> {
            self.call(Call::PrBody(number), Fails::PrBody)?;
            Ok(self.body.clone())
        }

        fn set_pr_body(&mut self, number: u64, body: &str) -> Result<()> {
            self.call(Call::SetPrBody(number, body.to_string()), Fails::SetPrBody)?;
            self.body = body.to_string();
            Ok(())
        }

        fn step(&mut self, line: String) {
            self.calls.push(Call::Step(line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::in_memory::{Call, Fails, InMemory, Pr, url};
    use super::*;

    fn spec() -> IssueUrl {
        IssueUrl::parse("https://github.com/acme/widgets/issues/20").unwrap()
    }

    /// The Spec PR of Spec #20 from `issue-20` into `main`, resumed through
    /// `outside`.
    fn resume(spec: &IssueUrl, outside: InMemory) -> Result<SpecPr<'_, InMemory>> {
        SpecPr::resume_through(outside, spec, "issue-20", "main")
    }

    /// The calls made after resuming.
    fn after_resume<'a>(spec_pr: &'a SpecPr<'_, InMemory>) -> &'a [Call] {
        let calls = &spec_pr.outside.calls;
        assert_eq!(calls.first(), Some(&Call::PullRequest), "{calls:?}");
        &calls[1..]
    }

    fn step(line: &str) -> Call {
        Call::Step(line.to_string())
    }

    /// An open PR, numbered 9, holding `body`.
    fn open_with(body: &str) -> InMemory {
        InMemory {
            pr: Some(Pr::draft(9)),
            body: body.to_string(),
            ..InMemory::default()
        }
    }

    const UPDATING: &str = "updating the Spec PR's Tickets checklist";

    const LIST: &str = "<!-- thirdshift:tickets -->\nnew\n<!-- /thirdshift:tickets -->\n";

    #[test]
    fn the_checklist_replaces_the_one_between_the_markers_and_leaves_the_rest() {
        let body = "Intro.\n\n<!-- thirdshift:tickets -->\nold\n<!-- /thirdshift:tickets -->\n\nCloses #20\n";

        assert_eq!(
            with_checklist(body, "new\n"),
            format!("Intro.\n\n{LIST}\nCloses #20\n")
        );
    }

    #[test]
    fn the_checklist_is_appended_when_the_markers_are_gone() {
        assert_eq!(
            with_checklist("A rewritten body.\n\nCloses #20\n", "new\n"),
            format!("A rewritten body.\n\nCloses #20\n\n{LIST}")
        );
        assert_eq!(with_checklist("", "new\n"), LIST);
    }

    #[test]
    fn the_checklist_is_appended_when_only_one_marker_is_left() {
        let body = "Text\n<!-- /thirdshift:tickets -->\n<!-- thirdshift:tickets -->\nmore";

        assert_eq!(with_checklist(body, "new\n"), format!("{body}\n\n{LIST}"));
    }

    #[test]
    fn resuming_with_a_ready_open_pr_converts_it_to_a_draft() {
        let spec = spec();
        let outside = InMemory {
            pr: Some(Pr {
                draft: false,
                ..Pr::draft(9)
            }),
            ..InMemory::default()
        };

        let spec_pr = resume(&spec, outside).unwrap();

        assert_eq!(after_resume(&spec_pr), [Call::ConvertToDraft]);
        assert_eq!(spec_pr.url(), Some(url(9).as_str()));
    }

    #[test]
    fn resuming_with_a_draft_a_closed_pr_or_none_converts_nothing() {
        let closed = Pr {
            draft: false,
            open: false,
            ..Pr::draft(9)
        };
        for (pr, url) in [
            (Some(Pr::draft(9)), Some(url(9))),
            (Some(closed), None),
            (None, None),
        ] {
            let spec = spec();
            let outside = InMemory {
                pr,
                ..InMemory::default()
            };

            let spec_pr = resume(&spec, outside).unwrap();

            assert_eq!(after_resume(&spec_pr), []);
            assert_eq!(spec_pr.url(), url.as_deref());
        }
    }

    #[test]
    fn landing_with_no_pr_opens_one_as_a_draft_titled_from_the_spec_and_closing_it() {
        let spec = spec();
        let mut spec_pr = resume(&spec, InMemory::default()).unwrap();

        let landed = spec_pr.landed("- [x] #21 landed\n").unwrap().to_string();

        assert_eq!(landed, url(30));
        assert_eq!(
            after_resume(&spec_pr),
            [
                step("opening the Spec PR into main as a draft"),
                Call::SpecTitle,
                Call::CreateDraft {
                    title: "Widgets".to_string(),
                    body: "The work on Spec #20, gathered from its Tickets on issue-20.\n\n\
                           Closes #20\n\n\
                           <!-- thirdshift:tickets -->\n- [x] #21 landed\n\
                           <!-- /thirdshift:tickets -->\n"
                        .to_string(),
                },
                Call::PullRequest,
            ]
        );
        assert_eq!(spec_pr.url(), Some(url(30).as_str()));
    }

    #[test]
    fn landing_when_the_pr_just_opened_is_not_found_fails() {
        let spec = spec();
        let outside = InMemory {
            opened_is_lost: true,
            ..InMemory::default()
        };
        let mut spec_pr = resume(&spec, outside).unwrap();

        let error = spec_pr.landed("new\n").unwrap_err();

        assert_eq!(format!("{error:#}"), "the Spec PR just opened is not found");
        assert_eq!(spec_pr.url(), None);
    }

    #[test]
    fn landing_with_a_pr_open_writes_the_checklist_and_creates_nothing() {
        let spec = spec();
        let mut spec_pr = resume(&spec, open_with("Closes #20\n")).unwrap();

        let landed = spec_pr.landed("new\n").unwrap().to_string();

        assert_eq!(landed, url(9));
        assert_eq!(
            after_resume(&spec_pr),
            [
                Call::PrBody(9),
                step(UPDATING),
                Call::SetPrBody(9, format!("Closes #20\n\n{LIST}")),
            ]
        );
    }

    #[test]
    fn showing_with_no_pr_makes_no_call() {
        let spec = spec();
        let mut spec_pr = resume(&spec, InMemory::default()).unwrap();

        spec_pr.show("new\n");

        assert_eq!(after_resume(&spec_pr), []);
    }

    #[test]
    fn showing_an_unchanged_checklist_reads_the_body_but_does_not_set_it() {
        let spec = spec();
        let mut spec_pr = resume(&spec, open_with(&format!("Closes #20\n\n{LIST}"))).unwrap();

        spec_pr.show("new\n");

        assert_eq!(after_resume(&spec_pr), [Call::PrBody(9)]);
    }

    #[test]
    fn showing_a_checklist_that_cannot_be_set_is_only_a_progress_line() {
        let spec = spec();
        let outside = InMemory {
            failing: vec![Fails::SetPrBody],
            ..open_with("Closes #20\n")
        };
        let mut spec_pr = resume(&spec, outside).unwrap();

        spec_pr.show("new\n");

        assert_eq!(
            after_resume(&spec_pr).last(),
            Some(&step(
                "could not update the Spec PR's Tickets checklist: SetPrBody failed"
            ))
        );
    }

    #[test]
    fn putting_back_with_no_pr_fails() {
        let spec = spec();
        let mut spec_pr = resume(&spec, InMemory::default()).unwrap();

        let error = spec_pr.put_back("new\n").unwrap_err();

        assert_eq!(format!("{error:#}"), "the Spec PR is not open");
        assert_eq!(after_resume(&spec_pr), []);
    }

    #[test]
    fn putting_back_writes_the_checklist() {
        let spec = spec();
        let mut spec_pr = resume(&spec, open_with("Rewritten. Closes #20\n")).unwrap();

        spec_pr.put_back("new\n").unwrap();

        assert_eq!(
            spec_pr.outside.body,
            format!("Rewritten. Closes #20\n\n{LIST}")
        );
    }

    #[test]
    fn putting_back_a_checklist_that_cannot_be_set_fails() {
        let spec = spec();
        let outside = InMemory {
            failing: vec![Fails::SetPrBody],
            ..open_with("Rewritten. Closes #20\n")
        };
        let mut spec_pr = resume(&spec, outside).unwrap();

        let error = spec_pr.put_back("new\n").unwrap_err();

        assert_eq!(format!("{error:#}"), "SetPrBody failed");
    }
}
