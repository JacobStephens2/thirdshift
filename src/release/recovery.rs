//! Discover interrupted cuts before selecting new content. Every adopted tree
//! must be reproducible from main with only the exact manifest/lockfile bump.
use std::collections::BTreeMap;

use crate::github::release::ReleasePr;
use anyhow::{Context, Result, ensure};

use super::*;

pub(super) struct Cut {
    pub version: String,
    pub base: String,
    pub previous: Option<String>,
    pub head: String,
    pub pr: Option<crate::review::PullRequestUrl>,
    pub tagged: Option<String>,
    pub published: bool,
    pub recut: bool,
    pub supersedes: Option<String>,
    pub expected_branch: Option<String>,
    pub expected_tag: Option<String>,
}

impl Cut {
    pub fn branch(&self) -> String {
        format!("release-{}", self.version)
    }
    pub fn tag(&self) -> String {
        format!("v{}", self.version)
    }

    pub fn commit(&mut self, git: &Git) -> Result<()> {
        checkout(git, &self.base)?;
        bump(git.dir(), &self.version)?;
        git_run(git, &["add", "--", "Cargo.toml", "Cargo.lock"])?;
        git_run(
            git,
            &[
                "-c",
                "commit.gpgSign=false",
                "commit",
                "--allow-empty",
                "-m",
                &format!(
                    "Release {}{}",
                    self.version,
                    self.supersedes
                        .as_ref()
                        .map(|old| format!("\n\nThirdshift-Supersedes: {old}"))
                        .unwrap_or_default()
                ),
            ],
        )?;
        self.head = git_run(git, &["rev-parse", "HEAD"])?;
        Ok(())
    }

    pub fn validate(&mut self, git: &Git, github: &GitHub) -> Result<()> {
        fetch(git)?;
        git_run(git, &["fetch", "--quiet", "origin", &self.head])?;
        let target = if let Some(pr) = &self.pr {
            let snapshot = github
                .pr_snapshot(&pr.issue, &pr.url)?
                .context("release PR disappeared")?;
            bump_identity(&snapshot, &self.branch())?;
            ensure!(
                snapshot.pr.state != PrState::Closed,
                "{} identity/state changed; inspect it before retrying thirdshift release {}",
                pr.url,
                self.version
            );
            ensure!(
                snapshot.head_commit == self.head,
                "release PR head changed during validation; retry thirdshift release {}",
                self.version
            );
            if snapshot.pr.state == PrState::Merged {
                let input = github
                    .release_prs()?
                    .into_iter()
                    .find(|row| row.pr.url == pr.url)
                    .context("merged release PR disappeared")?;
                let merge = input
                    .merge_commit
                    .as_deref()
                    .context("merged release PR has no merge commit")?;
                git_run(git, &["fetch", "--quiet", "origin", merge])?;
                format!("{merge}^1")
            } else {
                let remote = remote_head(git, &self.branch())?;
                ensure!(
                    remote.as_deref() == Some(&self.head),
                    "release PR head does not match origin; inspect {}",
                    pr.url
                );
                "origin/main".into()
            }
        } else {
            "origin/main".into()
        };
        self.base = git_run(git, &["merge-base", &self.head, &target])?;
        exact_bump(git, &self.base, &self.head, &self.version)
    }

    pub fn push(&self, git: &Git, github: &GitHub) -> Result<()> {
        ensure!(
            !github.release_published(&self.tag())?,
            "{} is already published; its branch and tag will not be moved",
            self.tag()
        );
        if let Some(old) = &self.supersedes {
            ensure!(
                !github.release_published(&format!("v{old}"))?,
                "superseded release v{old} was published; retry discovery before continuing"
            );
        }
        let branch = self.branch();
        let old = remote_head(git, &branch)?;
        ensure!(
            old == self.expected_branch,
            "{branch} changed while preparing the release; inspect it before retrying"
        );
        let lease = format!(
            "--force-with-lease=refs/heads/{branch}:{}",
            old.as_deref().unwrap_or("")
        );
        git_run(
            git,
            &[
                "push",
                "--no-verify",
                &lease,
                "origin",
                &format!("{}:refs/heads/{branch}", self.head),
            ],
        )?;
        Ok(())
    }

    fn recut(&mut self, git: &Git, github: &GitHub, pinned: Option<Bump>) -> Result<()> {
        let candidate = selection::candidate(
            git,
            github,
            Some(&Version::parse(&self.version)?),
            self.previous.as_deref(),
        )?;
        let selection::Candidate::Ready { base, .. } = candidate else {
            return Ok(());
        };
        if base == self.base || git_run(git, &["merge-base", &base, &self.base])? != self.base {
            return Ok(());
        }
        self.base = base;
        if let Some(pinned) = pinned {
            let requested = main_version(git)?.bumped(pinned)?;
            if Version::parse(&requested)? > Version::parse(&self.version)? {
                self.supersedes = Some(self.version.clone());
                self.version = requested;
                self.pr = None;
                self.tagged = None;
                self.expected_branch = None;
                self.expected_tag = None;
            }
        }
        if let Some(pr) = &self.pr
            && github.pr_state(&pr.issue, &pr.url)? == PrState::Merged
        {
            self.pr = None;
        }
        self.commit(git)?;
        self.recut = true;
        Ok(())
    }
}

pub(super) enum Discovery {
    Fresh { previous: Option<String> },
    Resume(Box<Cut>),
    Blocked(String),
}

struct Unfinished {
    version: String,
    branch_head: Option<String>,
    tag_head: Option<String>,
    pr: Option<ReleasePr>,
}

impl Unfinished {
    fn new(version: &str) -> Self {
        Self {
            version: version.into(),
            branch_head: None,
            tag_head: None,
            pr: None,
        }
    }
}

/// Tags are inspected newest first, stopping at the newest published Release.
/// Failed publication lookups are human blockers, including during Passes.
pub(super) fn discover(
    git: &Git,
    launch: &Git,
    github: &GitHub,
    wanted: Option<&str>,
    pinned: Option<Bump>,
) -> Result<Discovery> {
    match inspect(git, launch, github, wanted, pinned) {
        Ok(discovery) => Ok(discovery),
        Err(error) if !interrupt::requested() => Ok(Discovery::Blocked(format!(
            "unfinished release discovery: {error:#}"
        ))),
        Err(error) => Err(error),
    }
}

fn inspect(
    git: &Git,
    launch: &Git,
    github: &GitHub,
    wanted: Option<&str>,
    pinned: Option<Bump>,
) -> Result<Discovery> {
    let prs = github.release_prs()?;
    let refs = git_run(
        git,
        &[
            "ls-remote",
            "--refs",
            "origin",
            "refs/heads/release-*",
            "refs/tags/v*",
        ],
    )?;
    let mut live: BTreeMap<Version, Unfinished> = BTreeMap::new();
    let mut tags = Vec::new();
    for row in refs.lines() {
        let Some((sha, reference)) = row.split_once('\t') else {
            continue;
        };
        if let Some(version) = reference.strip_prefix("refs/heads/release-") {
            if let Ok(parsed) = Version::parse(version) {
                let mut unfinished = Unfinished::new(version);
                unfinished.branch_head = Some(sha.into());
                live.insert(parsed, unfinished);
            }
        } else if let Some(version) = reference.strip_prefix("refs/tags/v")
            && let Ok(parsed) = Version::parse(version)
        {
            tags.push((parsed, version.to_owned()));
        }
    }
    tags.sort_by(|a, b| b.0.cmp(&a.0));
    let mut previous = None;
    let mut boundary = None;
    for (parsed, version) in tags {
        let tag = format!("v{version}");
        match github.release_published(&tag) {
            Ok(true) => {
                previous = Some(tag);
                boundary = Some(parsed);
                break;
            }
            Ok(false) => {
                let head = git_run(git, &["rev-parse", &format!("{tag}^{{commit}}")])?;
                live.entry(parsed)
                    .or_insert_with(|| Unfinished::new(&version))
                    .tag_head = Some(head);
            }
            Err(error) => {
                return Ok(Discovery::Blocked(format!(
                    "unfinished release {version}: {error:#}; verify publication before retrying thirdshift release {version}"
                )));
            }
        }
    }
    for pr in prs {
        let Some(version) = pr.pr.head.strip_prefix("release-") else {
            continue;
        };
        let Ok(parsed) = Version::parse(version) else {
            continue;
        };
        if pr.pr.state == PrState::Open || wanted == Some(version) || live.contains_key(&parsed) {
            let entry = live
                .entry(parsed)
                .or_insert_with(|| Unfinished::new(version));
            // GitHub lists newest first. Never substitute an older closed PR.
            if entry.pr.is_none() {
                entry.pr = Some(pr);
            }
        }
    }
    live.retain(|version, entry| {
        boundary.as_ref().is_none_or(|latest| version > latest)
            || entry
                .pr
                .as_ref()
                .is_some_and(|pr| pr.pr.state == PrState::Open)
            || wanted == Some(&entry.version)
    });
    if live.len() > 1 {
        let mut superseded = Vec::new();
        for entry in live.values() {
            let Some(head) = &entry.branch_head else {
                continue;
            };
            git_run(git, &["fetch", "--quiet", "origin", head])?;
            if let Some(old) = supersedes(git, head)?
                && let Some(prior) = live.get(&Version::parse(&old)?)
            {
                let old_head = prior
                    .branch_head
                    .as_ref()
                    .or(prior.tag_head.as_ref())
                    .context("superseded release has no head")?;
                git_run(git, &["fetch", "--quiet", "origin", old_head])?;
                let base = git_run(git, &["merge-base", head, "origin/main"])?;
                let old_base = git_run(git, &["merge-base", old_head, "origin/main"])?;
                ensure!(
                    prior
                        .pr
                        .as_ref()
                        .is_none_or(|pr| !pr.pr.is_draft && pr.pr.state != PrState::Closed),
                    "superseded release {old} requires human review"
                );
                ensure!(
                    !github.release_published(&format!("v{old}"))?,
                    "superseded release {old} was published; inspect both cuts"
                );
                exact_bump(git, &base, head, &entry.version)?;
                exact_bump(git, &old_base, old_head, &old)?;
                superseded.push(Version::parse(&old)?);
            }
        }
        for old in superseded {
            live.remove(&old);
        }
    }
    if let Some(wanted) = wanted
        && let Some(other) = live.values().find(|entry| entry.version != wanted)
    {
        return Ok(Discovery::Blocked(format!(
            "unfinished release {} is live; finish it with thirdshift release {} before releasing {wanted}",
            other.version, other.version
        )));
    }
    if live.len() > 1 {
        return Ok(Discovery::Blocked(format!(
            "multiple unfinished releases: {}; finish or remove the obsolete cuts before retrying",
            live.values()
                .map(|entry| entry.version.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let Some((
        _,
        Unfinished {
            version,
            branch_head,
            tag_head: tagged,
            pr,
        },
    )) = live.into_iter().next()
    else {
        return Ok(match pinned_cut(launch, git, wanted, previous.clone()) {
            Ok(Some(cut)) => Discovery::Resume(Box::new(cut)),
            Ok(None) => Discovery::Fresh { previous },
            Err(error) => Discovery::Blocked(format!("unfinished local release: {error:#}")),
        });
    };
    let result = (|| -> Result<Cut> {
        let tag = format!("v{version}");
        let published = github.release_published(&tag)?;
        let expected_tag =
            git_optional(git, &["rev-parse", "--verify", &format!("refs/tags/{tag}")])?;
        let tagged = if expected_tag.is_some() {
            Some(git_run(
                git,
                &["rev-parse", &format!("refs/tags/{tag}^{{commit}}")],
            )?)
        } else {
            tagged
        };
        let pr = pr
            .map(|pr| -> Result<_> {
                ensure!(
                    pr.title == format!("Release {version}"),
                    "release PR has an unexpected title"
                );
                ensure!(
                    pr.pr.state != PrState::Closed,
                    "release {version} was closed; reopen its PR before retrying"
                );
                crate::review::PullRequestUrl::parse(&pr.pr.url)
            })
            .transpose()?;
        let head = if let Some(pr) = &pr {
            github
                .pr_snapshot(&pr.issue, &pr.url)?
                .context("release PR disappeared")?
                .head_commit
        } else {
            branch_head
                .clone()
                .or_else(|| tagged.clone())
                .context("release has no head")?
        };
        let mut cut = Cut {
            version,
            base: String::new(),
            previous,
            head,
            pr,
            tagged,
            published,
            recut: false,
            supersedes: None,
            expected_branch: branch_head,
            expected_tag,
        };
        cut.validate(git, github)?;
        cut.supersedes = supersedes(git, &cut.head)?;
        if cut.tagged.is_none()
            && !cut.published
            && let Some(previous) = &cut.previous
            && git_run(git, &["merge-base", previous, &cut.base])?
                != git_run(git, &["rev-parse", &format!("{previous}^{{commit}}")])?
        {
            cut.recut(git, github, pinned)?;
        }
        if let Some(tagged) = cut.tagged.clone() {
            ensure!(
                !published || tagged == cut.head,
                "published tag differs from the release PR; never move a published tag"
            );
            if !published {
                let run = github.release_push_run("release.yml", "", &tagged)?;
                if tagged != cut.head {
                    ensure!(
                        run.as_ref().is_some_and(|run| run.status == "completed"
                            && run.conclusion.as_deref() != Some("success")),
                        "tag {} differs from the bump head without a failed unpublished workflow",
                        cut.tag()
                    );
                    let parents = git_run(git, &["show", "-s", "--format=%P", &tagged])?;
                    let mut valid = false;
                    for parent in parents.split_whitespace() {
                        if parent != cut.base
                            && git_run(git, &["merge-base", parent, &cut.base])? == parent
                            && is_exact_bump(git, parent, &tagged, &cut.version)?
                        {
                            valid = true;
                            break;
                        }
                    }
                    ensure!(
                        valid,
                        "{} is not an earlier exact version bump; inspect its unrelated tag before recovery",
                        cut.tag()
                    );
                }
                if run.as_ref().is_some_and(|run| {
                    run.status == "completed" && run.conclusion.as_deref() != Some("success")
                }) {
                    cut.recut(git, github, pinned)?;
                }
            }
        }
        Ok(cut)
    })();
    match result {
        Ok(cut) => Ok(Discovery::Resume(Box::new(cut))),
        Err(error) => Ok(Discovery::Blocked(format!("unfinished release: {error:#}"))),
    }
}

pub(super) fn remote_head(git: &Git, branch: &str) -> Result<Option<String>> {
    Ok(git_run(
        git,
        &[
            "ls-remote",
            "--refs",
            "origin",
            &format!("refs/heads/{branch}"),
        ],
    )?
    .split_whitespace()
    .next()
    .map(str::to_owned))
}

fn supersedes(git: &Git, head: &str) -> Result<Option<String>> {
    let message = git_run(git, &["show", "-s", "--format=%B", head])?;
    Ok(message
        .lines()
        .filter_map(|line| line.strip_prefix("Thirdshift-Supersedes: "))
        .find(|version| Version::parse(version).is_ok())
        .map(str::to_owned))
}

fn pinned_cut(
    launch: &Git,
    git: &Git,
    wanted: Option<&str>,
    previous: Option<String>,
) -> Result<Option<Cut>> {
    let refs = git_run(
        launch,
        &[
            "for-each-ref",
            "--format=%(refname:strip=3) %(objectname)",
            "refs/thirdshift/release-candidates/",
        ],
    )?;
    let mut pins = Vec::new();
    for line in refs.lines() {
        let Some((version, base)) = line.split_once(' ') else {
            continue;
        };
        let Ok(parsed) = Version::parse(version) else {
            continue;
        };
        if previous
            .as_deref()
            .and_then(|tag| Version::parse(tag.trim_start_matches('v')).ok())
            .is_some_and(|published| parsed <= published)
        {
            continue;
        }
        git_run(
            git,
            &[
                "fetch",
                "--quiet",
                launch.dir().to_str().context("non-UTF8 Launch path")?,
                base,
            ],
        )?;
        ensure!(
            git_run(git, &["merge-base", base, "origin/main"])? == base,
            "candidate for {version} is not content from origin/main; inspect refs/thirdshift/release-candidates/{version}"
        );
        if let Some(tag) = &previous
            && git_run(git, &["merge-base", tag, base])?
                != git_run(git, &["rev-parse", &format!("{tag}^{{commit}}")])?
        {
            continue;
        }
        if parsed <= main_version(git)? {
            ensure!(
                wanted != Some(version),
                "main overtook pinned release {version}; choose a higher version"
            );
            continue;
        }
        pins.push((version.to_owned(), base.to_owned()));
    }
    ensure!(
        pins.len() <= 1,
        "multiple unfinished local release candidates; inspect refs/thirdshift/release-candidates before retrying"
    );
    let Some((version, base)) = pins.pop() else {
        return Ok(None);
    };
    ensure!(
        wanted.is_none_or(|wanted| wanted == version),
        "unfinished local release {version}; resume it with thirdshift release {version}"
    );
    Ok(Some(Cut {
        version,
        base,
        previous,
        head: String::new(),
        pr: None,
        tagged: None,
        published: false,
        recut: false,
        supersedes: None,
        expected_branch: None,
        expected_tag: None,
    }))
}

pub(super) fn ensure_previous(git: &Git, github: &GitHub, previous: Option<&str>) -> Result<()> {
    fetch(git)?;
    let mut versions = git_run(git, &["tag", "--list", "v[0-9]*"])?
        .lines()
        .filter_map(|tag| {
            Version::parse(tag.strip_prefix('v')?)
                .ok()
                .map(|version| (version, tag.to_owned()))
        })
        .collect::<Vec<_>>();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, tag) in versions {
        if github.release_published(&tag)? {
            ensure!(
                Some(tag.as_str()) == previous,
                "a newer published release {tag} appeared; retry release discovery"
            );
            return Ok(());
        }
    }
    ensure!(
        previous.is_none(),
        "previous GitHub Release disappeared; verify publication before retrying"
    );
    Ok(())
}

pub(super) fn exact_bump(git: &Git, base: &str, head: &str, version: &str) -> Result<()> {
    ensure!(
        is_exact_bump(git, base, head, version)?,
        "release-{version} changes more than the exact version bump in Cargo.toml and Cargo.lock; review the non-bump commits before retrying thirdshift release {version}"
    );
    Ok(())
}

fn is_exact_bump(git: &Git, base: &str, head: &str, version: &str) -> Result<bool> {
    checkout(git, base)?;
    bump(git.dir(), version)?;
    git_run(git, &["add", "--", "Cargo.toml", "Cargo.lock"])?;
    let expected = git_run(git, &["write-tree"])?;
    let actual = git_run(git, &["rev-parse", &format!("{head}^{{tree}}")])?;
    git_run(git, &["reset", "--hard", base])?;
    Ok(expected == actual)
}
