//! The Security audit session and its validated report artifacts. Kept under
//! the repository's logs, independently of the disposable audit checkout.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::git::Git;
use crate::github::{DraftAdvisory, Package};
use crate::harness::Choice;
use crate::issue::Repo;
use crate::logs;
use crate::process::{self, Control, Interruption};
use crate::progress;
use crate::prompt;
use crate::session::{Logs, Sessions};
use crate::skills;
use crate::worktree::ReviewWorktree;

pub struct Audited {
    pub findings: Vec<DraftAdvisory>,
}

pub fn check_node() -> Result<()> {
    node(&["--version"])
        .context("Node.js is required for the Security audit validators; install node on PATH")?;
    Ok(())
}

pub fn run(
    launch: &Git,
    repo: &Repo,
    base: &str,
    harness: &Choice,
) -> (Result<Audited>, Option<PathBuf>) {
    let prepared = (|| -> Result<_> {
        let worktree = ReviewWorktree::create(launch, &repo.name, base)?;
        let commit = Git::new(worktree.path()).run(&["rev-parse", "HEAD"])?;
        let root = logs::root(repo).join("audits");
        fs::create_dir_all(&root)
            .with_context(|| format!("could not create {}", root.display()))?;
        let output = tempfile::Builder::new()
            .prefix(&format!("{}-", logs::stamp()))
            .tempdir_in(&root)?
            .keep();
        let threat_model = [
            "SECURITY.md",
            "THREAT_MODEL.md",
            "THREAT-MODEL.md",
            "docs/threat-model.md",
            "docs/THREAT_MODEL.md",
        ]
        .into_iter()
        .find(|file| worktree.path().join(file).is_file());
        let prompt = prompt::security_audit(base, &commit, &root, &output, threat_model);
        Ok((worktree, output, prompt, commit))
    })();
    let (worktree, output, prompt, commit) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => return (Err(error), None),
    };
    let logs = Logs::of_security_run(repo);
    Sessions::within(&logs, worktree.path(), harness, |sessions| {
        progress::step(format!("starting the Security audit of {base}"));
        let message = sessions.run_to_final_message("security-audit", &prompt)?;
        let line = message.as_deref().and_then(|message| {
            message
                .lines()
                .map(str::trim)
                .rfind(|line| !line.is_empty())
        });
        match line {
            Some(prompt::AUDIT_COMPLETE_LINE) => {}
            Some(prompt::AUDIT_INCOMPLETE_LINE) => bail!(
                "the Security audit ended incomplete; artifacts: {}",
                output.display()
            ),
            _ => bail!("the Security audit ended without the final line its prompt asks for"),
        }
        let metadata: Value = serde_json::from_slice(
            &fs::read(output.join("run-metadata.json"))
                .context("could not read the Security audit's run-metadata.json")?,
        )
        .context("the Security audit's run-metadata.json is invalid JSON")?;
        if metadata["run_status"] != "complete" {
            bail!(
                "the Security audit's run-metadata.json does not mark the audit complete; artifacts: {}",
                output.display()
            );
        }
        let skill = skills::written_out()?.join("thirdshift-security-audit");
        for (validator, report) in [
            ("validate-findings.cjs", "findings.json"),
            ("validate-coverage-ledger.cjs", "coverage-ledger.json"),
        ] {
            node(&[
                skill
                    .join(validator)
                    .to_str()
                    .context("non-UTF-8 skill path")?,
                output
                    .join(report)
                    .to_str()
                    .context("non-UTF-8 report path")?,
            ])
            .with_context(|| {
                format!("the Security audit report failed the skill's validator {validator}")
            })?;
        }
        let findings: Vec<Value> =
            serde_json::from_slice(&fs::read(output.join("findings.json"))?)?;
        let package = package_in(worktree.path());
        let findings = findings.into_iter().filter(|finding| finding["verdict"] != "rejected").map(|finding| {
            let fingerprint = finding["fingerprint"].as_str().context("finding has no fingerprint")?.to_string();
            Ok(DraftAdvisory {
                summary: finding["title"].as_str().context("finding has no title")?.to_string(),
                description: format!("Found by thirdshift's Security run.\n\nFingerprint: `{fingerprint}`\nAudited commit: `{commit}`\n\n{}\n\n```json\n{}\n```\n", finding["description"].as_str().unwrap_or_default(), serde_json::to_string_pretty(&finding)?),
                fingerprint,
                package: package.clone(),
            })
        }).collect::<Result<_>>()?;
        Ok(Audited { findings })
    })
}

fn node(args: &[&str]) -> Result<()> {
    let output = process::output(
        Command::new("node").args(args),
        None,
        Control {
            name: "Node.js",
            interruption: Interruption::Ordinary,
            stop: &|child| process::stop(child, &[libc::SIGTERM]),
        },
    )?;
    if !output.status.success() {
        // The validators' diagnostics can contain private evidence. Leave
        // details in the audit artifacts, out of progress and Command logs.
        bail!("Node.js exited {}", output.status);
    }
    Ok(())
}

/// Manifest identity only: no affected or patched versions are inferred.
fn package_in(worktree: &Path) -> Package {
    let rust = fs::read_to_string(worktree.join("Cargo.toml"))
        .ok()
        .and_then(|text| toml::from_str::<toml::Value>(&text).ok())
        .and_then(|manifest| {
            manifest
                .get("package")?
                .get("name")?
                .as_str()
                .map(String::from)
        });
    if let Some(name) = rust {
        return Package {
            ecosystem: "rust".to_string(),
            name: Some(name),
        };
    }
    let npm = fs::read(worktree.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_slice::<Value>(&text).ok())
        .and_then(|manifest| manifest["name"].as_str().map(String::from));
    match npm {
        Some(name) => Package {
            ecosystem: "npm".to_string(),
            name: Some(name),
        },
        None => Package {
            ecosystem: "other".to_string(),
            name: None,
        },
    }
}
