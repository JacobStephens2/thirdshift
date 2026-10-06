//! Progress lines on stderr: thirdshift's own steps, and a session's
//! stream, condensed by its Harness adapter to one short
//! line per notable event. Each line is stamped with the local time it was
//! printed, so a stalled Run can be told from a busy one. Each is kept in the
//! Command log too, if the command keeps one.

use std::fmt::Display;

use chrono::Local;

use crate::logs;

/// The longest detail a session line shows before it is cut short.
const MAX_DETAIL: usize = 100;

/// What every progress line starts with.
const PREFIX: &str = "thirdshift: ";

/// The local time that follows the prefix, as `HH:MM:SS`.
const STAMP: &str = "%H:%M:%S";

/// Print one of thirdshift's own steps, stamped with the time now.
pub fn step(message: impl Display) {
    logs::eprint(&stamped(message));
}

/// Report `error`, then a warning saying what to do about it by hand.
pub fn warn(error: &anyhow::Error, warning: std::fmt::Arguments) {
    step(format_args!("{error:#}"));
    step(format_args!("warning: {warning}"));
}

/// The progress line [`step`] prints for `message`.
pub fn stamped(message: impl Display) -> String {
    progress_line(Local::now().format(STAMP), message)
}

/// What a line from the stderr of a child thirdshift is.
#[derive(Debug, PartialEq, Eq)]
pub enum ChildLine<'a> {
    /// One of the child's own progress lines, with its message: the line
    /// without prefix or time.
    Own(&'a str),
    /// A progress line the child relayed from a child of its own.
    Relayed,
    /// A line that continues the one before it, as the later lines of an
    /// error of several lines do.
    Continuation,
}

impl<'a> ChildLine<'a> {
    /// What `line` is: a progress line if it starts with their prefix,
    /// relayed if its message then starts as [`relayed`] starts one.
    pub fn of(line: &'a str) -> Self {
        let Some(rest) = line.strip_prefix(PREFIX) else {
            return ChildLine::Continuation;
        };
        let message = split_stamp(rest).map_or(rest, |(_, message)| message);
        let from_its_own_child = message
            .strip_prefix('#')
            .and_then(|rest| rest.split_once(": "))
            .is_some_and(|(number, _)| {
                !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
            });
        if from_its_own_child {
            ChildLine::Relayed
        } else {
            ChildLine::Own(message)
        }
    }
}

/// Print `line`, from the stderr of the child thirdshift running issue
/// `number`, as [`relayed`] gives it, and return what the line is.
pub fn relay(number: u64, line: &str) -> ChildLine<'_> {
    logs::eprint(&relayed(number, line));
    ChildLine::of(line)
}

/// The progress line that relays `line`, from the stderr of the child
/// thirdshift running issue `number`: its message after `#<number>: `,
/// keeping the time the child stamped it with (or stamped now, if it has
/// none).
pub fn relayed(number: u64, line: &str) -> String {
    let rest = line.strip_prefix(PREFIX).unwrap_or(line);
    match split_stamp(rest) {
        Some((time, message)) => progress_line(time, format_args!("#{number}: {message}")),
        None => stamped(format_args!("#{number}: {rest}")),
    }
}

/// The progress line for `message`, stamped with `time`.
fn progress_line(time: impl Display, message: impl Display) -> String {
    format!("{PREFIX}{time} {message}")
}

/// A line without its prefix split into its `HH:MM:SS` stamp and message,
/// if it starts with one.
fn split_stamp(line: &str) -> Option<(&str, &str)> {
    let (time, message) = line.split_at_checked(8)?;
    let message = message.strip_prefix(' ')?;
    let is_stamp = time.bytes().enumerate().all(|(at, byte)| match at {
        2 | 5 => byte == b':',
        _ => byte.is_ascii_digit(),
    });
    is_stamp.then_some((time, message))
}

/// Commits and pushes by name; any other command by its first line.
pub(crate) fn bash(command: &str) -> String {
    let actions: Vec<&str> = ["commit", "push"]
        .into_iter()
        .filter(|subcommand| runs_git(command, subcommand))
        .collect();
    if actions.is_empty() {
        format!("$ {}", shorten(command.lines().next().unwrap_or("")))
    } else {
        actions.join(" and ")
    }
}

/// Does one of the commands chained in `command` start `git <subcommand>`?
fn runs_git(command: &str, subcommand: &str) -> bool {
    command.split(['\n', ';', '&', '|']).any(|part| {
        let mut words = part.split_whitespace();
        words.next() == Some("git") && words.next() == Some(subcommand)
    })
}

pub(crate) fn shorten(text: &str) -> String {
    match text.char_indices().nth(MAX_DETAIL) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamped_line_splits_into_its_time_and_message() {
        assert_eq!(
            split_stamp("12:14:49 pushing issue-7"),
            Some(("12:14:49", "pushing issue-7"))
        );
    }

    #[test]
    fn a_line_without_a_stamp_does_not_split() {
        for line in [
            "pushing issue-7",
            "12:14:49",
            "12:14:49pushing",
            "12-14-49 pushing",
            "1:14:49 pushing",
            "#21: 12:14:49 pushing",
            "é2:14:49 pushing",
        ] {
            assert_eq!(split_stamp(line), None, "{line}");
        }
    }

    #[test]
    fn a_relayed_line_keeps_its_time_under_the_issues_number() {
        assert_eq!(
            relayed(21, "thirdshift: 12:14:49 could not push issue-21"),
            "thirdshift: 12:14:49 #21: could not push issue-21"
        );
    }

    #[test]
    fn a_relayed_line_with_no_time_is_stamped_now() {
        let line = relayed(21, "hint: fetch first");
        let (_, message) = split_stamp(line.strip_prefix(PREFIX).unwrap()).unwrap();
        assert_eq!(message, "#21: hint: fetch first");
    }

    #[test]
    fn a_childs_line_that_starts_with_the_prefix_is_its_own_progress_line() {
        assert_eq!(
            ChildLine::of("thirdshift: 12:14:49 could not push issue-21"),
            ChildLine::Own("could not push issue-21")
        );
        assert_eq!(
            ChildLine::of(&stamped("#21 failed: claude exited 1")),
            ChildLine::Own("#21 failed: claude exited 1")
        );
    }

    #[test]
    fn a_childs_line_without_the_prefix_continues_the_one_before() {
        for line in [
            "hint: fetch first",
            "12:14:49 thirdshift: pushing",
            " thirdshift: 12:14:49 pushing",
            "#8: claude exited 1",
            "",
        ] {
            assert_eq!(ChildLine::of(line), ChildLine::Continuation, "{line}");
        }
    }

    #[test]
    fn a_line_a_child_relayed_is_not_its_own_progress_line() {
        for line in [
            "thirdshift: 12:14:49 claude exited 1",
            "hint: fetch first",
            "thirdshift: 12:14:49 #9: relayed before",
        ] {
            assert_eq!(
                ChildLine::of(&relayed(8, line)),
                ChildLine::Relayed,
                "{line}"
            );
        }
    }
}
