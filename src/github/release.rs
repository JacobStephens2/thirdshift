//! Release-specific GitHub transport. The Release run owns sequencing and policy.
use super::*;

#[derive(Deserialize)]
pub(crate) struct ReleasePushRun {
    #[serde(rename = "headSha")]
    pub head_sha: String,
    pub status: String,
    pub conclusion: Option<String>,
}

impl GitHub {
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
            "headSha,status,conclusion",
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
