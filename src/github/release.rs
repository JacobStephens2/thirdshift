//! Release-specific GitHub transport. The Release run owns sequencing and policy.
use super::*;

#[derive(Deserialize)]
pub(crate) struct ReleasePushRun {
    #[serde(rename = "databaseId")]
    pub id: u64,
    #[serde(default)]
    pub attempt: u64,
    #[serde(rename = "headSha")]
    pub head_sha: String,
    pub status: String,
    pub conclusion: Option<String>,
}

pub(crate) struct ReleasePr {
    pub pr: PullRequest,
    pub title: String,
    pub merge_commit: Option<String>,
}

impl ReleasePr {
    fn from_json(value: Value) -> Result<Self> {
        let pr = PullRequest::from_json(&value)?;
        let title = value["title"]
            .as_str()
            .context("release PR has no title")?
            .to_owned();
        let merge_commit = value["mergeCommit"]["oid"].as_str().map(str::to_owned);
        Ok(Self {
            pr,
            title,
            merge_commit,
        })
    }
}

impl GitHub {
    pub(crate) fn close_superseded_release(&self, version: &str) -> Result<()> {
        for pr in self.release_prs()? {
            if pr.pr.head == format!("release-{version}") && pr.pr.state == PrState::Open {
                self.gh_stdout(&[
                    "pr",
                    "close",
                    &pr.pr.url,
                    "--repo",
                    crate::release::REPOSITORY,
                ])?;
            }
        }
        Ok(())
    }
    /// Only a verified missing/draft Release is safe to re-cut. Authentication,
    /// transport errors and malformed responses must never authorize a tag move.
    pub(crate) fn release_published(&self, tag: &str) -> Result<bool> {
        let args = [
            "release",
            "view",
            tag,
            "--repo",
            crate::release::REPOSITORY,
            "--json",
            "url,isDraft",
        ];
        let output = self.output(&args)?;
        if !output.status.success() {
            if output.status.code() == Some(1)
                && String::from_utf8_lossy(&output.stderr).trim() == "release not found"
            {
                return Ok(false);
            }
            bail!(
                "cannot verify GitHub Release publication for {tag}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let value: Value = serde_json::from_slice(&output.stdout)?;
        let draft = value["isDraft"]
            .as_bool()
            .context("release publication response has no isDraft")?;
        anyhow::ensure!(
            value["url"].as_str().is_some_and(|url| !url.is_empty()),
            "release publication response has no URL"
        );
        Ok(!draft)
    }

    pub(crate) fn release_prs(&self) -> Result<Vec<ReleasePr>> {
        let values: Vec<Value> = serde_json::from_value(self.gh_json(&[
            "pr", "list", "--repo", crate::release::REPOSITORY,
            "--state", "all", "--search", "head:release-", "--limit", "1000", "--json",
            "number,url,title,state,headRefName,baseRefName,headRefOid,isCrossRepository,isDraft,mergeCommit",
        ])?)?;
        values.into_iter().map(ReleasePr::from_json).collect()
    }

    pub(crate) fn release_push_run(
        &self,
        workflow: &str,
        branch: &str,
        head: &str,
    ) -> Result<Option<ReleasePushRun>> {
        let mut args = vec![
            "run",
            "list",
            "--repo",
            crate::release::REPOSITORY,
            "--workflow",
            workflow,
            "--event",
            "push",
            "--commit",
            head,
            "--limit",
            "1",
            "--json",
            "databaseId,attempt,headSha,status,conclusion",
        ];
        if !branch.is_empty() {
            args.extend(["--branch", branch]);
        }
        let runs: Vec<ReleasePushRun> = serde_json::from_value(self.gh_json(&args)?)?;
        Ok(runs.into_iter().find(|run| run.head_sha == head))
    }

    pub(crate) fn release_notes(
        &self,
        tag: &str,
        base: &str,
        previous: Option<&str>,
    ) -> Result<String> {
        let mut args = vec![
            "api".to_owned(),
            format!(
                "repos/{}/releases/generate-notes",
                crate::release::REPOSITORY
            ),
            "-f".into(),
            format!("tag_name={tag}"),
            "-f".into(),
            format!("target_commitish={base}"),
            "--jq".into(),
            ".body".into(),
        ];
        if let Some(previous) = previous {
            args.extend(["-f".into(), format!("previous_tag_name={previous}")]);
        }
        self.gh_stdout(&args.iter().map(String::as_str).collect::<Vec<_>>())
    }

    pub(crate) fn release_failure(&self, id: u64) -> Result<String> {
        let detail = self.gh_json(&[
            "run",
            "view",
            &id.to_string(),
            "--repo",
            crate::release::REPOSITORY,
            "--json",
            "url,jobs",
        ])?;
        let url = detail["url"]
            .as_str()
            .context("release workflow has no URL")?;
        let jobs = detail["jobs"]
            .as_array()
            .context("release workflow has no jobs")?;
        let failed = jobs
            .iter()
            .filter(|job| {
                !matches!(
                    job["conclusion"].as_str(),
                    Some("success" | "skipped" | "neutral")
                )
            })
            .filter_map(|job| job["name"].as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Ok(format!(
            "{failed} at {url}; recover with gh run rerun --failed {id}"
        ))
    }

    pub(crate) fn open_release(&self, version: &str, branch: &str, body: &str) -> Result<String> {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new()?;
        file.write_all(body.as_bytes())?;
        self.gh_stdout(&[
            "pr",
            "create",
            "--repo",
            crate::release::REPOSITORY,
            "--head",
            branch,
            "--base",
            "main",
            "--title",
            &format!("Release {version}"),
            "--body-file",
            file.path().to_str().context("non-UTF8 release body path")?,
        ])
    }

    pub(crate) fn release_pr_input(&self, number: &str) -> Result<Value> {
        self.gh_json(&[
            "pr",
            "view",
            number,
            "--repo",
            crate::release::REPOSITORY,
            "--json",
            "number,title,body",
        ])
    }
}
