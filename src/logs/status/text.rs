//! Format the CLI view of the checked Inbox.
use crate::logs::reading::Group;

use super::{
    ready,
    rows::{Cli, CliCommand, Row},
};

fn print_command(command: CliCommand<'_>, state: &str, indent: &str, machine: Option<&str>) {
    match command.facts {
        Cli::Reservation(row) => {
            println!(
                "{indent}{}{} · {state} · {}",
                machine
                    .map(|machine| format!("[{machine}] "))
                    .unwrap_or_default(),
                row.repository,
                row.reference
            );
            let held = chrono::DateTime::parse_from_rfc3339(&row.started_at)
                .map(|time| (chrono::Utc::now() - time.with_timezone(&chrono::Utc)).num_seconds())
                .unwrap_or_default();
            println!(
                "{indent}  {} · Held for {}",
                row.owner_description,
                super::age(held)
            );
            println!(
                "{indent}  {}\n{indent}  {}",
                row.blocking, row.release_command
            );
        }
        Cli::Record(record) => {
            let stage = super::stage_details(
                record.facts.outcome.as_deref(),
                &record.view.state,
                record.view.stage.as_ref(),
                record.view.health.as_ref(),
                &record.facts.waiting_for,
            );
            super::print_command_line(
                &record.facts,
                &stage,
                record.view.account.as_deref(),
                state,
                indent,
                machine,
            );
            super::print_tickets(&record.facts, &format!("{indent}  "));
            super::print_pause(&record.facts, indent);
            for child in command.children() {
                print_command(child, child.facts.state(), &format!("{indent}  "), machine);
            }
        }
        Cli::History(history) => {
            let account = history
                .account
                .as_ref()
                .map(|account| format!("[{account}] "))
                .unwrap_or_default();
            let issue = history
                .issue
                .as_ref()
                .map(|issue| format!("#{}", issue.number))
                .unwrap_or_default();
            println!(
                "{indent}{}{account}{}{issue} {} · {} → {} · {}",
                machine
                    .map(|machine| format!("[{machine}] "))
                    .unwrap_or_default(),
                history.repository,
                crate::logs::display_name(&history.kind),
                history.started_at,
                history.ended_at.as_deref().unwrap_or("unknown end"),
                history.outcome.as_deref().unwrap_or("unknown outcome")
            );
        }
    }
}

impl Row {
    pub fn print(&self, tags: bool) {
        self.print_indented(tags, "");
    }

    fn print_indented(&self, tags: bool, indent: &str) {
        let meta = &self.metadata;
        let state = meta.group.state_word(
            meta.needs_you_kind,
            meta.stopping,
            self.command().facts.state(),
        );
        print_command(
            self.command(),
            state,
            indent,
            tags.then_some(meta.machine.as_str()),
        );
        if meta.group == Group::ReadyForReview {
            println!("{indent}  {}", meta.github_checks.ci().label());
            if meta.github_checks.has_merge_conflict() {
                println!("{indent}  Merge conflict");
            }
            if let Some(note) = &meta.github_checks.stale_ci {
                println!("{indent}  {note}");
            }
            if self.stale_github {
                println!("{indent}  Stale GitHub facts");
            }
        }
        if let Some(departure) = &meta.departure {
            println!("{indent}  {}", departure.line());
        }
        if let Some(dismissal) = &meta.dismissal {
            println!("{indent}  {}", dismissal.line());
        }
        if meta.count > 1
            && let Cli::Record(record) = self.command().facts
        {
            println!(
                "{indent}  {} Commands with this cause: {}",
                meta.count,
                record.facts.cause.as_deref().unwrap_or_default()
            );
        }
    }

    pub fn print_reference(&self, tags: bool) {
        if let Cli::Record(record) = self.command().facts {
            let name = record
                .facts
                .issue
                .as_ref()
                .map(|issue| format!("{}#{}", record.facts.repository, issue.number))
                .unwrap_or_else(|| record.facts.name.clone());
            let account = record
                .view
                .account
                .as_ref()
                .map(|account| format!("[{account}] "))
                .unwrap_or_default();
            println!(
                "{}{account}{name} · {} · see Needs you",
                if tags {
                    format!("[{}] ", self.metadata.machine)
                } else {
                    String::new()
                },
                self.metadata
                    .needs_you_kind
                    .map(|kind| kind.title())
                    .unwrap_or("Needs you")
            );
        }
    }
}

pub(super) fn print_ready(rows: Vec<&Row>, tags: bool) {
    for group in ready::group(rows, |row| row.ready_member()) {
        let Some(pr) = group.pull_request else {
            group.members[0].print(tags);
            continue;
        };
        let newest = group.members[0];
        // The title follows the validated URL on this line. Document text can
        // carry a carriage return or escape, which a terminal would use to
        // erase that URL, so render it as visible text first.
        let title = newest.ready_title().unwrap_or("(title unavailable)");
        println!(
            "{} · {} · {} · {} Command{}",
            pr.label,
            pr.url,
            crate::cronjobs::report_line(title),
            group.members.len(),
            if group.members.len() == 1 { "" } else { "s" },
        );
        for member in group.members {
            member.print_indented(true, "  ");
            if let Cli::Record(record) = member.command().facts {
                println!(
                    "    Command: {}",
                    crate::cronjobs::report_line(member.command().selector())
                );
                println!(
                    "    Started {} · Ended {}",
                    crate::cronjobs::report_line(&record.facts.started_at),
                    crate::cronjobs::report_line(
                        record.facts.ended_at.as_deref().unwrap_or("unknown")
                    )
                );
            }
        }
    }
}
