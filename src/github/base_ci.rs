//! Pickup's Base branch CI re-runs, evidence and Ticket publication, independent
//! of Base fixes and the head checks that authorize Self-merge.

use super::*;
use crate::labels::{FLAKY_TEST, Label, NEEDS_INFO, READY_FOR_AGENT};

const TITLE_PREFIX: &str = "CI red on ";
const FLAKY_PREFIX: &str = "Flaky CI on ";
const GRACE: chrono::Duration = chrono::Duration::minutes(30);

pub(crate) struct BaseCiEvidence {
    base: String,
    run: PushRun,
    failed: Vec<Job>,
    previous: Option<PushRun>,
}

pub(crate) enum BaseCiAction {
    Nothing,
    RerunRequested,
    CiRedFiled(IssueUrl),
    FlakyFiled(IssueUrl),
    Commented(IssueUrl),
}

impl BaseCiEvidence {
    fn flaky(&self) -> bool {
        self.run.conclusion.as_deref() == Some("success")
    }

    fn failed_attempt_marker(&self) -> String {
        format!(
            "<!-- thirdshift:base-ci-flaky {}/{} -->",
            self.run.id,
            self.run.run_attempt - 1
        )
    }
}

#[derive(Deserialize)]
struct PushRun {
    id: u64,
    status: String,
    conclusion: Option<String>,
    updated_at: DateTime<Utc>,
    head_sha: String,
    html_url: String,
    run_attempt: u64,
}

#[derive(Deserialize)]
struct Job {
    id: u64,
    name: String,
    conclusion: Option<String>,
}

struct CiReport {
    title: String,
    body: String,
    tests: Vec<String>,
}

fn failing_test_name(line: &str) -> Option<&str> {
    if let Some((_, rest)) = line.split_once("test ")
        && let Some((name, _)) = rest.split_once(" ...")
    {
        return Some(name.trim());
    }
    if let Some((_, rest)) = line.split_once("FAIL [")
        && let Some((_, test)) = rest.split_once("] ")
    {
        return test.split_whitespace().last();
    }
    for marker in ["✕", "×", "✖"] {
        if let Some((_, name)) = line.split_once(marker) {
            if name.trim() == "failing tests:" {
                return None;
            }
            return Some(name.trim().split(" (").next().unwrap());
        }
    }
    line.split_once("not ok ")?
        .1
        .split_once(" - ")
        .map(|(_, name)| name.trim())
}

/// Outsiders can open an issue or comment with any text. Only a Trusted
/// author's report is dedup evidence, and only that issue may receive the
/// owner's CI comment.
fn trusted_report(record: &serde_json::Value) -> bool {
    crate::trust::trusted_association(record["author_association"].as_str().unwrap_or(""))
}

fn mentions_test(text: &str, test: &str) -> bool {
    fn continues_name(mut characters: impl Iterator<Item = char>) -> bool {
        let name_character =
            |character: char| character.is_alphanumeric() || "_:/-".contains(character);
        match characters.next() {
            // An internal dot joins an identifier; a sentence's final period
            // still delimits a test named in prose.
            Some('.') => characters.next().is_some_and(name_character),
            Some(character) => name_character(character),
            None => false,
        }
    }
    text.match_indices(test).any(|(start, _)| {
        !continues_name(text[..start].chars().rev())
            && !continues_name(text[start + test.len()..].chars())
    })
}

impl GitHub {
    /// Read the latest push attempt and its preceding outcome. A pending latest
    /// run hides earlier failures; report construction reads logs before any
    /// publication, so a failed evidence read never publishes a Ticket.
    pub(crate) fn base_ci_evidence(
        &self,
        repo: &Repo,
        base: &str,
    ) -> Result<Option<BaseCiEvidence>> {
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
        let flaky = run.conclusion.as_deref() == Some("success");
        if run.status != "completed"
            || !matches!(run.conclusion.as_deref(), Some("success" | "failure"))
            || (flaky && run.run_attempt == 1)
            || (!flaky
                && run.run_attempt > 1
                && Utc::now().signed_duration_since(run.updated_at) <= GRACE)
        {
            return Ok(None);
        }
        let previous: Option<PushRun> = if run.run_attempt > 1 {
            Some(serde_json::from_value(self.gh_json(&[
                "api",
                &format!(
                    "repos/{}/actions/runs/{}/attempts/{}",
                    repo.slug(),
                    run.id,
                    run.run_attempt - 1
                ),
            ])?)?)
        } else {
            None
        };
        if flaky
            && !previous
                .as_ref()
                .is_some_and(|run| run.conclusion.as_deref() == Some("failure"))
        {
            return Ok(None);
        }
        let failed = self.failed_push_jobs(
            repo,
            if flaky {
                previous.as_ref().unwrap()
            } else {
                &run
            },
        )?;
        Ok(Some(BaseCiEvidence {
            base: base.to_string(),
            run,
            failed,
            previous,
        }))
    }

    fn failed_push_jobs(&self, repo: &Repo, run: &PushRun) -> Result<Vec<Job>> {
        let jobs = self.gh_api_items(
            &format!(
                "repos/{}/actions/runs/{}/attempts/{}/jobs?per_page=100",
                repo.slug(),
                run.id,
                run.run_attempt
            ),
            "jobs",
        )?;
        let jobs: Vec<Job> = jobs
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()?;
        Ok(jobs
            .into_iter()
            .filter(|job| {
                matches!(
                    job.conclusion.as_deref(),
                    Some("failure" | "timed_out" | "action_required")
                )
            })
            .collect())
    }

    fn ci_report(&self, repo: &Repo, failure: &BaseCiEvidence) -> Result<CiReport> {
        let BaseCiEvidence {
            base,
            run,
            failed,
            previous,
        } = failure;
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
        let mut body = if failure.flaky() {
            format!(
                "The same commit passed on re-run of the latest push CI on `{base}`. The recorded failure is the evidence; fix the test's dependence on timing or scheduling, or the race it exposes, without requiring reproduction on demand.\n\nFailing commit: `{}`\nFailed jobs: {names}\n\n{}\n",
                run.head_sha,
                failure.failed_attempt_marker(),
            )
        } else {
            format!(
                "The latest push CI on `{base}` failed.\n\nFailing commit: `{}`\nRun: {}\n\nFailed jobs: {names}\n",
                run.head_sha, run.html_url,
            )
        };
        let mut tests = Vec::new();
        if let Some(previous) = previous {
            let jobs = self.failed_push_jobs(repo, previous)?;
            body.push_str(&self.push_attempt_evidence(repo, previous, &jobs, &mut tests)?);
        }
        body.push_str(&self.push_attempt_evidence(
            repo,
            run,
            if failure.flaky() { &[] } else { failed },
            &mut tests,
        )?);
        body.push_str(&format!(
            "\n<!-- thirdshift:base-ci-commit {} -->\n",
            run.head_sha
        ));
        let prefix = if failure.flaky() {
            FLAKY_PREFIX
        } else {
            TITLE_PREFIX
        };
        Ok(CiReport {
            title: format!("{prefix}{base}: {names}"),
            body,
            tests,
        })
    }

    fn push_attempt_evidence(
        &self,
        repo: &Repo,
        run: &PushRun,
        failed: &[Job],
        tests: &mut Vec<String>,
    ) -> Result<String> {
        let mut body = format!(
            "\nAttempt {} ({}): {}/attempts/{}\n",
            run.run_attempt,
            run.conclusion.as_deref().unwrap_or("unfinished"),
            run.html_url,
            run.run_attempt
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
                        || line.contains("✖")
                        || line.contains("not ok ")
                })
                .take(50)
            {
                found = true;
                if let Some(test) = failing_test_name(line).filter(|name| !name.is_empty()) {
                    tests.push(test.to_string());
                }
                body.push_str("    ");
                body.extend(line.chars().take(500));
                body.push('\n');
            }
            if !found {
                body.push_str("No failing test names recognized in this job's logs.\n");
            }
        }
        Ok(body)
    }

    /// Caller holds the repository-wide CI filing reservation. Use the REST
    /// issue list rather than the eventually indexed search API, and inspect
    /// every page irrespective of labels or Base branch.
    pub(crate) fn handle_base_ci(
        &self,
        repo: &Repo,
        failure: BaseCiEvidence,
    ) -> Result<BaseCiAction> {
        if failure.run.run_attempt == 1 {
            if failure.failed.is_empty() {
                crate::progress::step(format!(
                    "warning: run {} has no failed jobs to re-run; falling back to CI-red filing",
                    failure.run.id
                ));
            } else {
                match self.gh(&[
                    "run",
                    "rerun",
                    &failure.run.id.to_string(),
                    "--failed",
                    "--repo",
                    &repo.slug(),
                ]) {
                    Ok(()) => {
                        crate::progress::step(format!(
                            "re-running failed jobs of run {}: {}",
                            failure.run.id,
                            failure
                                .failed
                                .iter()
                                .map(|job| job.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                        return Ok(BaseCiAction::RerunRequested);
                    }
                    Err(error) => crate::progress::warn(
                        &error,
                        format_args!(
                            "could not re-run Base branch CI; falling back to CI-red filing"
                        ),
                    ),
                }
            }
        }
        let report = self.ci_report(repo, &failure)?;
        let state = if failure.flaky() { "all" } else { "open" };
        let issues = self.gh_api_items(
            &format!(
                "repos/{}/issues?state={state}&sort=created&direction=desc&per_page=100",
                repo.slug()
            ),
            "",
        )?;
        // A title match on an outsider's issue still blocks a second CI-red
        // Ticket, as it did before evidence comments existed. It is not a
        // place to post that comment, and it is not a Flaky-test report.
        let mut untrusted_ci_red = false;
        for issue in issues {
            if issue.get("pull_request").is_some() {
                continue;
            }
            let title = issue["title"].as_str().context("issue has no title")?;
            let labels: Labels = issue["labels"]
                .as_array()
                .context("issue has no labels")?
                .iter()
                .map(|label| label["name"].as_str().context("label has no name"))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .collect();
            if !trusted_report(&issue) {
                if !failure.flaky() && title.starts_with(TITLE_PREFIX) && !labels.has(NEEDS_INFO) {
                    untrusted_ci_red = true;
                }
                continue;
            }
            if failure.flaky() {
                let body = issue["body"].as_str().unwrap_or("");
                let open = issue["state"]
                    .as_str()
                    .is_some_and(|state| state.eq_ignore_ascii_case("open"));
                if body.contains(&failure.failed_attempt_marker())
                    || (open
                        && (title.starts_with(FLAKY_PREFIX) || labels.has(FLAKY_TEST))
                        && report
                            .tests
                            .iter()
                            .any(|test| mentions_test(body, test) || mentions_test(title, test)))
                {
                    return Ok(BaseCiAction::Nothing);
                }
            } else if title.starts_with(TITLE_PREFIX) && !labels.has(NEEDS_INFO) {
                let commit = format!("Failing commit: `{}`", failure.run.head_sha);
                if issue["body"].as_str().unwrap_or("").contains(&commit) {
                    return Ok(BaseCiAction::Nothing);
                }
                let number = issue["number"].as_u64().context("issue has no number")?;
                let comments = self.gh_api_items(
                    &format!(
                        "repos/{}/issues/{number}/comments?per_page=100",
                        repo.slug()
                    ),
                    "",
                )?;
                if comments.iter().any(|comment| {
                    trusted_report(comment)
                        && comment["body"].as_str().unwrap_or("").contains(&commit)
                }) {
                    return Ok(BaseCiAction::Nothing);
                }
                let issue =
                    IssueUrl::parse(issue["html_url"].as_str().context("issue has no URL")?)?;
                self.comment_issue(&issue, &report.body)?;
                return Ok(BaseCiAction::Commented(issue));
            }
        }
        if untrusted_ci_red {
            return Ok(BaseCiAction::Nothing);
        }
        let mut labels = vec![Label::new("bug", "Something isn't working")];
        if failure.flaky() {
            labels.push(FLAKY_TEST);
        }
        labels.push(READY_FOR_AGENT);
        let issue = self.create_issue_in(repo, &report.title, &report.body, &labels)?;
        Ok(if failure.flaky() {
            BaseCiAction::FlakyFiled(issue)
        } else {
            BaseCiAction::CiRedFiled(issue)
        })
    }
}
