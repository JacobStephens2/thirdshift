//! Evidence and publication for Pickup's ordinary CI-red Ticket. This is
//! independent of Base fixes and the head checks that authorize Self-merge.

use super::*;
use crate::labels::{Label, READY_FOR_AGENT};

const TITLE_PREFIX: &str = "CI red on ";
const GRACE: chrono::Duration = chrono::Duration::minutes(30);

pub(crate) struct BaseCiFailure {
    title: String,
    body: String,
}

#[derive(Deserialize)]
struct PushRun {
    id: u64,
    status: String,
    conclusion: Option<String>,
    updated_at: DateTime<Utc>,
    head_sha: String,
    html_url: String,
}

#[derive(Deserialize)]
struct Job {
    id: u64,
    name: String,
    conclusion: Option<String>,
}

impl GitHub {
    /// The latest push run, including pending runs so an older failure cannot
    /// trigger filing while a newer run is underway. All evidence must be
    /// readable before returning a draft; a failed read never publishes one.
    pub(crate) fn base_ci_failure(&self, repo: &Repo, base: &str) -> Result<Option<BaseCiFailure>> {
        let query =
            serde_urlencoded::to_string([("branch", base), ("event", "push"), ("per_page", "1")])?;
        let runs = self.gh_json(&[
            "api",
            &format!("repos/{}/actions/runs?{query}", repo.slug()),
        ])?;
        let runs = runs["workflow_runs"]
            .as_array()
            .context("CI response has no workflow_runs")?;
        let Some(run) = runs.first() else {
            return Ok(None);
        };
        let run: PushRun = serde_json::from_value(run.clone())?;
        if run.status != "completed"
            || run.conclusion.as_deref() != Some("failure")
            || Utc::now().signed_duration_since(run.updated_at) <= GRACE
        {
            return Ok(None);
        }
        let jobs = self.gh_api_items(
            &format!(
                "repos/{}/actions/runs/{}/jobs?per_page=100",
                repo.slug(),
                run.id
            ),
            "jobs",
        )?;
        let jobs: Vec<Job> = jobs
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()?;
        let failed: Vec<_> = jobs
            .iter()
            .filter(|job| {
                matches!(
                    job.conclusion.as_deref(),
                    Some("failure" | "timed_out" | "action_required")
                )
            })
            .collect();
        // A failed workflow may have no jobs (for example an invalid workflow).
        // Its run still provides actionable evidence.
        let names = if failed.is_empty() {
            "workflow failure".to_string()
        } else {
            failed
                .iter()
                .map(|job| job.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut body = format!(
            "The latest push CI on `{base}` failed more than 30 minutes ago.\n\nFailing commit: `{}`\nRun: {}\n\nFailed jobs: {names}\n",
            run.head_sha, run.html_url,
        );
        for job in failed {
            let logs = self.gh_stdout(&[
                "api",
                &format!("repos/{}/actions/jobs/{}/logs", repo.slug(), job.id),
            ])?;
            body.push_str(&format!(
                "\n### {}\n\nRecognizable failing test lines (up to 50):\n\n",
                job.name
            ));
            let mut found = false;
            for line in logs
                .lines()
                .filter(|line| {
                    line.contains("FAILED")
                        || line.contains("FAIL ")
                        || line.contains("✕")
                        || line.contains("×")
                })
                .take(50)
            {
                found = true;
                body.push_str("    ");
                body.extend(line.chars().take(500));
                body.push('\n');
            }
            if !found {
                body.push_str("No failing test names recognized in this job's logs.\n");
            }
        }
        Ok(Some(BaseCiFailure {
            title: format!("{TITLE_PREFIX}{base}: {names}"),
            body,
        }))
    }

    /// Caller holds the repository-wide CI filing reservation. Use the REST
    /// issue list rather than the eventually indexed search API, and inspect
    /// every page irrespective of labels or Base branch.
    pub(crate) fn file_ci_red_ticket(
        &self,
        repo: &Repo,
        failure: BaseCiFailure,
    ) -> Result<Option<IssueUrl>> {
        let issues = self.gh_api_items(
            &format!(
                "repos/{}/issues?state=open&sort=created&direction=desc&per_page=100",
                repo.slug()
            ),
            "",
        )?;
        for issue in issues {
            if issue.get("pull_request").is_some() {
                continue;
            }
            let title = issue["title"].as_str().context("issue has no title")?;
            if title.starts_with(TITLE_PREFIX) {
                return Ok(None);
            }
        }
        self.create_issue_in(
            repo,
            &failure.title,
            &failure.body,
            &[
                Label::new("bug", "Something isn't working"),
                READY_FOR_AGENT,
            ],
        )
        .map(Some)
    }
}
