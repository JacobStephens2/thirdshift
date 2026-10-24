//! Dismiss an Inbox row from the CLI, or take a dismissal back, keeping
//! each dismissal beside its Command record as a Stop request is kept.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::reading::{Group, Observation, Snapshot};
use crate::config::UserConfig;

/// The stored kind of a dismissed Ready for review row.
pub(crate) const READY_KIND: &str = "Ready";

/// One Inbox row a dismissal covers, within its Command record. A dispatched
/// Spec run or Ticket run shares its outer Command's record, so the selector
/// says which of the record's rows this entry covers.
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct DismissedRow {
    pub selector: String,
    pub kind: String,
    pub who: String,
    pub at: String,
    #[serde(default)]
    pub from_console: bool,
}

impl DismissedRow {
    fn source(&self) -> &'static str {
        if self.from_console {
            "Console"
        } else {
            "CLI"
        }
    }

    pub(crate) fn view(&self) -> DismissalView {
        DismissalView {
            who: self.who.clone(),
            dismissed_at: self.at.clone(),
            source: self.source(),
            kind: self.kind.clone(),
        }
    }
}

/// A row's dismissal as `status --json` reports it: who, when, from where
/// and the kind it covers.
#[derive(Serialize)]
pub(crate) struct DismissalView {
    pub who: String,
    pub dismissed_at: String,
    pub source: &'static str,
    pub kind: String,
}

impl DismissalView {
    pub(crate) fn line(&self) -> String {
        format!(
            "dismissed by {} from the {} at {}",
            self.who, self.source, self.dismissed_at
        )
    }
}

/// Every dismissal kept beside one Command record.
#[derive(Default, Deserialize, Serialize)]
struct DismissalFile {
    #[serde(default)]
    rows: Vec<DismissedRow>,
}

fn dismissal_path(record: &Path) -> PathBuf {
    record.with_extension("dismiss.json")
}

fn read(record: &Path) -> DismissalFile {
    std::fs::read(dismissal_path(record))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// The dismissal covering this record's row, if one names its selector.
pub(crate) fn covering(record: &Path, selector: &str) -> Option<DismissedRow> {
    read(record)
        .rows
        .into_iter()
        .find(|row| row.selector == selector)
}

fn write(record: &Path, file: &DismissalFile) -> Result<()> {
    let path = dismissal_path(record);
    if file.rows.is_empty() {
        // Taking back the only dismissal already removed the file.
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let parent = path.parent().context("Command record has no directory")?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(&serde_json::to_vec(&file)?)?;
    staged
        .persist(&path)
        .map(|_| ())
        .with_context(|| {
            format!(
                "could not keep the dismissal beside {}",
                record.display()
            )
        })
}

/// One Inbox row holding the named Command: where to record and report the
/// dismissal, the kind a dismissal would cover, and every record-backed row
/// it stands for. A row read from history stands for no record.
struct DismissalTarget {
    repository: String,
    name: String,
    subject: String,
    group: Group,
    kind: String,
    members: Vec<(PathBuf, String)>,
}

fn row_for(snapshot: &Snapshot, input: &str, days: i64, undo: bool) -> Result<DismissalTarget> {
    let selected = snapshot.resolve(input)?;
    let record = match selected.observation {
        Observation::Record(command) => &command.record,
        Observation::History(_) => {
            bail!("cannot dismiss {input}: row has no Command record");
        }
    };
    let name = record.name.clone();
    let mut target = DismissalTarget {
        repository: record.repository.clone(),
        subject: match &record.issue {
            Some(issue) => format!("{} #{}", record.kind, issue.number),
            None => name.clone(),
        },
        name,
        group: Group::History,
        kind: String::new(),
        members: Vec::new(),
    };
    let selector = selected
        .observation
        .selector()
        .with_context(|| format!("cannot dismiss {input}: row has no Command record"))?;
    let found = snapshot
        .inbox(true, days)
        .into_iter()
        .find(|item| {
            item.command.selector().as_deref() == Some(selector.as_str())
                || item
                    .collapsed
                    .iter()
                    .any(|child| child.command.selector().as_deref() == Some(selector.as_str()))
        })
        .with_context(|| format!("cannot dismiss {input}: row is not in Needs you or Ready for review"))?;
    for item in std::iter::once(&found).chain(found.collapsed.iter()) {
        if let Observation::Record(command) = item.command
            && let Some(selector) = item.command.selector()
        {
            target.members.push((command.record_path.clone(), selector));
        }
    }
    // Taking a dismissal back works wherever the row now is, so only a
    // dismissal names a kind.
    let kind = match found.group {
        Group::NeedsYou => Some(
            found
                .needs_you_kind
                .map(|kind| kind.title().to_owned())
                .with_context(|| format!("cannot dismiss {input}: row has no Needs you kind"))?,
        ),
        Group::ReadyForReview => Some(READY_KIND.to_owned()),
        _ => None,
    };
    target.kind = match kind {
        Some(kind) => kind,
        None if undo => String::new(),
        None => {
            bail!(
                "cannot dismiss {input}: row is in {}, not Needs you or Ready for review",
                found.group.title()
            );
        }
    };
    target.group = found.group;
    Ok(target)
}

fn activity_write(config: &UserConfig, repository: &str, line: &str) -> Result<()> {
    let path = config.logs_dir.join(repository).join(super::activity::FILE);
    let mut file = super::activity::open(&path)?;
    writeln!(
        file,
        "{} {line}",
        chrono::Local::now().format(super::activity::TIME_FORMAT)
    )
    .with_context(|| format!("could not keep {}", path.display()))
}

pub fn dismiss(input: &str, undo: bool, json: bool) -> Result<()> {
    let user = std::process::Command::new("id")
        .arg("-un")
        .output()
        .context("could not identify the Unix user asking for dismissal")?;
    if !user.status.success() {
        bail!("could not identify the Unix user asking for dismissal");
    }
    let who = String::from_utf8(user.stdout)?.trim().to_owned();
    let at = chrono::Local::now().to_rfc3339();
    let config = UserConfig::load()?;
    let snapshot = Snapshot::load(&config.logs_dir, chrono::Local::now())?;
    let target = row_for(&snapshot, input, config.inbox_days, undo)?;
    if target.members.is_empty() {
        bail!("cannot dismiss {input}: row has no Command record");
    }
    if undo {
        let mut taken_back = false;
        for (record, selector) in &target.members {
            let mut file = read(record);
            let kept = file.rows.len();
            file.rows.retain(|row| row.selector != *selector);
            if file.rows.len() != kept {
                taken_back = true;
                write(record, &file)?;
            }
        }
        if !taken_back {
            bail!("cannot take back the dismissal of {input}: row is not dismissed");
        }
        activity_write(
            &config,
            &target.repository,
            &format!(
                "{} dismissal taken back from the CLI by {who}",
                target.subject
            ),
        )?;
        if json {
            println!(
                "{}",
                serde_json::json!({"version": 1, "name": target.name, "undone": true})
            );
        } else {
            println!("took back the dismissal of {}", target.name);
        }
        return Ok(());
    }
    for (record, selector) in &target.members {
        let mut file = read(record);
        file.rows.retain(|row| row.selector != *selector);
        file.rows.push(DismissedRow {
            selector: selector.clone(),
            kind: target.kind.clone(),
            who: who.clone(),
            at: at.clone(),
            from_console: false,
        });
        write(record, &file)?;
    }
    activity_write(
        &config,
        &target.repository,
        &format!("{} dismissed from the CLI by {who}", target.subject),
    )?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "version": 1,
                "name": target.name,
                "group": target.group,
                "dismissal": {
                    "who": who,
                    "dismissed_at": at,
                    "source": "CLI",
                    "kind": target.kind,
                },
            })
        );
    } else {
        println!("dismissed {} from {}", target.name, target.group.title());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Context;

    /// anyhow's `.context()` takes a plain `Display`, so brace placeholders
    /// never interpolate: the refusal contexts in `row_for` must keep the
    /// `with_context` plus `format!` construction pinned here. The old
    /// `.context("cannot dismiss {input}...")` form printed `{input}`
    /// literally on all three sites.
    #[test]
    fn refusal_contexts_interpolate_the_command() {
        let input = "widgets#7";
        let err = Err::<(), _>(anyhow::anyhow!("base"))
            .with_context(|| format!("cannot dismiss {input}: row has no Command record"))
            .unwrap_err();
        assert!(format!("{err:#}").contains("widgets#7"), "{err:#}");
    }
}
