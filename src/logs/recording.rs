//! One Command's owned recording implementation. Production and tests use
//! these same operations; only the effects adapter differs.

use std::collections::VecDeque;
use std::fmt::Display;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use super::activity::{self, Kind};
use super::command_record::{
    CommandRecord, CommandTree, Stage, StageName, Suspension, TicketRecord,
};
use super::effects::{Outside, Stream};
use super::owner::{self, Owner};
use super::{Begin, CommandKind, Pass, Work, root_under};
use crate::config::UserConfig;
use crate::harness::Choice;
use crate::issue::Repo;
use crate::progress;

const STAMP_FORMAT: &str = "%Y%m%dT%H%M%S%z";

pub(super) struct Record<O: Outside> {
    outside: O,
    stamp: Option<String>,
    owner: Option<Owner>,
    owner_error: Option<String>,
    logs_dir: Option<PathBuf>,
    otlp_endpoint: Option<String>,
    child: bool,
    command: CommandKind,
    started: bool,
    command_log: Log<O::CommandLog>,
    activity_started: Option<Started>,
    activity_warned: bool,
    /// The User config's warnings, until the Activity log of the command's
    /// repository has them.
    config_warnings: Vec<String>,
    command_record: Option<(PathBuf, CommandTree)>,
    preflight_repo: Option<Repo>,
    preflight_issue: Option<crate::issue::IssueUrl>,
    quota_record: bool,
    custom_process: Option<crate::processes::Identity>,
    parent_record: Option<PathBuf>,
    parent_dispatch: Option<String>,
    record_warned: bool,
    unaddressed_findings: Option<crate::pull_request::UnaddressedFindings>,
    clocks: Option<Clocks>,
}

struct Clocks {
    wall: chrono::DateTime<chrono::FixedOffset>,
    monotonic: Duration,
}

#[derive(Clone, Copy)]
enum LogActivity {
    Work,
    Suspension,
}

enum Log<W> {
    NotKept,
    Held {
        lines: Vec<(Stream, String)>,
        shown: bool,
        /// The indexes of `lines` already on the terminal while the rest
        /// were held, which showing them leaves out.
        shown_early: Vec<usize>,
    },
    Kept {
        path: PathBuf,
        writer: W,
    },
    Failed {
        path: Option<PathBuf>,
    },
}

struct Started {
    root: PathBuf,
    kind: Kind,
    issue: Option<u64>,
    process: Option<String>,
}

enum History<'a> {
    Kind(Kind),
    ProcessSkip(&'a str),
}

impl Started {
    fn subject(&self) -> String {
        if let Some(name) = &self.process {
            return format!("Process {name}");
        }
        match self.issue {
            Some(number) => format!("{} #{number}", self.kind),
            None => self.kind.to_string(),
        }
    }
}

impl<O: Outside> Record<O> {
    pub(super) fn suspended_seconds(&self) -> u64 {
        self.command_record.as_ref().map_or(0, |(_, tree)| {
            tree.command.stages.iter().fold(0, |total, stage| {
                total.saturating_add(stage.suspended_seconds)
            })
        })
    }

    /// Called by the periodic sampler and before transitions. Each reading
    /// replaces the baseline, so smaller clock corrections never accumulate.
    pub(super) fn check_suspension(&mut self) {
        let clocks = Clocks {
            wall: self.outside.now(),
            monotonic: self.outside.monotonic(),
        };
        let Some(previous) = self.clocks.replace(clocks) else {
            return;
        };
        let current = self.clocks.as_ref().unwrap();
        let Ok(wall) = (current.wall - previous.wall).to_std() else {
            return;
        };
        let suspended = wall.saturating_sub(current.monotonic.saturating_sub(previous.monotonic));
        if suspended < Duration::from_secs(60) {
            return;
        }
        let Some((_, tree)) = &mut self.command_record else {
            return;
        };
        if tree.command.outcome.is_some() {
            return;
        }
        for facts in std::iter::once(&mut tree.command).chain(&mut tree.dispatched) {
            if facts.outcome.is_none()
                && let Some(stage) = facts.stages.last_mut()
            {
                stage.suspended_seconds =
                    stage.suspended_seconds.saturating_add(suspended.as_secs());
                stage.suspensions.push(Suspension {
                    resumed_at: current.wall,
                    seconds: suspended.as_secs(),
                });
            }
        }
        self.write_record();
        // Child Ticket runs retain their own stage facts. Their owning
        // Command emits the one log line, rather than relaying duplicates.
        if !self.child {
            let line = self.progress_line(format_args!(
                "host was suspended {}",
                super::status::age(suspended.as_secs() as i64)
            ));
            self.output(Stream::Stderr, line, LogActivity::Suspension);
        }
    }

    pub(super) fn new(outside: O) -> Self {
        Self {
            outside,
            stamp: None,
            owner: None,
            owner_error: None,
            logs_dir: None,
            otlp_endpoint: None,
            child: false,
            command: CommandKind::Issue,
            started: false,
            command_log: Log::NotKept,
            activity_started: None,
            activity_warned: false,
            config_warnings: Vec::new(),
            command_record: None,
            preflight_repo: None,
            preflight_issue: None,
            quota_record: false,
            custom_process: None,
            parent_record: None,
            parent_dispatch: None,
            record_warned: false,
            unaddressed_findings: None,
            clocks: None,
        }
    }

    pub(super) fn begin(&mut self, begin: Begin) {
        if let Begin::Run(issue) = &begin {
            self.preflight_issue((*issue).clone());
        }
        self.command = match &begin {
            Begin::Run(_) => CommandKind::Issue,
            Begin::ChildRun(_, command) => *command,
            Begin::ArchitectRun => CommandKind::Architect,
            Begin::PickupRun => CommandKind::Pickup,
            Begin::SecurityRun => CommandKind::Secure,
            Begin::ScoringRun => CommandKind::Scoring,
            Begin::ReviewRun { .. } => CommandKind::Review,
            Begin::FixRun { .. } => CommandKind::Fix,
            Begin::ReleaseRun { .. } => CommandKind::Release,
            Begin::TriageRun { .. } => CommandKind::Triage,
            Begin::Experiment => CommandKind::Experiment,
            Begin::Process { .. } => CommandKind::Process,
        };
        let (starting, quiet) = match begin {
            Begin::ChildRun(stamp, _) => {
                self.child = true;
                self.stamp.get_or_insert_with(|| stamp.to_string());
                return;
            }
            Begin::Run(issue) => (format!("starting on {}", issue.url), false),
            Begin::ArchitectRun => ("Architect run starting".into(), true),
            Begin::PickupRun => ("Pickup run starting".into(), true),
            Begin::SecurityRun => ("Security run starting".into(), true),
            Begin::ScoringRun => ("Scoring run starting".into(), false),
            Begin::ReviewRun { pass } => ("Review run starting".into(), pass),
            Begin::FixRun { pass } => ("Fix run starting".into(), pass),
            Begin::ReleaseRun { pass } => ("Release run starting".into(), pass),
            Begin::TriageRun { pass } => ("Triage run starting".into(), pass),
            Begin::Experiment => ("Experiment starting".into(), false),
            Begin::Process { name, pass } => (format!("Process {name} starting"), pass),
        };
        let now = self.outside.now();
        self.stamp
            .get_or_insert_with(|| now.format(STAMP_FORMAT).to_string());
        self.command_log = Log::Held {
            lines: Vec::new(),
            shown: !quiet,
            shown_early: Vec::new(),
        };
        self.step(format_args!("{starting}, {}", now.format("%Y-%m-%d %z")));
    }

    pub(super) fn configured(&mut self, config: &UserConfig) {
        self.logs_dir = Some(config.logs_dir.clone());
        self.otlp_endpoint = config.otlp_endpoint.clone();
        if !config.quiet_skips && self.command != CommandKind::Release {
            self.show_held();
        }
        if !self.child {
            for warning in &config.warnings {
                self.step_shown(format_args!("warning: {warning}"));
            }
            self.config_warnings = config.warnings.clone();
        }
    }

    pub(super) fn logs_directory(&self) -> Option<PathBuf> {
        self.logs_dir.clone()
    }

    pub(super) fn preflight_repo(&mut self, repo: Option<Repo>) {
        if self.preflight_repo.is_none() {
            self.preflight_repo = repo;
        }
    }

    pub(super) fn needs_quota_repo(&self) -> bool {
        self.command_record.is_none() && self.preflight_repo.is_none()
    }

    pub(super) fn preflight_issue(&mut self, issue: crate::issue::IssueUrl) {
        self.preflight_repo = Some(issue.repo());
        self.preflight_issue = Some(issue);
    }

    /// Hand on the progress line `message`, to the terminal even while a
    /// quiet pass holds its other lines, and to the Command log as any
    /// other.
    fn step_shown(&mut self, message: impl Display) {
        let line = self.progress_line(message);
        if let Log::Held {
            lines,
            shown: false,
            shown_early,
        } = &mut self.command_log
        {
            let _ = self.outside.emit(Stream::Stderr, &line);
            shown_early.push(lines.len());
            lines.push((Stream::Stderr, line));
        } else {
            self.output(Stream::Stderr, line, LogActivity::Work);
        }
    }

    /// Record each of the User config's warnings in the Activity log under
    /// `root`, once: each is written unless it is already the log's last
    /// User config line, so it is recorded once per change, not once per
    /// command.
    fn record_config_warnings(&mut self, root: &Path) {
        for warning in std::mem::take(&mut self.config_warnings) {
            self.activity_write(
                root,
                &format!("{} warning: {warning}", Kind::UserConfig),
                Some(Kind::UserConfig),
            );
        }
    }

    pub(super) fn pass_fired(&mut self, repo: &Repo) {
        let pass = match self.command {
            CommandKind::Architect => Pass::Architect,
            CommandKind::Pickup => Pass::Pickup,
            CommandKind::Secure => Pass::Security,
            CommandKind::Review => Pass::Review,
            CommandKind::Fix => Pass::Fix,
            CommandKind::Triage => Pass::Triage,
            CommandKind::Release => Pass::Release,
            CommandKind::Issue
            | CommandKind::Scoring
            | CommandKind::Experiment
            | CommandKind::Process => return,
        };
        self.record_config_warnings(&self.root(repo));
        let path = super::pass_starts::path(&self.root(repo), pass);
        let time = self.outside.now().to_rfc3339();
        // One atomic replacement per kind also keeps competing launches from
        // truncating a reader's time or overwriting another kind's start.
        if let Err(error) = self.outside.replace_record(&path, time.as_bytes()) {
            self.show_held();
            self.step(format_args!(
                "warning: could not keep the Pass's last-fired time: {error:#}"
            ));
        }
    }

    pub(super) fn started(&mut self, work: Work, harness: &Choice) {
        let first = !self.started;
        self.started = true;
        if self.child && first {
            if self.parent_record.is_some() {
                self.start_record(work, harness);
            }
            return;
        }
        if !first || self.child {
            if !self.child
                && self.command != CommandKind::Issue
                && matches!(work, Work::Run(_) | Work::SpecRun(_))
                && let Some((_, tree)) = &mut self.command_record
                && let Some(dispatched) = tree.active_dispatch()
            {
                // The launch checks have now established whether the issue is
                // a Spec. The dispatch itself was kept before those checks.
                dispatched.kind = work.kind().to_string();
                dispatched.process = work.process();
                if let Some(stage) = tree.command.stages.last_mut()
                    && StageName::is_dispatch(&stage.name)
                {
                    stage.name = StageName::dispatched(work).name().into();
                }
                self.stage(work.initial_stage().unwrap(), None);
            }
            return;
        }
        let root = self.root(&work.repo());
        self.record_config_warnings(&root);
        let stamp = self.stamp();
        let began = matches!(self.command_log, Log::Held { .. });
        self.keep(&root, work, &stamp);
        if began || self.quota_record {
            self.start_record(work, harness);
        }
        self.step(format_args!("sessions run on {harness}"));
        let command_log = match self.command_log_path() {
            Some(path) => path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string(),
            None => "no Command log".into(),
        };
        let started = Started {
            root: root.clone(),
            kind: work.kind(),
            issue: work.issue(),
            process: match work {
                Work::Process { name, .. } => Some((*name).to_string()),
                Work::Starting { .. }
                | Work::Run(_)
                | Work::BaseFix(_)
                | Work::SpecRun(_)
                | Work::PickupRun(_)
                | Work::ArchitectRun(_)
                | Work::SecurityRun(_)
                | Work::ScoringRun(_)
                | Work::ReviewRun(_)
                | Work::FixRun(_)
                | Work::ReleaseRun(_)
                | Work::TriageRun { .. }
                | Work::Experiment(_) => None,
            },
        };
        let actor = self
            .command_record
            .as_ref()
            .and_then(|(_, tree)| tree.command.started_by.as_ref())
            .map(|actor| format!(", {actor}"))
            .unwrap_or_default();
        let line = format!(
            "{} started: {command_log}, on {harness}{actor}",
            started.subject()
        );
        if self.activity_started.is_none() {
            self.activity_started = Some(started);
            self.activity_write(&root, &line, None);
        }
    }

    pub(super) fn child_of(&mut self, parent: Option<PathBuf>) {
        self.parent_record = parent;
    }

    pub(super) fn child_dispatch(&mut self, issue: Option<String>) {
        self.parent_dispatch = issue;
    }

    pub(super) fn dispatch_issue(&self) -> Option<String> {
        self.command_record
            .as_ref()?
            .1
            .active_dispatch_ref()?
            .issue
            .as_ref()
            .map(|issue| issue.url.clone())
    }

    pub(super) fn record_path(&self) -> Option<PathBuf> {
        self.command_record.as_ref().map(|(path, _)| path.clone())
    }

    pub(super) fn stop_request(&mut self) -> Option<super::stop::Request> {
        let path = self
            .parent_record
            .as_ref()
            .or_else(|| self.command_record.as_ref().map(|(path, _)| path))?;
        self.outside.stop_request(path)
    }

    fn start_record(&mut self, work: Work, harness: &Choice) {
        let stamp = self.stamp();
        let mut facts = CommandRecord::new(work, &stamp, harness, self.outside.now());
        if self.quota_record
            && let Some((_, tree)) = &self.command_record
        {
            facts.started_at = tree.command.started_at.clone();
        }
        self.quota_record = false;
        if matches!(work, Work::Process { .. })
            && let Some(identity) = &self.custom_process
        {
            facts.process = identity.clone();
        }
        facts.hostname = self.outside.hostname();
        facts.process_start = self
            .outside
            .process_start(facts.process_id)
            .unwrap_or_default();
        facts.parent_record = self.parent_record.clone();
        facts.parent_dispatch = self.parent_dispatch.clone();
        let path = if let (Some(parent), Some(issue)) = (&self.parent_record, work.issue()) {
            owner::child_record(parent, self.parent_dispatch.as_deref(), issue)
        } else {
            let Some(owner) = &self.owner else {
                return;
            };
            let Some(path) = owner.record_path() else {
                return;
            };
            path
        };
        self.command_record = Some((
            path,
            CommandTree {
                command: facts,
                dispatched: Vec::new(),
            },
        ));
        self.clocks = None;
        self.check_suspension();
        if let Some(stage) = work.initial_stage() {
            self.stage(stage, None);
        } else {
            self.write_record();
        }
    }

    pub(super) fn tickets(&mut self, tickets: Vec<TicketRecord>) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.active().tickets = tickets;
            self.write_record();
        }
    }

    pub(super) fn skipped(&mut self, pass: Pass, repo: &Repo, reason: impl Display) {
        if self.quota_record {
            self.outcome("skipped", Some(&reason.to_string()));
        }
        let kind = Kind::Pass(pass);
        self.activity_write(
            &self.root(repo),
            &format!("{kind} skipped: {reason}"),
            Some(kind),
        );
    }

    pub(super) fn base_ci_skipped(&mut self, repo: &Repo, reason: &str) {
        let kind = Kind::BaseCi;
        let message = format!("{kind} skipped: {reason}");
        self.step(&message);
        self.activity_write(&self.root(repo), &message, Some(kind));
    }

    /// A skipped Custom process keeps the Activity log line and a Command
    /// record. The record is what shows the skipped ending; the other Passes
    /// drop the Command log they never kept.
    pub(super) fn skipped_process(
        &mut self,
        repo: &Repo,
        name: &str,
        harness: &Choice,
        reason: &crate::command_limit::NotAdmitted,
    ) {
        let root = self.root(repo);
        self.record_config_warnings(&root);
        // Quota waits compare this named Process's last reason even when
        // another Custom process fires between its Passes.
        let history = if reason.is_quota_wait() {
            History::ProcessSkip(name)
        } else {
            History::Kind(Kind::Process)
        };
        self.activity_write_with_history(
            &root,
            &format!("Process {name} skipped: {reason}"),
            Some(history),
        );
        let stamp = self.stamp();
        // Leave held lines unshown, so a quiet skip prints nothing. They are
        // still the Command log.
        self.retain(&root, Work::Process { repo, name }, &stamp, false);
        self.start_record(Work::Process { repo, name }, harness);
        self.outcome("skipped", Some(&reason.to_string()));
    }

    pub(super) fn activity(&mut self, message: impl Display) {
        if let Some(started) = &self.activity_started {
            let root = started.root.clone();
            self.activity_write(&root, &message.to_string(), None);
        }
    }

    pub(super) fn ended(&mut self, outcome: impl Display) {
        self.show_held();
        if let Some(started) = &self.activity_started {
            let root = started.root.clone();
            let subject = started.subject();
            if let Some(request) = self.stop_request() {
                self.activity_write(&root, &format!("{subject} stopped {}", request.actor), None);
            }
            let line = format!("{subject} ended: {outcome}");
            self.activity_write(&root, &line, None);
        }
    }

    pub(super) fn issue_title(&mut self, title: Option<String>) {
        if let Some((_, tree)) = &mut self.command_record
            && let Some(issue) = &mut tree.active().issue
        {
            issue.title = title;
            self.write_record();
        }
    }

    pub(super) fn issue_branch(&mut self, branch: Option<&str>) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.active().issue_branch = branch.map(String::from);
            self.write_record();
        }
    }

    pub(super) fn base_branch(&mut self, branch: &str) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.active().base_branch = Some(branch.into());
            self.write_record();
        }
    }

    pub(super) fn pull_request(&mut self, url: &str) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.active().pull_request = Some(url.into());
            self.write_record();
        }
    }

    pub(super) fn stage(&mut self, stage: StageName, log: Option<&Path>) {
        self.check_suspension();
        let name = stage.name();
        let Some((_, tree)) = &mut self.command_record else {
            return;
        };
        let facts = tree.active();
        if let Some(log) = log {
            facts.session_log = Some(log.into());
        }
        if facts.stages.last().is_none_or(|stage| stage.name != name) {
            facts.stages.push(Stage {
                name: name.into(),
                began_at: self.outside.now().to_rfc3339(),
                session_log: facts.session_log.clone(),
                suspended_seconds: 0,
                suspensions: Vec::new(),
            });
        } else if let Some(stage) = facts.stages.last_mut() {
            stage.session_log = facts.session_log.clone();
        }
        self.write_record();
    }

    pub(super) fn quota_wait(&mut self, until: Option<&str>) {
        if until.is_some()
            && self.command_record.is_none()
            && let Some(repo) = self.preflight_repo.clone()
            && self.logs_dir.is_some()
            && (!self.child || self.preflight_issue.is_some())
        {
            let issue = self.preflight_issue.clone();
            let work = Work::Starting {
                repo: &repo,
                command: self.command,
                issue: issue.as_ref(),
            };
            if !self.child {
                let stamp = self.stamp();
                self.keep(&self.root(&repo), work, &stamp);
            }
            self.start_record(work, &Choice::default());
            if let Some((_, tree)) = &mut self.command_record {
                // No Harness has been selected or launched during preflight.
                tree.command.harness = "not started".into();
            }
            self.quota_record = true;
        }
        if let Some((_, tree)) = &mut self.command_record {
            tree.active().quota_wait_until = until.map(str::to_owned);
            self.write_record();
        }
    }

    pub(super) fn unaddressed_findings(
        &mut self,
        counts: Option<crate::pull_request::UnaddressedFindings>,
    ) {
        self.unaddressed_findings = counts;
    }

    pub(super) fn finished(&mut self, account: &crate::run_ending::Account<'_>) {
        self.check_suspension();
        let ranges = self.usual_ranges();
        if let Some((_, tree)) = &mut self.command_record {
            tree.command.finish(
                account,
                &mut self.outside,
                &ranges,
                self.unaddressed_findings.take(),
            );
            self.write_record();
        }
    }

    pub(super) fn experiment(&mut self, description: &crate::evals::experiment::Description) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.command.experiment = Some(description.clone());
            self.write_record();
        }
    }

    pub(super) fn waiting_for(&mut self, commands: &[String]) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.command.waiting_for = commands.to_vec();
            self.write_record();
        }
    }

    pub(super) fn resumable(&mut self, command: &str) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.command.resume_command = Some(command.into());
            self.write_record();
        }
    }

    pub(super) fn paused(&mut self, limit: &crate::session::limits::Limit) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.command.limit = Some(limit.clone());
            self.write_record();
        }
    }

    /// How a Command ended when it has no Run account: an Experiment or a Process.
    pub(super) fn outcome(&mut self, outcome: &str, cause: Option<&str>) {
        self.check_suspension();
        if let Some((_, tree)) = &mut self.command_record {
            tree.command.outcome = Some(outcome.into());
            tree.command.cause = cause.map(String::from);
            tree.command.ended_at = Some(self.outside.now().to_rfc3339());
            self.write_record();
        }
    }

    pub(super) fn dispatch_ended(&mut self, account: &crate::run_ending::Account<'_>) {
        self.check_suspension();
        let ranges = self.usual_ranges();
        if let Some((_, tree)) = &mut self.command_record
            && let Some(dispatched) = tree.active_dispatch()
        {
            dispatched.finish(
                account,
                &mut self.outside,
                &ranges,
                self.unaddressed_findings.take(),
            );
            self.write_record();
        }
    }

    pub(super) fn dispatching(
        &mut self,
        work: Work,
        harness: &Choice,
        security_fix_findings: &[crate::security::RecordedFinding],
    ) {
        self.check_suspension();
        // Entered while the pass is still the active record, timing the
        // dispatch. An Architect run keeps its Architecture review stage,
        // as #889 asked only for Pickup runs and Security runs.
        if matches!(self.command, CommandKind::Pickup | CommandKind::Secure) {
            self.stage(StageName::dispatched(work), None);
        }
        let stamp = self.stamp();
        let mut facts = CommandRecord::new(work, &stamp, harness, self.outside.now());
        facts.security_fix_findings = security_fix_findings.to_vec();
        if let Some((_, tree)) = &mut self.command_record {
            facts.hostname = tree.command.hostname.clone();
            facts.process_start = tree.command.process_start.clone();
            tree.dispatched.push(facts);
            self.write_record();
        }
    }

    fn usual_ranges(&mut self) -> super::usual_range::UsualRanges {
        let Some((path, tree)) = &self.command_record else {
            return super::usual_range::UsualRanges::default();
        };
        let root = self
            .logs_dir
            .as_ref()
            .unwrap()
            .join(&tree.command.repository);
        match self.outside.command_records(&root) {
            Ok(mut records) => {
                // The Run being finished must not supply its own comparison times.
                records.remove(path);
                super::usual_range::UsualRanges::from_records(&records)
            }
            Err(error) => {
                self.step(format_args!(
                    "warning: could not read Usual ranges for Run metrics: {error:#}"
                ));
                super::usual_range::UsualRanges::default()
            }
        }
    }

    fn write_record(&mut self) {
        let Some((path, facts)) = &self.command_record else {
            return;
        };
        let written = serde_json::to_vec_pretty(facts)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| self.outside.replace_record(path, &bytes));
        if let Err(error) = written
            && !self.record_warned
        {
            self.record_warned = true;
            self.step(format_args!(
                "warning: could not keep the Command record: {error:#}"
            ));
        }
    }

    pub(super) fn show_held(&mut self) {
        if let Log::Held {
            lines,
            shown,
            shown_early,
        } = &mut self.command_log
            && !*shown
        {
            for (index, (stream, line)) in lines.iter().enumerate() {
                if !shown_early.contains(&index) {
                    let _ = self.outside.emit(*stream, line);
                }
            }
            *shown = true;
        }
    }

    pub(super) fn root(&self, repo: &Repo) -> PathBuf {
        let logs_dir = self.logs_dir.as_deref().expect(
            "the root of a repository's logs is asked for before the User config is loaded",
        );
        root_under(logs_dir, repo)
    }

    pub(super) fn root_if_configured(&self, repo: &Repo) -> Option<PathBuf> {
        Some(root_under(self.logs_dir.as_deref()?, repo))
    }

    pub(super) fn command_kind(&self) -> CommandKind {
        self.command
    }

    pub(super) fn stamp(&mut self) -> String {
        self.stamp
            .get_or_insert_with(|| self.outside.now().format(STAMP_FORMAT).to_string())
            .clone()
    }

    pub(super) fn command_log_path(&self) -> Option<PathBuf> {
        match &self.command_log {
            Log::Kept { path, .. } => Some(path.clone()),
            Log::Failed { path } => path.clone(),
            Log::NotKept | Log::Held { .. } => None,
        }
    }

    pub(super) fn inherit_owner(&mut self, inherited: Result<Owner>) {
        if self.owner.is_none() {
            match inherited {
                Ok(owner) => self.owner = Some(owner),
                Err(error) => self.owner_error = Some(format!("{error:#}")),
            }
        }
    }

    pub(super) fn captured_owner(&self) -> Result<Owner, String> {
        self.owner.clone().ok_or_else(|| {
            self.owner_error
                .clone()
                .unwrap_or_else(|| "Command logging has not allocated an owner".into())
        })
    }

    pub(super) fn process_loaded(&mut self, identity: crate::processes::Identity) {
        self.custom_process = Some(identity);
    }

    pub(super) fn process_running(&mut self, identity: crate::processes::Identity) {
        if let Some((_, tree)) = &mut self.command_record {
            tree.active().process = identity;
            self.write_record();
        }
    }

    pub(super) fn custom_process(&self) -> Option<crate::processes::Identity> {
        self.custom_process.clone()
    }

    pub(super) fn trace_context(&self) -> Option<super::trace::Context> {
        let owner = self.owner.as_ref()?;
        let (_, tree) = self.command_record.as_ref()?;
        Some(super::trace::Context::from_record(owner, tree.active_ref()))
    }

    /// Child Runs carry the owning CommandKind, so private Security and
    /// Scoring Session trees stay local, including public Security fixes.
    pub(super) fn otlp_endpoint(&self) -> Option<String> {
        if matches!(self.command, CommandKind::Secure | CommandKind::Scoring) {
            None
        } else {
            self.otlp_endpoint.clone()
        }
    }

    pub(super) fn print(&mut self, line: &str) {
        self.output(Stream::Stdout, line.to_string(), LogActivity::Work);
    }
    pub(super) fn eprint(&mut self, line: &str) {
        // A child Command's wake annotation can be relayed through several
        // parents. None of those copies is evidence that work resumed.
        let activity = if progress::is_suspension_annotation(line) {
            LogActivity::Suspension
        } else {
            LogActivity::Work
        };
        self.output(Stream::Stderr, line.to_string(), activity);
    }

    fn step(&mut self, message: impl Display) {
        let line = self.progress_line(message);
        self.output(Stream::Stderr, line, LogActivity::Work);
    }

    fn progress_line(&mut self, message: impl Display) -> String {
        progress::progress_line(self.outside.now().format("%H:%M:%S"), message)
    }

    /// Warnings enter the same ordered output path without recursion. A
    /// failed writer is disabled before its warning is queued.
    fn output(&mut self, stream: Stream, line: String, activity: LogActivity) {
        let mut pending = VecDeque::from([(stream, line, activity)]);
        while let Some((stream, line, activity)) = pending.pop_front() {
            if !matches!(self.command_log, Log::Held { shown: false, .. }) {
                let _ = self.outside.emit(stream, &line);
            }
            match &mut self.command_log {
                Log::NotKept | Log::Failed { .. } => {}
                Log::Held { lines, .. } => lines.push((stream, line)),
                Log::Kept { path, writer } => {
                    let written = match activity {
                        LogActivity::Work => writeln!(writer, "{line}"),
                        LogActivity::Suspension => self.outside.suspension_line(writer, &line),
                    };
                    if let Err(error) = written {
                        let error = anyhow::Error::new(error)
                            .context(format!("could not write {}", path.display()));
                        self.command_log = Log::Failed {
                            path: Some(path.clone()),
                        };
                        let warning = self.progress_line(format_args!(
                            "warning: could not keep the Command log: {error:#}"
                        ));
                        pending.push_back((Stream::Stderr, warning, LogActivity::Work));
                    }
                }
            }
        }
    }

    fn keep(&mut self, root: &Path, work: Work<'_>, stamp: &str) {
        self.retain(root, work, stamp, true);
    }

    /// Keep the Command log. `announce` shows lines held from the terminal and
    /// names the log there. A skipped Custom process passes `false`: under
    /// `activity.quiet_skips` those lines stay unprinted, and the log-path
    /// line stays off the terminal either way.
    fn retain(&mut self, root: &Path, work: Work<'_>, stamp: &str, announce: bool) {
        if announce {
            self.show_held();
        }
        if !matches!(self.command_log, Log::Held { .. }) {
            return;
        }
        let Log::Held { lines, .. } = std::mem::replace(&mut self.command_log, Log::NotKept) else {
            unreachable!()
        };
        let created: Result<O::CommandLog> = (|| {
            let allocation = owner::allocate(&mut self.outside, root, work, stamp)?;
            self.owner = Some(allocation.owner);
            let mut writer = allocation.writer;
            let path = self.owner.as_ref().unwrap().log_path().unwrap();
            for (_, line) in lines {
                writeln!(writer, "{line}")
                    .with_context(|| format!("could not write {}", path.display()))?;
            }
            Ok(writer)
        })();
        match created {
            Ok(writer) => {
                self.command_log = Log::Kept {
                    path: self
                        .owner
                        .as_ref()
                        .unwrap()
                        .log_path()
                        .unwrap()
                        .to_path_buf(),
                    writer,
                };
                if announce {
                    let path = self.command_log_path().unwrap();
                    self.step(format_args!("logging this command to {}", path.display()));
                }
            }
            Err(error) => {
                self.owner_error = Some(format!("{error:#}"));
                self.command_log = Log::Failed {
                    path: self
                        .owner
                        .as_ref()
                        .and_then(|owner| owner.log_path().map(Path::to_path_buf)),
                };
                self.step(format_args!(
                    "warning: could not keep the Command log: {error:#}"
                ));
            }
        }
    }

    fn activity_write(&mut self, root: &Path, line: &str, unless_last_of: Option<Kind>) {
        self.activity_write_with_history(root, line, unless_last_of.map(History::Kind));
    }

    fn activity_write_with_history(
        &mut self,
        root: &Path,
        line: &str,
        unless_last_of: Option<History<'_>>,
    ) {
        let appended = self.activity_append(&root.join(activity::FILE), line, unless_last_of);
        if let Err(error) = appended
            && !self.activity_warned
        {
            self.activity_warned = true;
            self.show_held();
            self.step(format_args!(
                "warning: could not keep the Activity log: {error:#}"
            ));
        }
    }

    fn activity_append(
        &mut self,
        path: &Path,
        line: &str,
        unless_last_of: Option<History<'_>>,
    ) -> Result<()> {
        if let Some(history) = unless_last_of {
            let tail = self.outside.activity_tail(path, activity::TAIL)?;
            let text = String::from_utf8_lossy(&tail.bytes);
            let whole = if tail.from > 0 {
                text.split_once('\n').map_or("", |(_, rest)| rest)
            } else {
                &text
            };
            let last = match history {
                History::Kind(kind) => activity::last_of(whole, kind),
                History::ProcessSkip(name) => activity::last_process_skip(whole, name),
            };
            if last.map(|(_, text)| text) == Some(line) {
                return Ok(());
            }
        }
        let stamped = format!(
            "{} {line}\n",
            self.outside.now().format(activity::TIME_FORMAT)
        );
        self.outside.append_activity(path, &stamped)
    }
}

#[cfg(test)]
mod tests;
