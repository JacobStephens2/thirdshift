//! Publish a Review run's fixes only while the branch is still the reviewed head, then its report.

use super::*;
use crate::github::review::ReviewInput;

pub(super) fn deliver(
    git: &Git,
    github: &GitHub,
    pr: &PullRequestUrl,
    snapshot: &ReviewPullRequest,
    mode: Mode,
    input: &ReviewInput,
    findings: &str,
) -> Result<()> {
    let publication = match mode {
        Mode::RecommendOnly => {
            unchanged_checkout(git, snapshot)?;
            Ok(None)
        }
        Mode::Address => push_fixes(git, github, pr, snapshot),
    };
    let current = if mode == Mode::RecommendOnly {
        unchanged(github, pr, snapshot)?;
        None
    } else {
        Some(compatible(github, pr, snapshot)?)
    };
    let mut marker = serde_json::json!({"reviewed_head": snapshot.head});
    let status = match &publication {
        Ok(Some(head)) => {
            marker["pushed_head"] = head.clone().into();
            format!("Pushed fixes to {} at {head}.", snapshot.branch)
        }
        Ok(None) if mode == Mode::RecommendOnly => "--recommend-only; nothing pushed.".into(),
        Ok(None) => "No fixes to push.".into(),
        Err(error) if current.as_ref().is_some_and(|pr| pr.head != snapshot.head) => {
            format!(
                "The branch moved; no fixes were pushed. Proposed fixes remain in the local worktree. {error:#}"
            )
        }
        Err(error) => {
            format!("Could not push fixes; proposed fixes remain in the local worktree. {error:#}")
        }
    };
    let spec = if input.body_alone {
        "Spec axis used the pull request body alone; no closing issue was linked."
    } else {
        "Spec axis used the linked closing issues."
    };
    github.comment_review(
        pr,
        &format!(
            "Review run. {status} {spec}\n\n{findings}\n\n<!-- thirdshift-review {marker} -->\n"
        ),
    )?;
    publication.map(|_| ())
}

fn push_fixes(
    git: &Git,
    github: &GitHub,
    pr: &PullRequestUrl,
    snapshot: &ReviewPullRequest,
) -> Result<Option<String>> {
    let head = git.run(&["rev-parse", "HEAD"])?;
    ensure!(
        git.run(&["branch", "--show-current"])? == snapshot.branch
            && git.run(&["status", "--porcelain"])?.is_empty()
            && git.succeeds(&["merge-base", "--is-ancestor", &snapshot.head, &head])?,
        "review fixes must be committed on the pull request's captured branch without rewriting history"
    );
    ensure!(
        compatible(github, pr, snapshot)?.head == snapshot.head,
        "the branch moved during Review run"
    );
    if head == snapshot.head {
        return Ok(None);
    }
    // Compare the remote ref with the reviewed head at receive time. A plain
    // fast-forward republishes that head when the branch was rewound to an
    // ancestor after the API check. The reviewed head is an ancestor of this
    // commit, so a matching lease stays a fast-forward and sends no force update.
    git.push_checked(&snapshot.branch, &head, Some(&snapshot.head), || {
        compatible(github, pr, snapshot).map(|_| ())
    })?;
    Ok(Some(head))
}
