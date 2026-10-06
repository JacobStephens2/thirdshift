//! What a run is asked: its command's flags, and, for whatever the command
//! says nothing about, the User config. The argument parser fills in the
//! [`Flags`] as the command gave them; what they come to, for each kind of
//! run, is its [`Asks`].

use std::num::NonZeroUsize;

use crate::args;
use crate::base_fix::BaseFixAsk;
use crate::child_run::Given;
use crate::config::UserConfig;
use crate::harness::{self, Choice};
use crate::issue::IssueUrl;
use crate::notification::NotificationAsk;
use crate::run::Goal;

/// The flags a Run, an Architect run and a Pickup run share, as the command
/// gave them: each is none if the command said nothing about it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Flags {
    /// The goal `merge` or `no-merge` asked for.
    pub goal: Option<Goal>,
    /// What `email` or `no-email` asked for.
    pub email: Option<NotificationAsk>,
    /// How many Tickets a Spec run runs at once, as `parallel <n>` gave it.
    pub parallel: Option<NonZeroUsize>,
    /// What `base-fix` or `no-base-fix` asked for.
    pub base_fix: Option<BaseFixAsk>,
    /// What `harness <name>`, `model <name>` and `effort <level>` asked for.
    pub harness: harness::Asked,
}

/// Everything a Run, or a Spec run, is asked, resolved.
#[derive(Debug, PartialEq, Eq)]
pub struct Asks {
    /// Whether it is a Merge run.
    pub goal: Goal,
    /// What it is asked about its Run notification.
    pub notification: NotificationAsk,
    /// How many Tickets a Spec run runs at once.
    pub tickets_at_once: NonZeroUsize,
    /// Whether the command asked for that with `parallel <n>`, rather than
    /// the User config deciding: only a Spec can be asked it.
    pub parallel_asked: bool,
    /// What it is asked about a Base fix.
    pub base_fix: BaseFixAsk,
    /// Whether it first brings the Launch directory's checkout of the Base
    /// branch up to date with origin.
    pub launch_pull: bool,
    /// The Harness, Model and Effort its sessions, and its child Runs', run
    /// on.
    pub harness: Choice,
}

impl Asks {
    /// What the Run, or the Spec run, started on `issue`'s URL is asked: by
    /// `flags`, else by `config`. Where neither decided about a Base fix,
    /// the command that retries `issue` is the one on its Issue URL with
    /// `flags`.
    pub fn of_run(issue: &IssueUrl, flags: &Flags, config: &UserConfig) -> Asks {
        let goal = flags.goal.unwrap_or(if config.merge_always {
            Goal::Merged
        } else {
            Goal::ReadyForReview
        });
        let base_fix = flags.base_fix.clone().unwrap_or_else(|| {
            if config.base_fix {
                BaseFixAsk::Allow
            } else {
                let retry = retry_with_base_fix(issue, flags);
                BaseFixAsk::Undecided { retry }
            }
        });
        Asks {
            goal,
            notification: flags.notification(config),
            tickets_at_once: flags.parallel.unwrap_or(config.spec_parallel),
            parallel_asked: flags.parallel.is_some(),
            base_fix,
            launch_pull: config.launch_pull,
            harness: flags.harness(config),
        }
    }

    /// What a child Run, a Ticket's Run in a Spec run or a Base fix, is
    /// asked, from what it was `given`: it is always a Merge run, sends no
    /// Run notification, leaves the Launch directory alone, and asks about a
    /// Base fix what it was given, running its sessions on the Harness,
    /// Model and Effort it was given. Neither its flags nor `config` decide
    /// any of that: those are for what started it. As it runs no Tickets,
    /// `config` says only how many it would run at once.
    pub fn of_child_run(given: &Given, config: &UserConfig) -> Asks {
        Asks {
            goal: Goal::Merged,
            notification: NotificationAsk::Skip,
            tickets_at_once: config.spec_parallel,
            parallel_asked: false,
            base_fix: given.base_fix.clone(),
            launch_pull: false,
            harness: given.harness.clone(),
        }
    }

    /// What the Spec run or Run an Architect run dispatches its Architect
    /// plan `plan` as is asked by the Architect run's `flags`, else by
    /// `config`: see [`Asks::dispatched`]. The command a Base fix is offered
    /// with retries the plan as a Run of its own: another Architect run
    /// would start a new review instead.
    pub fn of_architect_plan(plan: &IssueUrl, flags: &Flags, config: &UserConfig) -> Asks {
        Asks::dispatched(plan, flags, config)
    }

    /// What the Spec run or Run a Pickup run dispatches the Ready issue
    /// `issue` as is asked by the Pickup run's `flags`, else by `config`:
    /// see [`Asks::dispatched`]. `parallel` is ignored for an issue that is
    /// not a Spec, as `is_spec` says, since the command can't know which
    /// the Pickup run will take. It is left out of the command a Base fix is
    /// offered with too, which retries the issue as a Run of its own:
    /// another Pickup run never takes an issue that was started.
    pub fn of_ready_issue(
        issue: &IssueUrl,
        is_spec: bool,
        flags: &Flags,
        config: &UserConfig,
    ) -> Asks {
        let flags = Flags {
            parallel: flags.parallel.filter(|_| is_spec),
            ..flags.clone()
        };
        Asks::dispatched(issue, &flags, config)
    }

    /// What the Spec run or Run that an Architect run or a Pickup run
    /// dispatches `issue` as is asked: what [`Asks::of_run`] would ask a Run
    /// started on the Issue URL with `flags`, except that it sends no Run
    /// notification of its own. The run that dispatched it sends the one, as
    /// [`Flags::notification`] asks.
    fn dispatched(issue: &IssueUrl, flags: &Flags, config: &UserConfig) -> Asks {
        Asks {
            notification: NotificationAsk::Skip,
            ..Asks::of_run(issue, flags, config)
        }
    }
}

impl Flags {
    /// Whether any flag here is for the Spec run or Run that an Architect
    /// run or a Pickup run dispatches: every one but `email` and `no-email`,
    /// which ask that run for its own Run notification.
    pub fn any_for_dispatched_run(&self) -> bool {
        let for_dispatched_run = Flags {
            email: None,
            ..self.clone()
        };
        for_dispatched_run != Flags::default()
    }

    /// What a run with these flags is asked about its own Run notification:
    /// what `email` or `no-email` asked for, else what `config` says. It is
    /// a Run's ask, and an Architect run's or a Pickup run's for the one
    /// notification it sends itself.
    pub fn notification(&self, config: &UserConfig) -> NotificationAsk {
        self.email.clone().unwrap_or(if config.email.always {
            NotificationAsk::Send(None)
        } else {
            NotificationAsk::Skip
        })
    }

    /// The Harness, Model and Effort a command with these flags runs its
    /// sessions on: each as its flag asked, else as `config` sets it, else
    /// the default. It is a Run's, and an Architect run's or a Pickup run's
    /// for its own sessions as for the run it dispatches.
    pub fn harness(&self, config: &UserConfig) -> Choice {
        Choice::of(&self.harness, &config.harness)
    }
}

/// The command that starts the Run on `issue` again as `flags` asked for it,
/// with `base-fix` added.
fn retry_with_base_fix(issue: &IssueUrl, flags: &Flags) -> String {
    let mut command = format!("thirdshift {}", issue.url);
    match flags.goal {
        Some(Goal::Merged) => command += " merge",
        Some(Goal::ReadyForReview) => command += " --no-merge",
        None => {}
    }
    match &flags.email {
        Some(NotificationAsk::Send(Some(to))) => command += &format!(" --email {to}"),
        Some(NotificationAsk::Send(None)) => command += " --email",
        Some(NotificationAsk::Skip) => command += " --no-email",
        None => {}
    }
    if let Some(parallel) = flags.parallel {
        command += &format!(" parallel {parallel}");
    }
    let asked = &flags.harness;
    if let Some(harness) = asked.harness {
        command += &format!(" harness {}", harness.name());
    }
    if let Some(model) = &asked.model_and_effort.model {
        command += &format!(" model {model}");
    }
    if let Some(effort) = &asked.model_and_effort.effort {
        command += &format!(" effort {effort}");
    }
    command + " " + args::BASE_FIX
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::child_run::Kind;
    use crate::config::EmailSettings;
    use crate::harness::{ChosenBy, Harness, ModelAndEffort};

    const URL: &str = "https://github.com/acme/widgets/issues/7";

    fn issue() -> IssueUrl {
        IssueUrl::parse(URL).unwrap()
    }

    fn n(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    fn to(address: &str) -> NotificationAsk {
        NotificationAsk::Send(Some(address.to_string()))
    }

    /// A User config that sets nothing: three Tickets at once is its own
    /// default.
    fn no_settings() -> UserConfig {
        UserConfig {
            merge_always: false,
            base_fix: false,
            launch_pull: false,
            logs_dir: PathBuf::from("/home/me/.thirdshift/logs"),
            quiet_skips: false,
            email: EmailSettings::default(),
            spec_parallel: n(3),
            pickup_limit: n(3),
            harness: harness::Settings::default(),
        }
    }

    /// A User config that sets every default a Run can be asked by.
    fn every_setting() -> UserConfig {
        UserConfig {
            merge_always: true,
            base_fix: true,
            launch_pull: true,
            email: EmailSettings {
                always: true,
                to: Some("config@example.com".to_string()),
                from: None,
            },
            spec_parallel: n(5),
            harness: harness::Settings {
                default: Some(Harness::Codex),
                claude: model_and_effort("opus", "high"),
                codex: model_and_effort("gpt-6.1-sol", "max"),
                muse: ModelAndEffort::default(),
            },
            ..no_settings()
        }
    }

    fn model_and_effort(model: &str, effort: &str) -> ModelAndEffort {
        ModelAndEffort {
            model: Some(model.to_string()),
            effort: Some(effort.to_string()),
        }
    }

    /// The sessions [`every_setting`] chooses with no flag.
    fn configured_codex() -> Choice {
        Choice {
            harness: Harness::Codex,
            model: Some("gpt-6.1-sol".to_string()),
            effort: Some("max".to_string()),
            chosen_by: ChosenBy::UserConfig,
        }
    }

    /// Claude on the Model `sonnet`, as the command chose, with `effort`.
    fn sonnet(effort: Option<&str>) -> Choice {
        Choice {
            harness: Harness::Claude,
            model: Some("sonnet".to_string()),
            effort: effort.map(str::to_string),
            chosen_by: ChosenBy::Command,
        }
    }

    /// The flags that choose Claude, on the Model `sonnet`.
    fn claude_on_sonnet() -> harness::Asked {
        harness::Asked {
            harness: Some(Harness::Claude),
            model_and_effort: ModelAndEffort {
                model: Some("sonnet".to_string()),
                effort: None,
            },
        }
    }

    /// The command that starts the Run on [`URL`] again with a Base fix
    /// allowed, `flags` being its other flags as the command would spell
    /// them.
    fn undecided(flags: &str) -> BaseFixAsk {
        BaseFixAsk::Undecided {
            retry: format!("thirdshift {URL}{flags} base-fix"),
        }
    }

    #[test]
    fn a_run_with_neither_a_flag_nor_a_setting_does_only_what_a_run_does() {
        let asks = Asks::of_run(&issue(), &Flags::default(), &no_settings());

        assert_eq!(
            asks,
            Asks {
                goal: Goal::ReadyForReview,
                notification: NotificationAsk::Skip,
                tickets_at_once: n(3),
                parallel_asked: false,
                base_fix: undecided(""),
                launch_pull: false,
                harness: Choice::default(),
            }
        );
    }

    #[test]
    fn the_user_config_decides_each_ask_the_command_says_nothing_about() {
        let asks = Asks::of_run(&issue(), &Flags::default(), &every_setting());

        assert_eq!(
            asks,
            Asks {
                goal: Goal::Merged,
                notification: NotificationAsk::Send(None),
                tickets_at_once: n(5),
                parallel_asked: false,
                base_fix: BaseFixAsk::Allow,
                launch_pull: true,
                harness: configured_codex(),
            }
        );
    }

    #[test]
    fn each_flag_decides_its_ask_whatever_the_user_config_says() {
        let against = Flags {
            goal: Some(Goal::ReadyForReview),
            email: Some(NotificationAsk::Skip),
            parallel: Some(n(2)),
            base_fix: Some(BaseFixAsk::Forbid),
            harness: claude_on_sonnet(),
        };
        let asks = Asks::of_run(&issue(), &against, &every_setting());
        assert_eq!(
            asks,
            Asks {
                goal: Goal::ReadyForReview,
                notification: NotificationAsk::Skip,
                tickets_at_once: n(2),
                parallel_asked: true,
                base_fix: BaseFixAsk::Forbid,
                // No flag sets it.
                launch_pull: true,
                harness: sonnet(Some("high")),
            }
        );

        let for_it = Flags {
            goal: Some(Goal::Merged),
            email: Some(to("flag@example.com")),
            parallel: Some(n(2)),
            base_fix: Some(BaseFixAsk::Allow),
            harness: claude_on_sonnet(),
        };
        let asks = Asks::of_run(&issue(), &for_it, &no_settings());
        assert_eq!(
            asks,
            Asks {
                goal: Goal::Merged,
                notification: to("flag@example.com"),
                tickets_at_once: n(2),
                parallel_asked: true,
                base_fix: BaseFixAsk::Allow,
                launch_pull: false,
                harness: sonnet(None),
            }
        );
    }

    /// What a child Run of `kind` is given, asked `base_fix` about a Base
    /// fix, its sessions on Claude's `sonnet`.
    fn given(kind: Kind, base_fix: BaseFixAsk) -> Given {
        Given {
            kind,
            stamp: "20261003T120000-0400".to_string(),
            base_fix,
            harness: sonnet(None),
        }
    }

    fn ticket() -> Kind {
        Kind::Ticket {
            spec_branch: "issue-20".to_string(),
        }
    }

    fn base_fix() -> Kind {
        Kind::BaseFix {
            base: "main".to_string(),
        }
    }

    #[test]
    fn a_child_run_is_a_merge_run_that_sends_nothing_pulls_nothing_and_starts_no_base_fix() {
        for child in [ticket(), base_fix()] {
            for config in [no_settings(), every_setting()] {
                let given = given(child.clone(), BaseFixAsk::Forbid);

                let asks = Asks::of_child_run(&given, &config);

                assert_eq!(asks.goal, Goal::Merged, "{child:?}");
                assert_eq!(asks.notification, NotificationAsk::Skip, "{child:?}");
                assert!(!asks.launch_pull, "{child:?}");
                assert!(!asks.parallel_asked, "{child:?}");
                assert_eq!(asks.base_fix, BaseFixAsk::Forbid, "{child:?}");
            }
        }
    }

    #[test]
    fn a_child_run_runs_its_sessions_on_what_it_was_given_whatever_the_user_config_says() {
        for config in [no_settings(), every_setting()] {
            let asks = Asks::of_child_run(&given(ticket(), BaseFixAsk::Forbid), &config);

            assert_eq!(asks.harness, sonnet(None));
        }
    }

    #[test]
    fn a_child_run_asks_about_a_base_fix_what_it_was_given_whatever_the_user_config_says() {
        let offer = BaseFixAsk::Undecided {
            retry: format!("thirdshift {URL} base-fix"),
        };
        for asked in [BaseFixAsk::Allow, offer] {
            for config in [no_settings(), every_setting()] {
                let asks = Asks::of_child_run(&given(ticket(), asked.clone()), &config);

                assert_eq!(asks.base_fix, asked);
            }
        }
    }

    /// Every flag an Architect run or a Pickup run takes for the run it
    /// dispatches, with `email` for its own Run notification.
    fn dispatch_flags() -> Flags {
        Flags {
            goal: Some(Goal::Merged),
            email: Some(to("flag@example.com")),
            parallel: Some(n(2)),
            base_fix: None,
            harness: claude_on_sonnet(),
        }
    }

    #[test]
    fn a_dispatched_architect_plan_sends_no_notification_whoever_decides_its_asks() {
        let asks = Asks::of_architect_plan(&issue(), &dispatch_flags(), &no_settings());
        assert_eq!(
            asks,
            Asks {
                goal: Goal::Merged,
                notification: NotificationAsk::Skip,
                tickets_at_once: n(2),
                parallel_asked: true,
                base_fix: undecided(
                    " merge --email flag@example.com parallel 2 harness claude model sonnet"
                ),
                launch_pull: false,
                harness: sonnet(None),
            }
        );

        let asks = Asks::of_architect_plan(&issue(), &Flags::default(), &every_setting());
        assert_eq!(
            asks,
            Asks {
                goal: Goal::Merged,
                // Whatever `email.always` says: the Architect run sends it.
                notification: NotificationAsk::Skip,
                tickets_at_once: n(5),
                parallel_asked: false,
                base_fix: BaseFixAsk::Allow,
                launch_pull: true,
                harness: configured_codex(),
            }
        );
    }

    #[test]
    fn a_dispatched_ready_issue_that_is_a_spec_sends_no_notification_whoever_decides_its_asks() {
        let asks = Asks::of_ready_issue(&issue(), true, &dispatch_flags(), &no_settings());
        assert_eq!(
            asks,
            Asks {
                goal: Goal::Merged,
                notification: NotificationAsk::Skip,
                tickets_at_once: n(2),
                parallel_asked: true,
                base_fix: undecided(
                    " merge --email flag@example.com parallel 2 harness claude model sonnet"
                ),
                launch_pull: false,
                harness: sonnet(None),
            }
        );

        let asks = Asks::of_ready_issue(&issue(), true, &Flags::default(), &every_setting());
        assert_eq!(
            asks,
            Asks {
                goal: Goal::Merged,
                // Whatever `email.always` says: the Pickup run sends it.
                notification: NotificationAsk::Skip,
                tickets_at_once: n(5),
                parallel_asked: false,
                base_fix: BaseFixAsk::Allow,
                launch_pull: true,
                harness: configured_codex(),
            }
        );
    }

    #[test]
    fn only_the_email_flags_are_not_for_a_dispatched_run() {
        let email_only = Flags {
            email: Some(to("flag@example.com")),
            ..Flags::default()
        };
        assert!(!Flags::default().any_for_dispatched_run());
        assert!(!email_only.any_for_dispatched_run());
        for flags in [
            Flags {
                goal: Some(Goal::ReadyForReview),
                ..email_only.clone()
            },
            Flags {
                parallel: Some(n(1)),
                ..Flags::default()
            },
            Flags {
                base_fix: Some(BaseFixAsk::Forbid),
                ..Flags::default()
            },
            Flags {
                harness: claude_on_sonnet(),
                ..Flags::default()
            },
        ] {
            assert!(flags.any_for_dispatched_run(), "{flags:?}");
        }
    }

    #[test]
    fn parallel_is_ignored_for_a_dispatched_ready_issue_that_is_not_a_spec() {
        let asks = Asks::of_ready_issue(&issue(), false, &dispatch_flags(), &every_setting());

        assert!(!asks.parallel_asked);
        assert_eq!(asks.tickets_at_once, n(5));

        // Nor is it in the command that retries the issue, which would fail
        // a Run on an issue that is not a Spec.
        let asks = Asks::of_ready_issue(&issue(), false, &dispatch_flags(), &no_settings());
        assert_eq!(
            asks.base_fix,
            undecided(" merge --email flag@example.com harness claude model sonnet")
        );
        let asks = Asks::of_ready_issue(&issue(), true, &dispatch_flags(), &no_settings());
        assert_eq!(
            asks.base_fix,
            undecided(" merge --email flag@example.com parallel 2 harness claude model sonnet")
        );
    }

    /// The Run the command `retry` starts, read by the argument parser.
    fn parsed(retry: &str) -> args::RunArgs {
        let words: Vec<String> = retry.split(' ').skip(1).map(str::to_string).collect();
        match args::parse(&words) {
            Ok(args::Command::Run(run)) => run,
            Ok(_) => panic!("{retry}: not a Run"),
            Err(error) => panic!("{retry}: {error:#}"),
        }
    }

    /// The command `asks` offer a Base fix with.
    fn retry_of(asks: Asks) -> String {
        match asks.base_fix {
            BaseFixAsk::Undecided { retry } => retry,
            decided => panic!("no command to offer: {decided:?}"),
        }
    }

    #[test]
    fn the_retry_command_is_the_issue_url_and_the_flags_as_given_with_base_fix_added() {
        let flags = |goal, email, parallel| Flags {
            goal,
            email,
            parallel,
            base_fix: None,
            harness: harness::Asked::default(),
        };
        let on_codex = Flags {
            harness: harness::Asked {
                harness: Some(Harness::Codex),
                model_and_effort: model_and_effort("gpt-6.1-sol", "max"),
            },
            ..flags(Some(Goal::Merged), None, None)
        };
        for (flags, retry) in [
            (
                on_codex,
                format!(
                    "thirdshift {URL} merge harness codex model gpt-6.1-sol effort max base-fix"
                ),
            ),
            (Flags::default(), format!("thirdshift {URL} base-fix")),
            (
                flags(Some(Goal::Merged), Some(NotificationAsk::Send(None)), None),
                format!("thirdshift {URL} merge --email base-fix"),
            ),
            (
                flags(
                    Some(Goal::ReadyForReview),
                    Some(NotificationAsk::Skip),
                    Some(n(2)),
                ),
                format!("thirdshift {URL} --no-merge --no-email parallel 2 base-fix"),
            ),
            (
                flags(None, Some(to("me@example.com")), None),
                format!("thirdshift {URL} --email me@example.com base-fix"),
            ),
        ] {
            let config = no_settings();
            let allowed = Flags {
                base_fix: Some(BaseFixAsk::Allow),
                ..flags.clone()
            };
            // Each kind of run offers the same command: a Run on the Issue
            // URL, whatever dispatched it.
            for (kind, asks) in [
                ("a Run", Asks::of_run(&issue(), &flags, &config)),
                (
                    "an Architect plan",
                    Asks::of_architect_plan(&issue(), &flags, &config),
                ),
                (
                    "a Ready issue that is a Spec",
                    Asks::of_ready_issue(&issue(), true, &flags, &config),
                ),
            ] {
                let offered = retry_of(asks);
                assert_eq!(offered, retry, "{kind}");
                // The command it gives asks for what the run was asked for.
                let again = parsed(&offered);
                assert_eq!(again.issue.url, URL, "{kind}: {offered}");
                assert_eq!(again.flags, allowed, "{kind}: {offered}");
                assert_eq!(again.given, None, "{kind}: {offered}");
            }
            // Without the `parallel` that would fail its Run by hand.
            let offered = retry_of(Asks::of_ready_issue(&issue(), false, &flags, &config));
            let again = parsed(&offered);
            let without_parallel = Flags {
                parallel: None,
                ..allowed
            };
            assert_eq!(again.flags, without_parallel, "{offered}");
        }
    }

    #[test]
    fn an_architect_run_or_a_pickup_run_is_asked_for_its_own_notification_as_a_run_is() {
        for (email, config, asked) in [
            (None, no_settings(), NotificationAsk::Skip),
            (None, every_setting(), NotificationAsk::Send(None)),
            (
                Some(NotificationAsk::Skip),
                every_setting(),
                NotificationAsk::Skip,
            ),
            (
                Some(NotificationAsk::Send(None)),
                no_settings(),
                NotificationAsk::Send(None),
            ),
            (
                Some(to("flag@example.com")),
                every_setting(),
                to("flag@example.com"),
            ),
        ] {
            let flags = Flags {
                email,
                ..dispatch_flags()
            };

            assert_eq!(flags.notification(&config), asked, "{flags:?}");
        }
    }

    #[test]
    fn an_address_after_the_email_flag_wins_over_email_always() {
        let flags = Flags {
            email: Some(to("flag@example.com")),
            ..Flags::default()
        };

        let asks = Asks::of_run(&issue(), &flags, &every_setting());

        assert_eq!(asks.notification, to("flag@example.com"));
    }

    #[test]
    fn email_to_alone_asks_for_no_notification() {
        let mut config = no_settings();
        config.email.to = Some("config@example.com".to_string());

        let asks = Asks::of_run(&issue(), &Flags::default(), &config);

        assert_eq!(asks.notification, NotificationAsk::Skip);
    }
}
