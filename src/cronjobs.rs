//! Read this Unix account's thirdshift entries without changing its crontab.
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::UserConfig;
use crate::logs::pass_starts::{self, PassStart};

pub(crate) mod fleet;

#[derive(Serialize, Deserialize)]
pub(crate) struct CronJobs {
    pub jobs: Vec<CronJob>,
    pub pass_starts: Vec<PassStart>,
}

pub(crate) struct JobGroup<'a> {
    pub heading: &'a str,
    pub jobs: &'a [CronJob],
}

impl CronJobs {
    /// Inventories arrive in repository order; older inventories form one Other group.
    pub(crate) fn groups(&self) -> Vec<JobGroup<'_>> {
        self.jobs
            .chunk_by(|a, b| a.repository == b.repository)
            .map(|jobs| JobGroup {
                heading: jobs[0].repository.as_deref().unwrap_or("Other"),
                jobs,
            })
            .collect()
    }
}

#[derive(Serialize, Deserialize)]
pub struct CronJob {
    pub schedule: String,
    pub command: String,
    /// Absent on older inventories. A label with a control character is not a
    /// repository: an origin URL can contain a newline, and printing that as a
    /// heading would add a line to the schedule listing.
    #[serde(default, deserialize_with = "deserialize_repository")]
    pub repository: Option<String>,
    /// Owning-machine recognition only; absent on older inventories.
    #[serde(default, deserialize_with = "deserialize_command_kind")]
    pub command_kind: Option<CommandKind>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum CommandKind {
    #[serde(rename = "Pickup run")]
    Pickup,
    #[serde(rename = "Security run")]
    Security,
    #[serde(rename = "Architect run")]
    Architect,
    #[serde(rename = "Triage run")]
    Triage,
    #[serde(rename = "Fix run")]
    Fix,
    #[serde(rename = "Review run")]
    Review,
    #[serde(rename = "Custom process")]
    Process,
}

impl std::fmt::Display for CommandKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Pickup => "Pickup run",
            Self::Security => "Security run",
            Self::Architect => "Architect run",
            Self::Triage => "Triage run",
            Self::Fix => "Fix run",
            Self::Review => "Review run",
            Self::Process => "Custom process",
        };
        f.write_str(name)
    }
}

fn deserialize_command_kind<'de, D>(deserializer: D) -> Result<Option<CommandKind>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // A future owner's Command kind must not hide an otherwise readable entry.
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or(None))
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "inventory recognizes work Commands only; inspection and administration commands have unknown kind"
)]
fn command_kind(command: &str) -> Option<CommandKind> {
    use crate::args::{self, Command as Parsed};
    let words = crate::launch_directories::shell_words(shell_command(command))?;
    let invocation = match words.as_slice() {
        [cd, path, and, rest @ ..] if cd == "cd" && and == "&&" && !path.contains(['$', '`']) => {
            rest
        }
        rest => rest,
    };
    let (executable, arguments) = invocation.split_first()?;
    if executable.contains(['$', '`'])
        || std::path::Path::new(executable).file_name()? != "thirdshift"
    {
        return None;
    }
    if arguments
        .iter()
        .any(|word| matches!(word.as_str(), "&&" | "||" | ";" | "|" | "&"))
    {
        return None;
    }
    let arguments: Vec<_> = arguments
        .iter()
        .take_while(|word| {
            let operator = word.trim_start_matches(|c: char| c.is_ascii_digit());
            !matches!(
                operator,
                ">" | ">>" | "&>" | "&>>" | "<" | ">&" | "<&" | "#"
            )
        })
        .cloned()
        .collect();
    if arguments.is_empty() || arguments.iter().any(|word| word.contains(['$', '`'])) {
        return None;
    }
    Some(match args::parse(&arguments).ok()? {
        Parsed::Pickup(_) => CommandKind::Pickup,
        Parsed::Secure(_) => CommandKind::Security,
        Parsed::Architect(_) => CommandKind::Architect,
        Parsed::Triage(_) => CommandKind::Triage,
        Parsed::Fix(_) => CommandKind::Fix,
        Parsed::Review(_) => CommandKind::Review,
        Parsed::Process(args::ProcessAction::Run(_)) => CommandKind::Process,
        // An Issue URL can select a Spec run or Ticket run; labels are not read here.
        _ => return None,
    })
}

fn repository_label(repository: Option<String>) -> Option<String> {
    repository.filter(|label| {
        !label
            .chars()
            .any(|character| character.is_control() || matches!(character, '\u{2028}' | '\u{2029}'))
    })
}

fn deserialize_repository<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(repository_label(Option::<String>::deserialize(
        deserializer,
    )?))
}

pub(crate) fn load() -> Result<CronJobs> {
    let config = UserConfig::load()?;
    let home = crate::config::home()?;
    let mut jobs = jobs()?;
    for job in &mut jobs {
        job.repository = repository_label(
            crate::launch_directories::from_cron(&job.command, &home)
                .map(|directory| directory.repository),
        );
    }
    // Stable sorting retains crontab order and duplicates within each repository.
    jobs.sort_by(|a, b| {
        (a.repository.is_none(), &a.repository).cmp(&(b.repository.is_none(), &b.repository))
    });
    Ok(CronJobs {
        jobs,
        pass_starts: pass_starts::load(&config.logs_dir)?,
    })
}

pub(crate) fn jobs() -> Result<Vec<CronJob>> {
    let output = Command::new("crontab")
        .arg("-l")
        .env("LC_ALL", "C")
        .output()
        .context("can't read cron jobs: could not run crontab -l")?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        let diagnostic = error
            .trim()
            .strip_prefix("crontab: ")
            .unwrap_or(error.trim());
        if output.status.code() == Some(1) && diagnostic.starts_with("no crontab for ") {
            return Ok(Vec::new());
        }
        bail!(
            "can't read cron jobs (crontab -l {}): {}",
            output.status,
            error.trim()
        );
    }
    let crontab = String::from_utf8(output.stdout).context("crontab is not UTF-8")?;
    Ok(crontab.lines().filter_map(entry).collect())
}

pub fn show(json: bool) -> Result<String> {
    let inventory = load()?;
    if json {
        return Ok(serde_json::to_string_pretty(&inventory)?);
    }
    let mut lines = Vec::new();
    if inventory.jobs.is_empty() {
        lines.push("No thirdshift cron jobs in this account's crontab.".into());
    }
    for group in inventory.groups() {
        lines.push(group.heading.into());
        lines.extend(
            group
                .jobs
                .iter()
                .map(|job| format!("{}  {}", job.schedule, job.command)),
        );
    }
    lines.extend(inventory.pass_starts.iter().map(PassStart::line));
    if inventory.pass_starts.is_empty() {
        lines.push("No Pass start times recorded on this host.".into());
    }
    Ok(lines.join("\n"))
}

fn entry(line: &str) -> Option<CronJob> {
    let mut command = line.trim();
    // User crontabs have five schedule fields, or one @-schedule field.
    // Requiring a schedule excludes comments and environment assignments.
    let fields = if command.starts_with('@') {
        1
    } else if command.starts_with(|c: char| c.is_ascii_digit() || c == '*') {
        5
    } else {
        return None;
    };
    let mut schedule = Vec::new();
    for _ in 0..fields {
        let end = command.find(char::is_whitespace)?;
        schedule.push(&command[..end]);
        command = command[end..].trim_start();
    }
    // Keep stdin text for display, but don't match it as a command.
    let mentions_thirdshift = shell_command(command)
        .split(|c: char| c.is_whitespace() || "'\";&|()<>".contains(c))
        .any(|word| word.rsplit('/').next() == Some("thirdshift"));
    mentions_thirdshift.then(|| CronJob {
        schedule: schedule.join(" "),
        command: command.into(),
        repository: None,
        command_kind: command_kind(command),
    })
}

/// Cron sends everything after its first unescaped % to stdin, even in quotes.
pub(crate) fn shell_command(command: &str) -> &str {
    let mut escaped = false;
    let command_end = command
        .char_indices()
        .find_map(|(index, character)| {
            let separator = character == '%' && !escaped;
            escaped = character == '\\' && !escaped;
            separator.then_some(index)
        })
        .unwrap_or(command.len());
    &command[..command_end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_label_with_a_newline_is_unresolved() {
        let forged: CronJob = serde_json::from_str(
            "{\"schedule\":\"@daily\",\"command\":\"thirdshift\",\"repository\":\"acme/widgets\\n@reboot echo forged\"}",
        )
        .unwrap();
        assert_eq!(forged.repository, None);
        let legacy: CronJob =
            serde_json::from_str("{\"schedule\":\"@daily\",\"command\":\"thirdshift\"}").unwrap();
        assert_eq!(legacy.repository, None);
        let kept: CronJob = serde_json::from_str(
            "{\"schedule\":\"@daily\",\"command\":\"thirdshift\",\"repository\":\"acme/widgets\"}",
        )
        .unwrap();
        assert_eq!(kept.repository.as_deref(), Some("acme/widgets"));
    }
}
