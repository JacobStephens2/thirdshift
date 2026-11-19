//! Compare the two immutable tips from the Inbox's batched PR read.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct BaseComparison {
    branch: String,
    base: String,
    head: String,
    first_parent_behind: Option<usize>,
}

fn commit_id(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

impl BaseComparison {
    pub(super) fn from_json(pr: &Value) -> Option<Self> {
        let base = commit_id(&pr["baseRef"]["target"]["oid"])?;
        let head = commit_id(&pr["headRefOid"])?;
        Some(Self {
            branch: pr["baseRefName"].as_str()?.to_owned(),
            base: base.into(),
            head: head.into(),
            first_parent_behind: (base == head).then_some(0),
        })
    }

    pub(super) fn stale_ci(&self) -> Option<String> {
        let count = self.first_parent_behind.filter(|count| *count > 0)?;
        let (merges, pronoun) = if count == 1 {
            ("merge", "it")
        } else {
            ("merges", "them")
        };
        Some(format!(
            "{count} {merges} behind {}: CI result predates {pronoun}",
            self.branch
        ))
    }
}

impl GitHub {
    pub(crate) fn refresh_inbox_base(
        &self,
        url: &str,
        pull: &mut PullRequestState,
        previous: Option<&InboxState>,
    ) -> Result<()> {
        let Some(comparison) = pull.base_comparison.as_mut() else {
            return Ok(());
        };
        if comparison.first_parent_behind.is_some() {
            return Ok(());
        }
        if let Some(InboxState::PullRequest(previous)) = previous
            && let Some(previous) = previous.base_comparison.as_ref()
            && previous.base == comparison.base
            && previous.head == comparison.head
            && previous.first_parent_behind.is_some()
        {
            comparison.first_parent_behind = previous.first_parent_behind;
            return Ok(());
        }
        let (owner, repo, _) = split_pull_request_url(url).context("invalid Inbox PR URL")?;
        let repo = format!("{owner}/{repo}");
        comparison.first_parent_behind = self.first_parent_behind(&repo, comparison)?;
        Ok(())
    }

    fn comparison_page(&self, path: &str) -> Result<Option<Value>> {
        if QuotaWait::warning(&self.quota_waits()?).is_some()
            || QuotaWait::warning(&self.inbox_quotas()?).is_some()
        {
            return Ok(None);
        }
        self.gh_json(&["api", path]).map(Some)
    }

    fn first_parent_behind(&self, repo: &str, tips: &BaseComparison) -> Result<Option<usize>> {
        let prefix = format!("repos/{repo}/compare");
        let Some(comparison) = self.comparison_page(&format!(
            "{prefix}/{}...{}?per_page=1",
            tips.head, tips.base
        ))?
        else {
            return Ok(None);
        };
        let fork = commit_id(&comparison["merge_base_commit"]["sha"])
            .context("GitHub comparison did not report a merge base")?;
        if fork == tips.base {
            return Ok(Some(0));
        }

        // GitHub compares all commits in fork..Base in chronological order.
        // Follow only parent zero from Base until the next commit is outside
        // that range. The merge base itself need not be on the first-parent chain.
        let mut parents = HashMap::new();
        for page in 1.. {
            let Some(result) = self.comparison_page(&format!(
                "{prefix}/{fork}...{}?per_page=100&page={page}",
                tips.base
            ))?
            else {
                return Ok(None);
            };
            let total = result["total_commits"]
                .as_u64()
                .context("GitHub comparison did not report a commit count")?;
            let commits = result["commits"]
                .as_array()
                .filter(|commits| !commits.is_empty())
                .context("GitHub comparison did not return the requested commits")?;
            for commit in commits {
                let sha = commit_id(&commit["sha"])
                    .context("GitHub comparison returned a commit without an ID")?;
                let first_parent = commit_id(&commit["parents"][0]["sha"])
                    .context("GitHub comparison returned a commit without a first parent")?;
                if parents
                    .insert(sha.to_owned(), first_parent.to_owned())
                    .is_some()
                {
                    bail!("GitHub comparison returned duplicate commits");
                }
            }
            if parents.len() as u64 >= total {
                break;
            }
        }
        if !parents.contains_key(&tips.base) {
            bail!("GitHub comparison omitted the Base tip");
        }
        let mut count = 0;
        let mut tip = tips.base.clone();
        while let Some(parent) = parents.remove(&tip) {
            count += 1;
            tip = parent;
        }
        Ok(Some(count))
    }
}
