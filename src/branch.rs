//! Issue branch selection: start a fresh Issue branch, or continue the one
//! already on origin (ADR-0002).

use anyhow::{Result, bail};

use crate::git::Git;
use crate::github::{self, PrState, PullRequest};
use crate::issue::IssueUrl;
use crate::progress;

pub enum Selection {
    /// No Issue branch has been used yet, or the highest-numbered one's PR is
    /// merged or closed: start `branch` from the Base branch.
    Fresh { branch: String },
    /// `branch` is on origin, with no PR or the open `pr`.
    Continuation {
        branch: String,
        pr: Option<PullRequest>,
    },
}

impl Selection {
    pub fn branch(&self) -> &str {
        match self {
            Selection::Fresh { branch } | Selection::Continuation { branch, .. } => branch,
        }
    }

    /// The open PR's base in a Continuation that has one, which names the
    /// Base branch whatever else would. `given` is the Base branch the Run was
    /// given by what started it, if any, and `checked_out` the branch
    /// checked out in the Launch directory (`None` on a detached HEAD). Says
    /// so on stderr when the PR's base replaces a different given or
    /// checked-out branch.
    pub fn pr_base(&self, given: Option<&str>, checked_out: Option<&str>) -> Option<&str> {
        self.pr_base_to(given, checked_out, progress::step)
    }

    /// [`Selection::pr_base`], with the line saying the PR's base replaces
    /// another branch going to `replaced_line`.
    fn pr_base_to(
        &self,
        given: Option<&str>,
        checked_out: Option<&str>,
        mut replaced_line: impl FnMut(String),
    ) -> Option<&str> {
        let Selection::Continuation {
            branch,
            pr: Some(pr),
        } = self
        else {
            return None;
        };
        if let Some(replaced) = given
            .or(checked_out)
            .filter(|&replaced| replaced != pr.base)
        {
            let which = if given.is_some() {
                "given"
            } else {
                "checked-out"
            };
            replaced_line(format!(
                "continuing {branch} and its PR {}, so the Base branch is {}, not the {which} {replaced}",
                pr.url, pr.base
            ));
        }
        Some(&pr.base)
    }
}

/// Pick the Issue branch for `issue` from the Issue branches on origin and
/// their PRs in any state: the highest number seen on either decides. Fails
/// if a local copy of the chosen branch in the Launch directory differs from
/// origin's: the Run replaces it and deletes it at cleanup.
pub fn select(launch: &Git, issue: &IssueUrl) -> Result<Selection> {
    select_through(&GitHubAndOrigin { launch }, issue)
}

/// What shows an issue was started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Started {
    /// This Issue branch for it is on origin.
    Branch(String),
    /// A pull request from an Issue branch for it exists, with this URL.
    PullRequest(String),
}

/// What shows `issue` was ever started, if it was: the first Issue branch for
/// it on origin, or else the newest pull request from one, open, merged or
/// closed.
pub fn started(launch: &Git, issue: &IssueUrl) -> Result<Option<Started>> {
    started_through(&GitHubAndOrigin { launch }, issue)
}

/// What selection reads of origin, GitHub and the Launch directory. Each
/// listing is by prefix, as git and GitHub answer it, so it can hold another
/// issue's Issue branches.
trait Reads {
    /// The branches on origin named `first_branch` or
    /// `first_branch-branch-*`, each with its head.
    fn on_origin(&self, first_branch: &str) -> Result<Vec<OnOrigin>>;
    /// The pull requests, in any state, whose head starts with
    /// `first_branch`, from `issue`'s repository only.
    fn pull_requests(&self, issue: &IssueUrl, first_branch: &str) -> Result<Vec<PullRequest>>;
    /// The head of the local `branch` in the Launch directory, if it has one.
    fn local_head(&self, branch: &str) -> Option<String>;
}

/// A branch on origin.
struct OnOrigin {
    name: String,
    head: String,
}

/// The reads of GitHub, of the Launch directory `launch` and of its origin.
struct GitHubAndOrigin<'a> {
    launch: &'a Git,
}

impl Reads for GitHubAndOrigin<'_> {
    /// One `git ls-remote`.
    fn on_origin(&self, first_branch: &str) -> Result<Vec<OnOrigin>> {
        let remote = self.launch.run(&[
            "ls-remote",
            "--heads",
            "origin",
            &format!("refs/heads/{first_branch}"),
            &format!("refs/heads/{first_branch}-branch-*"),
        ])?;
        Ok(remote
            .lines()
            .filter_map(|line| {
                let (head, name) = line.split_once('\t')?;
                Some(OnOrigin {
                    name: name.strip_prefix("refs/heads/")?.to_string(),
                    head: head.to_string(),
                })
            })
            .collect())
    }

    /// One `gh pr list`.
    fn pull_requests(&self, issue: &IssueUrl, first_branch: &str) -> Result<Vec<PullRequest>> {
        github::pull_requests_with_head_prefix(issue, first_branch)
    }

    fn local_head(&self, branch: &str) -> Option<String> {
        self.launch
            .run(&[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .ok()
    }
}

/// [`select`], reading through `reads`.
fn select_through(reads: &impl Reads, issue: &IssueUrl) -> Result<Selection> {
    let first_branch = branch_name(issue, 1);
    let Used { on_origin, prs } = used(reads, issue)?;

    let highest = on_origin
        .iter()
        .map(|(number, _)| *number)
        .chain(prs.iter().map(|(number, _)| *number))
        .max();
    let Some(highest) = highest else {
        check_local_branch(reads, &first_branch, None)?;
        return Ok(Selection::Fresh {
            branch: first_branch,
        });
    };
    let branch = branch_name(issue, highest);
    // A branch can have had several PRs; the newest one decides.
    let pr = prs
        .into_iter()
        .filter_map(|(number, pr)| (number == highest).then_some(pr))
        .max_by_key(|pr| pr.number);
    let origin_sha = on_origin
        .into_iter()
        .find_map(|(number, sha)| (number == highest).then_some(sha));
    match (origin_sha, pr) {
        // A merged or closed PR uses its number up, even if its branch was deleted.
        (_, Some(pr)) if pr.state != PrState::Open => {
            let next = branch_name(issue, highest + 1);
            check_local_branch(reads, &next, None)?;
            Ok(Selection::Fresh { branch: next })
        }
        (Some(origin_sha), pr) => {
            check_local_branch(reads, &branch, Some(&origin_sha))?;
            Ok(Selection::Continuation { branch, pr })
        }
        (None, _) => bail!("{branch} has an open PR but is gone from origin"),
    }
}

/// [`started`], reading through `reads`.
fn started_through(reads: &impl Reads, issue: &IssueUrl) -> Result<Option<Started>> {
    let Used { on_origin, prs } = used(reads, issue)?;
    let branch = on_origin.iter().map(|(number, _)| *number).min();
    let pr = prs.into_iter().map(|(_, pr)| pr).max_by_key(|pr| pr.number);
    Ok(branch
        .map(|number| Started::Branch(branch_name(issue, number)))
        .or(pr.map(|pr| Started::PullRequest(pr.url))))
}

/// What has been used of an issue's Issue branches.
struct Used {
    /// Those on origin, each by its number, with its head.
    on_origin: Vec<(u64, String)>,
    /// The pull requests from them, in any state, each with its Issue
    /// branch's number.
    prs: Vec<(u64, PullRequest)>,
}

/// What has been used of `issue`'s Issue branches: those on origin and the
/// pull requests from them, one listing of each, with another issue's that
/// the listings' prefix lets through dropped.
fn used(reads: &impl Reads, issue: &IssueUrl) -> Result<Used> {
    let first_branch = branch_name(issue, 1);
    let on_origin = reads
        .on_origin(&first_branch)?
        .into_iter()
        .filter_map(|branch| Some((branch_number(issue, &branch.name)?, branch.head)))
        .collect();
    let prs = reads
        .pull_requests(issue, &first_branch)?
        .into_iter()
        .filter_map(|pr| Some((branch_number(issue, &pr.head)?, pr)))
        .collect();
    Ok(Used { on_origin, prs })
}

/// A local `branch` in the launch repository must be at `origin_sha`, or not
/// exist when origin has no copy, so no local-only commits are destroyed.
fn check_local_branch(reads: &impl Reads, branch: &str, origin_sha: Option<&str>) -> Result<()> {
    let Some(local_sha) = reads.local_head(branch) else {
        return Ok(());
    };
    match origin_sha {
        Some(origin_sha) if origin_sha == local_sha => Ok(()),
        Some(_) => bail!(
            "the local branch {branch} differs from origin/{branch}; push, reset or delete it first"
        ),
        None => {
            bail!("the local branch {branch} is not on origin; push, rename or delete it first")
        }
    }
}

/// `issue-<n>` is branch 1, `issue-<n>-branch-<k>` is branch k for k ≥ 2.
fn branch_number(issue: &IssueUrl, branch: &str) -> Option<u64> {
    let rest = branch.strip_prefix(&branch_name(issue, 1))?;
    if rest.is_empty() {
        return Some(1);
    }
    let k: u64 = rest.strip_prefix("-branch-")?.parse().ok()?;
    (k >= 2 && branch == branch_name(issue, k)).then_some(k)
}

fn branch_name(issue: &IssueUrl, number: u64) -> String {
    match number {
        1 => format!("issue-{}", issue.number),
        k => format!("issue-{}-branch-{k}", issue.number),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// A read selection made, in the order it made it.
    #[derive(Debug, PartialEq, Eq)]
    enum Read {
        /// It listed the branches on origin for this first Issue branch.
        OnOrigin(String),
        /// It listed the pull requests whose head starts with this.
        PullRequests(String),
        /// It read the head of this local branch.
        LocalHead(String),
    }

    /// A pull request as the in-memory reads hold it.
    struct Pr {
        number: u64,
        head: &'static str,
        base: &'static str,
        state: PrState,
    }

    /// Origin, GitHub and the Launch directory in memory, each read recorded
    /// in `seen`. Listings match by prefix, as git's and GitHub's do.
    #[derive(Default)]
    struct InMemory {
        /// The branches on origin, each with its head.
        origin: Vec<(&'static str, &'static str)>,
        /// The pull requests, from this repository.
        prs: Vec<Pr>,
        /// The local branches in the Launch directory, each with its head.
        local: Vec<(&'static str, &'static str)>,
        seen: RefCell<Vec<Read>>,
    }

    impl Reads for InMemory {
        fn on_origin(&self, first_branch: &str) -> Result<Vec<OnOrigin>> {
            self.seen
                .borrow_mut()
                .push(Read::OnOrigin(first_branch.to_string()));
            let pattern = format!("{first_branch}-branch-");
            Ok(self
                .origin
                .iter()
                .filter(|(name, _)| *name == first_branch || name.starts_with(&pattern))
                .map(|(name, head)| OnOrigin {
                    name: name.to_string(),
                    head: head.to_string(),
                })
                .collect())
        }

        fn pull_requests(&self, _issue: &IssueUrl, first_branch: &str) -> Result<Vec<PullRequest>> {
            self.seen
                .borrow_mut()
                .push(Read::PullRequests(first_branch.to_string()));
            Ok(self
                .prs
                .iter()
                .filter(|pr| pr.head.starts_with(first_branch))
                .map(|pr| PullRequest {
                    number: pr.number,
                    url: url_of(pr.number),
                    state: pr.state,
                    head: pr.head.to_string(),
                    base: pr.base.to_string(),
                    is_draft: false,
                })
                .collect())
        }

        fn local_head(&self, branch: &str) -> Option<String> {
            self.seen
                .borrow_mut()
                .push(Read::LocalHead(branch.to_string()));
            self.local
                .iter()
                .find(|(name, _)| *name == branch)
                .map(|(_, head)| head.to_string())
        }
    }

    /// Issue #5 in `acme/widgets`.
    fn issue() -> IssueUrl {
        IssueUrl::parse("https://github.com/acme/widgets/issues/5").unwrap()
    }

    /// The URL of pull request `number` in `acme/widgets`.
    fn url_of(number: u64) -> String {
        format!("https://github.com/acme/widgets/pull/{number}")
    }

    /// Pull request `number` from `head` into `main`.
    fn pr(number: u64, head: &'static str, state: PrState) -> Pr {
        Pr {
            number,
            head,
            base: "main",
            state,
        }
    }

    /// A Selection as the tests compare it: a Fresh branch or a Continued
    /// one, with its PR's number if it has one.
    #[derive(Debug, PartialEq, Eq)]
    enum Picked {
        Fresh(String),
        Continued(String, Option<u64>),
    }

    fn fresh(branch: &str) -> Picked {
        Picked::Fresh(branch.to_string())
    }

    fn continued(branch: &str, pr: Option<u64>) -> Picked {
        Picked::Continued(branch.to_string(), pr)
    }

    /// What selection for #5 picks from `reads`.
    fn picked(reads: &InMemory) -> Picked {
        match select_through(reads, &issue()).unwrap() {
            Selection::Fresh { branch } => Picked::Fresh(branch),
            Selection::Continuation { branch, pr } => {
                Picked::Continued(branch, pr.map(|pr| pr.number))
            }
        }
    }

    /// The error selection for #5 fails with from `reads`.
    fn refusal(reads: &InMemory) -> String {
        match select_through(reads, &issue()) {
            Ok(_) => panic!("selection was not refused"),
            Err(error) => error.to_string(),
        }
    }

    /// What shows #5 was started, from `reads`.
    fn started_from(reads: &InMemory) -> Option<Started> {
        started_through(reads, &issue()).unwrap()
    }

    #[test]
    fn issue_branch_numbers_follow_the_names() {
        for (name, expected) in [
            ("issue-5", continued("issue-5", None)),
            ("issue-5-branch-2", continued("issue-5-branch-2", None)),
            ("issue-5-branch-12", continued("issue-5-branch-12", None)),
        ] {
            let reads = InMemory {
                origin: vec![(name, "abc")],
                ..InMemory::default()
            };

            assert_eq!(picked(&reads), expected, "{name}");
        }
    }

    #[test]
    fn names_that_are_not_issue_branches_of_the_issue_count_for_nothing() {
        for name in [
            "issue-5-branch-1",
            "issue-5-branch-02",
            "issue-5-branch-x",
            "issue-5-branch-",
            "issue-50",
        ] {
            let reads = InMemory {
                origin: vec![(name, "abc")],
                prs: vec![pr(1, name, PrState::Merged)],
                ..InMemory::default()
            };

            assert_eq!(picked(&reads), fresh("issue-5"), "{name}");
            assert_eq!(started_from(&reads), None, "{name}");
        }
    }

    #[test]
    fn nothing_used_gives_branch_1() {
        assert_eq!(picked(&InMemory::default()), fresh("issue-5"));
    }

    #[test]
    fn a_merged_or_closed_pr_on_branch_1_gives_branch_2_whether_or_not_it_is_on_origin() {
        for state in [PrState::Merged, PrState::Closed] {
            for origin in [vec![], vec![("issue-5", "abc")]] {
                let reads = InMemory {
                    origin,
                    prs: vec![pr(1, "issue-5", state)],
                    ..InMemory::default()
                };

                assert_eq!(picked(&reads), fresh("issue-5-branch-2"), "{state}");
            }
        }
    }

    #[test]
    fn a_merged_branch_2_gives_branch_3_whether_or_not_it_is_on_origin() {
        for origin in [vec![], vec![("issue-5-branch-2", "abc")]] {
            let reads = InMemory {
                origin,
                prs: vec![
                    pr(1, "issue-5", PrState::Merged),
                    pr(2, "issue-5-branch-2", PrState::Merged),
                ],
                ..InMemory::default()
            };

            assert_eq!(picked(&reads), fresh("issue-5-branch-3"));
        }
    }

    #[test]
    fn a_branch_on_origin_with_an_open_pr_is_continued_with_it() {
        let reads = InMemory {
            origin: vec![("issue-5", "abc")],
            prs: vec![pr(1, "issue-5", PrState::Open)],
            ..InMemory::default()
        };

        assert_eq!(picked(&reads), continued("issue-5", Some(1)));
    }

    #[test]
    fn of_several_prs_from_the_highest_branch_the_newest_decides() {
        let reads = InMemory {
            origin: vec![("issue-5", "abc")],
            prs: vec![
                pr(3, "issue-5", PrState::Open),
                pr(1, "issue-5", PrState::Closed),
            ],
            ..InMemory::default()
        };
        assert_eq!(picked(&reads), continued("issue-5", Some(3)));

        let reads = InMemory {
            origin: vec![("issue-5", "abc")],
            prs: vec![
                pr(1, "issue-5", PrState::Open),
                pr(3, "issue-5", PrState::Closed),
            ],
            ..InMemory::default()
        };
        assert_eq!(picked(&reads), fresh("issue-5-branch-2"));
    }

    #[test]
    fn a_higher_numbered_branch_beats_a_lower_one_with_an_open_pr() {
        let reads = InMemory {
            origin: vec![("issue-5", "abc"), ("issue-5-branch-2", "def")],
            prs: vec![pr(1, "issue-5", PrState::Open)],
            ..InMemory::default()
        };

        assert_eq!(picked(&reads), continued("issue-5-branch-2", None));
    }

    #[test]
    fn an_open_pr_whose_branch_is_gone_from_origin_is_refused_with_no_local_head_read() {
        let reads = InMemory {
            prs: vec![pr(1, "issue-5", PrState::Open)],
            local: vec![("issue-5", "abc")],
            ..InMemory::default()
        };

        assert_eq!(
            refusal(&reads),
            "issue-5 has an open PR but is gone from origin"
        );
        assert_eq!(
            reads.seen.into_inner(),
            [
                Read::OnOrigin("issue-5".to_string()),
                Read::PullRequests("issue-5".to_string()),
            ]
        );
    }

    #[test]
    fn a_local_copy_of_a_fresh_branch_is_refused() {
        let reads = InMemory {
            local: vec![("issue-5", "abc")],
            ..InMemory::default()
        };
        assert_eq!(
            refusal(&reads),
            "the local branch issue-5 is not on origin; push, rename or delete it first"
        );

        let reads = InMemory {
            prs: vec![pr(1, "issue-5", PrState::Merged)],
            local: vec![("issue-5-branch-2", "abc")],
            ..InMemory::default()
        };
        assert_eq!(
            refusal(&reads),
            "the local branch issue-5-branch-2 is not on origin; push, rename or delete it first"
        );
    }

    #[test]
    fn a_local_copy_of_a_continued_branch_at_another_head_is_refused() {
        let reads = InMemory {
            origin: vec![("issue-5", "abc")],
            local: vec![("issue-5", "def")],
            ..InMemory::default()
        };

        assert_eq!(
            refusal(&reads),
            "the local branch issue-5 differs from origin/issue-5; push, reset or delete it first"
        );
    }

    #[test]
    fn a_local_copy_at_origins_head_is_continued() {
        let reads = InMemory {
            origin: vec![("issue-5", "abc")],
            local: vec![("issue-5", "abc")],
            ..InMemory::default()
        };

        assert_eq!(picked(&reads), continued("issue-5", None));
    }

    #[test]
    fn selection_lists_origin_and_the_prs_once_whatever_the_history() {
        let reads = InMemory {
            origin: vec![("issue-5", "abc"), ("issue-5-branch-3", "def")],
            prs: vec![
                pr(1, "issue-5", PrState::Merged),
                pr(2, "issue-5-branch-2", PrState::Closed),
                pr(4, "issue-5-branch-3", PrState::Open),
                pr(3, "issue-50", PrState::Open),
            ],
            ..InMemory::default()
        };

        assert_eq!(picked(&reads), continued("issue-5-branch-3", Some(4)));
        assert_eq!(
            reads.seen.into_inner(),
            [
                Read::OnOrigin("issue-5".to_string()),
                Read::PullRequests("issue-5".to_string()),
                Read::LocalHead("issue-5-branch-3".to_string()),
            ]
        );
    }

    #[test]
    fn started_shows_the_lowest_numbered_branch_on_origin_over_any_pr() {
        let reads = InMemory {
            origin: vec![("issue-5-branch-3", "abc"), ("issue-5-branch-2", "def")],
            prs: vec![pr(9, "issue-5-branch-3", PrState::Open)],
            ..InMemory::default()
        };

        assert_eq!(
            started_from(&reads),
            Some(Started::Branch("issue-5-branch-2".to_string()))
        );
    }

    #[test]
    fn started_shows_the_newest_pr_in_any_state_with_no_branch_on_origin() {
        let reads = InMemory {
            prs: vec![
                pr(2, "issue-5", PrState::Merged),
                pr(4, "issue-5-branch-2", PrState::Closed),
                pr(6, "issue-50", PrState::Open),
            ],
            ..InMemory::default()
        };

        assert_eq!(started_from(&reads), Some(Started::PullRequest(url_of(4))));
    }

    #[test]
    fn started_shows_nothing_when_nothing_was_used() {
        let reads = InMemory {
            origin: vec![("issue-50", "abc")],
            prs: vec![pr(1, "issue-50", PrState::Open)],
            ..InMemory::default()
        };

        assert_eq!(started_from(&reads), None);
    }

    /// The PR's base `selection` gives for `given` and `checked_out`, with
    /// the lines it said.
    fn base_and_lines(
        selection: &Selection,
        given: Option<&str>,
        checked_out: Option<&str>,
    ) -> (Option<String>, Vec<String>) {
        let mut lines = Vec::new();
        let base = selection
            .pr_base_to(given, checked_out, |line| lines.push(line))
            .map(String::from);
        (base, lines)
    }

    /// A Continuation of `issue-5` with its open PR #1 into `develop`.
    fn continued_with_open_pr() -> Selection {
        let reads = InMemory {
            origin: vec![("issue-5", "abc")],
            prs: vec![Pr {
                base: "develop",
                ..pr(1, "issue-5", PrState::Open)
            }],
            ..InMemory::default()
        };
        select_through(&reads, &issue()).unwrap()
    }

    #[test]
    fn there_is_no_pr_base_outside_a_continuation_with_an_open_pr() {
        let fresh = Selection::Fresh {
            branch: "issue-5".to_string(),
        };
        let without_pr = Selection::Continuation {
            branch: "issue-5".to_string(),
            pr: None,
        };

        for selection in [fresh, without_pr] {
            assert_eq!(
                base_and_lines(&selection, Some("main"), Some("main")),
                (None, vec![])
            );
        }
    }

    #[test]
    fn the_pr_base_replaces_the_given_branch_over_the_checked_out_one() {
        assert_eq!(
            base_and_lines(&continued_with_open_pr(), Some("main"), Some("trunk")),
            (
                Some("develop".to_string()),
                vec![format!(
                    "continuing issue-5 and its PR {}, so the Base branch is develop, not the given main",
                    url_of(1)
                )]
            )
        );
    }

    #[test]
    fn the_pr_base_replaces_the_checked_out_branch_with_none_given() {
        assert_eq!(
            base_and_lines(&continued_with_open_pr(), None, Some("trunk")),
            (
                Some("develop".to_string()),
                vec![format!(
                    "continuing issue-5 and its PR {}, so the Base branch is develop, not the checked-out trunk",
                    url_of(1)
                )]
            )
        );
    }

    #[test]
    fn the_pr_base_says_nothing_when_it_matches_or_replaces_nothing() {
        for (given, checked_out) in [
            (Some("develop"), Some("trunk")),
            (None, Some("develop")),
            (None, None),
        ] {
            assert_eq!(
                base_and_lines(&continued_with_open_pr(), given, checked_out),
                (Some("develop".to_string()), vec![]),
                "{given:?} {checked_out:?}"
            );
        }
    }
}
