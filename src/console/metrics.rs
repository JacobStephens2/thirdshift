//! Metrics: outcome stacks and signals over time, the twin of `metrics`.
use anyhow::Result;
use askama::Template;
use axum::Extension;
use axum::body::Bytes;
use axum::extract::RawQuery;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use super::auth::Viewer;
use crate::args::{MetricsArgs, MetricsWindow};
use crate::config::UserConfig;
use crate::logs::metrics::fleet::{self, FailedCommand, Fleet};
use crate::logs::metrics::{Bucket, Class, Ending, Stopped, Timeline, Trends};

/// The query keys that are `metrics` filters and window, each a `--<key>` flag.
const FILTERS: [&str; 10] = [
    "window", "grain", "from", "through", "repo", "kind", "harness", "model", "version", "account",
];

/// The keys a failure drilldown adds to the view's own filters: a Where runs
/// failed row's stage and category, or a Failed bucket's day, kind and account.
const STOPPED_AT: &str = "stopped_at";
const CATEGORY: &str = "category";
const FAILED_ON: &str = "failed_on";
const FAILED_KIND: &str = "failed_kind";
const FAILED_ACCOUNT: &str = "failed_account";

/// The page's scope: this machine's name or a Fleet machine's. Without one,
/// the page shows the Fleet when it has one. Only the page reads other
/// machines; the API stays local and refuses both keys.
const MACHINE: &str = "machine";
const FLEET: &str = "fleet";

/// The machines a Metrics page can be scoped to, for its selector.
struct Machines {
    /// The machine chosen; missing for the whole Fleet. Any name, `fleet`
    /// included, is a machine's.
    chosen: Option<String>,
    /// Whether the Fleet can be chosen: a machine is paired or saved.
    fleet: bool,
    /// This machine's name, distinct from every Fleet entry's.
    local: String,
    /// Fleet machines and retired ones with a saved snapshot.
    others: Vec<String>,
}

impl Machines {
    /// The `machine` pair a link to this machine's view keeps, needed only
    /// where the page would otherwise open on the Fleet.
    fn pair(&self) -> Option<(&str, &str)> {
        self.chosen
            .as_deref()
            .filter(|_| self.fleet)
            .map(|chosen| (MACHINE, chosen))
    }

    fn is(&self, name: &str) -> bool {
        self.chosen.as_deref() == Some(name)
    }
}

#[derive(Template)]
#[template(path = "console/metrics.html")]
struct MetricsPage {
    page: super::PageContext,
    trends: Trends,
    error: Option<String>,
    form: Form,
    kinds: [&'static str; 9],
    windows: [(&'static str, MetricsWindow); 4],
    classes: [Class; 4],
    /// The page's own filters and window, as given, for links that keep them.
    view: Vec<(String, String)>,
    machines: Machines,
}

impl MetricsPage {
    /// `path` with this view's filters, less any of `replaced`, and `added`.
    fn link(&self, path: &str, replaced: &[&str], added: &[(&str, &str)]) -> String {
        let mut url = reqwest::Url::parse("http://localhost").expect("fixed URL");
        url.set_path(path);
        url.query_pairs_mut()
            .extend_pairs(
                self.view
                    .iter()
                    .filter(|(key, _)| !replaced.contains(&key.as_str())),
            )
            .extend_pairs(added);
        format!("{}?{}", url.path(), url.query().unwrap_or_default())
    }

    /// History showing a Where runs failed row's Commands. The link names the
    /// row, not its Commands, so it stays short however many failed.
    fn row_history(&self, row: &Stopped) -> String {
        let category = [(CATEGORY, row.ending.category())];
        match &row.stopped_at {
            Some(stage) => self.link("/history", &[], &[(STOPPED_AT, stage), category[0]]),
            None => self.link("/history", &[], &category),
        }
    }

    /// History showing a bucket's Failed Commands.
    fn bucket_history(&self, bucket: &Bucket) -> String {
        let day = bucket.bucket.to_string();
        let mut added = vec![(FAILED_ON, day.as_str()), (FAILED_KIND, bucket.kind)];
        if let Some(account) = &bucket.account {
            added.push((FAILED_ACCOUNT, account));
        }
        self.link("/history", &[], &added)
    }

    /// This view narrowed to one `thirdshift` version.
    fn version_slice(&self, version: &str) -> String {
        let mut added = vec![("version", version)];
        added.extend(self.machines.pair());
        self.link("/metrics", &["version"], &added)
    }

    /// A Failed Command's Console details.
    fn details(&self, selector: &str) -> String {
        crate::logs::status::inspect::url("/peek", selector, None)
    }
}

/// Fleet metrics, or one Fleet machine's, the twin of `metrics --fleet`
/// and `metrics --machine <name>`.
#[derive(Template)]
#[template(path = "console/metrics_fleet.html")]
struct FleetPage {
    page: super::PageContext,
    fleet: Fleet,
    error: Option<String>,
    form: Form,
    kinds: [&'static str; 9],
    windows: [(&'static str, MetricsWindow); 4],
    classes: [Class; 4],
    machines: Machines,
    /// The CLI twin, `metrics --fleet` or `metrics --machine <name>`.
    cli: String,
}

impl FleetPage {
    /// How long ago `at` was, roughly, as of now.
    fn age(&self, at: &chrono::DateTime<chrono::Utc>) -> String {
        let seconds = (chrono::Utc::now() - *at).num_seconds().max(0);
        let (count, unit) = match seconds {
            0..60 => return "just now".into(),
            60..3_600 => (seconds / 60, "minute"),
            3_600..86_400 => (seconds / 3_600, "hour"),
            _ => (seconds / 86_400, "day"),
        };
        format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" })
    }

    /// Why a Failed Command's details cannot be opened: its machine's
    /// summary is a saved read or one it pushed.
    fn unopened(&self, failed: &FailedCommand) -> &'static str {
        let reported = self.fleet.machines.iter().any(|machine| {
            machine.name == failed.machine && machine.status == fleet::Status::Reported
        });
        if reported { "reported" } else { "cached" }
    }

    /// A Failed Command's details on the machine that holds it, while that
    /// machine answers; a cached one's details cannot be opened from here.
    fn details(&self, failed: &FailedCommand) -> Option<String> {
        let machine = self
            .fleet
            .machines
            .iter()
            .find(|machine| machine.name == failed.machine)?;
        let route = crate::logs::status::inspect::url("/peek", &failed.command.selector, None);
        if machine.local {
            Some(route)
        } else if machine.status == fleet::Status::Ok {
            Some(crate::logs::status::inspect::machine_url(
                &route,
                &machine.name,
            ))
        } else {
            None
        }
    }
}

/// The filters as asked, to fill the form again. A repeated filter shows one
/// field per value, so Show keeps them all; an unused one shows one empty field.
#[derive(Default)]
struct Form {
    window: String,
    grain: String,
    from: String,
    through: String,
    repo: Vec<String>,
    kind: Vec<String>,
    harness: Vec<String>,
    model: Vec<String>,
    version: Vec<String>,
    account: Vec<String>,
}

impl Form {
    fn of(asked: &MetricsArgs) -> Self {
        let fields = |values: &[String]| {
            if values.is_empty() {
                vec![String::new()]
            } else {
                values.to_vec()
            }
        };
        let date =
            |date: Option<chrono::NaiveDate>| date.map(|d| d.to_string()).unwrap_or_default();
        Self {
            window: asked
                .window
                .map(|window| window.name().to_owned())
                .unwrap_or_default(),
            grain: asked
                .grain
                .map(|grain| grain.name().to_owned())
                .unwrap_or_default(),
            from: date(asked.from),
            through: date(asked.through),
            repo: fields(&asked.repositories),
            kind: fields(&asked.kinds),
            harness: fields(&asked.harnesses),
            model: fields(&asked.models),
            version: fields(&asked.versions),
            account: fields(&asked.accounts),
        }
    }
}

/// The non-empty query pairs among `keys`, in order.
fn pairs(query: &Option<String>, keys: &[&str]) -> Result<Vec<(String, String)>> {
    Ok(reqwest::Url::parse(&format!(
        "http://localhost/?{}",
        query.as_deref().unwrap_or_default()
    ))?
    .query_pairs()
    .filter(|(key, value)| !value.is_empty() && keys.contains(&key.as_ref()))
    .map(|(key, value)| (key.into_owned(), value.into_owned()))
    .collect())
}

/// A drilldown link's query pairs: the view and the cluster it names.
pub(super) fn drilldown(query: &Option<String>) -> Result<Vec<(String, String)>> {
    let mut keys = FILTERS.to_vec();
    keys.extend([STOPPED_AT, CATEGORY, FAILED_ON, FAILED_KIND, FAILED_ACCOUNT]);
    pairs(query, &keys)
}

/// The Failed Commands a drilldown link from the Metrics page names, by
/// reading the same view again; `None` when the query is no drilldown.
pub(super) fn chosen(config: &UserConfig, query: &Option<String>) -> Result<Option<Vec<String>>> {
    let drill = pairs(
        query,
        &[STOPPED_AT, CATEGORY, FAILED_ON, FAILED_KIND, FAILED_ACCOUNT],
    )?;
    let value = |key: &str| {
        drill
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    if value(CATEGORY).is_none() && value(FAILED_ON).is_none() {
        return Ok(None);
    }
    let view = pairs(query, &FILTERS)?;
    let view = reqwest::Url::parse_with_params("http://localhost/", &view)?;
    let trends =
        crate::logs::metrics::trends(config, &arguments(&view.query().map(str::to_owned))?)?;
    if let Some(category) = value(CATEGORY) {
        let ending = Ending::named(category)
            .ok_or_else(|| anyhow::anyhow!("unknown failure category: {category}"))?;
        return Ok(Some(
            trends
                .where_runs_failed
                .rows
                .iter()
                .filter(|row| {
                    row.ending == ending && row.stopped_at.as_deref() == value(STOPPED_AT)
                })
                .flat_map(Stopped::selectors)
                .cloned()
                .collect(),
        ));
    }
    Ok(Some(
        trends
            .outcomes
            .iter()
            .filter(|bucket| {
                value(FAILED_ON) == Some(bucket.bucket.to_string().as_str())
                    && value(FAILED_KIND) == Some(bucket.kind)
                    && value(FAILED_ACCOUNT) == bucket.account.as_deref()
            })
            .flat_map(|bucket| bucket.failed_commands.iter().cloned())
            .collect(),
    ))
}

/// Query parameters become the same `metrics` arguments the CLI parses. The
/// page's form sends empty fields, which mean the filter is not given.
fn arguments(query: &Option<String>) -> Result<MetricsArgs> {
    let mut arguments = vec!["metrics".to_owned(), "--json".to_owned()];
    if let Some(query) = query {
        for (key, value) in
            reqwest::Url::parse(&format!("http://localhost/?{query}"))?.query_pairs()
        {
            if value.is_empty() {
                continue;
            }
            match key.as_ref() {
                "command" => arguments.push(value.into_owned()),
                key if FILTERS.contains(&key) => {
                    arguments.push(format!("--{key}"));
                    arguments.push(value.into_owned());
                }
                _ => anyhow::bail!("unknown metrics argument: {key}"),
            }
        }
    }
    let crate::args::Command::Metrics(asked) = crate::args::parse(&arguments)? else {
        unreachable!("fixed metrics command")
    };
    Ok(asked)
}

/// `query` less its `machine` pair, which names the page's scope, not a filter.
fn without_machine(query: &Option<String>) -> Result<Option<String>> {
    let Some(query) = query else {
        return Ok(None);
    };
    let url = reqwest::Url::parse(&format!("http://localhost/?{query}"))?;
    Ok(Some(serde_urlencoded::to_string(
        url.query_pairs()
            .filter(|(key, _)| key != MACHINE)
            .collect::<Vec<_>>(),
    )?))
}

pub(super) async fn show(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
    RawQuery(query): RawQuery,
) -> Response {
    if format == super::api::Format::Json
        && pairs(&query, &[MACHINE, FLEET]).map_or(true, |pairs| !pairs.is_empty())
    {
        return super::api::error(
            StatusCode::BAD_REQUEST,
            "Fleet metrics are an explicit client view; call each owning machine's API directly",
        );
    }
    let machine = super::query_value(&query, MACHINE);
    super::api::read(format, move || {
        let config = UserConfig::load()?;
        let query = without_machine(&query)?;
        let (asked, error) = match arguments(&query) {
            Ok(asked) => (asked, None),
            // The page shows a refused filter beside the unfiltered view.
            Err(error) if format == super::api::Format::Html => {
                (MetricsArgs::default(), Some(format!("{error:#}")))
            }
            Err(error) => return Err(error),
        };
        if asked.command.is_some() {
            return format.render(crate::logs::metrics::read(&config, &asked)?, |value| {
                Ok(format!(
                    "<pre>{}</pre>",
                    super::filters::escaped(&serde_json::to_string_pretty(&value)?)
                ))
            });
        }
        let local = super::fleet_control::local_name(&config);
        // The API never reaches this point with a `machine`: it stays local.
        let fleet = format == super::api::Format::Html
            && fleet::default_view(&config, viewer.fleet_person());
        let machines = Machines {
            chosen: match machine.as_str() {
                "" if fleet => None,
                "" => Some(local.clone()),
                chosen => Some(chosen.to_owned()),
            },
            fleet,
            others: fleet::others(&config, viewer.fleet_person()),
            local,
        };
        if !machines.is(&machines.local) {
            let mut asked = asked;
            asked.machine = machines.chosen.clone();
            let mut cli = vec!["metrics".to_owned()];
            match &asked.machine {
                Some(name) => cli.extend(["--machine".to_owned(), name.clone()]),
                None => cli.push("--fleet".to_owned()),
            }
            cli.extend(
                pairs(&query, &FILTERS)?
                    .into_iter()
                    .flat_map(|(key, value)| [format!("--{key}"), value]),
            );
            cli.push("--json".into());
            let fleet = fleet::load(&config, &asked, viewer.fleet_person())?;
            return format.render(fleet, |fleet| {
                Ok(FleetPage {
                    page: super::PageContext::new(&config, &viewer.scopes),
                    fleet,
                    error,
                    form: Form::of(&asked),
                    kinds: crate::logs::metrics::COUNTED.map(crate::logs::display_name),
                    windows: MetricsWindow::NAMES,
                    classes: Class::ENDED,
                    machines,
                    cli: crate::args::shell_command(cli),
                }
                .render()?)
            });
        }
        // A refused query shows the unfiltered view, so its links carry none.
        let view = if error.is_none() {
            pairs(&query, &FILTERS)?
        } else {
            Vec::new()
        };
        format.render(crate::logs::metrics::trends(&config, &asked)?, |trends| {
            Ok(MetricsPage {
                page: super::PageContext::new(&config, &viewer.scopes),
                trends,
                error,
                form: Form::of(&asked),
                kinds: crate::logs::metrics::COUNTED.map(crate::logs::display_name),
                windows: MetricsWindow::NAMES,
                classes: Class::ENDED,
                view,
                machines,
            }
            .render()?)
        })
    })
    .await
}

/// The largest metrics report a machine may push: a summary lists each
/// Failed Command in its window.
pub(super) const REPORT_LIMIT: usize = 4 * 1024 * 1024;

/// The receiving end of `metrics --push`: save the summary a `report` token's
/// holder pushed in that machine's own reported slot. Any other caller, or a
/// report naming another machine, is refused.
pub(super) async fn receive(Extension(viewer): Extension<Viewer>, body: Bytes) -> Response {
    let super::identity::Identity::Token { holder, .. } = viewer.identity else {
        return super::api::error(StatusCode::FORBIDDEN, "Missing scope: report");
    };
    // Through a Value, as Fleet reads are: a flattened tally's rate cannot
    // be read straight from the bytes.
    let report = serde_json::from_slice::<serde_json::Value>(&body)
        .and_then(serde_json::from_value::<fleet::Report>);
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            return super::api::error(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("invalid metrics report: {error}"),
            );
        }
    };
    let result = tokio::task::spawn_blocking(move || -> Result<_> {
        let config = UserConfig::load()?;
        let now = chrono::Utc::now();
        report.receive(&config.logs_dir, &holder, now)?;
        Ok(serde_json::json!({"machine": holder, "pushed_at": now}))
    })
    .await;
    match result {
        Ok(Ok(receipt)) => axum::Json(receipt).into_response(),
        Ok(Err(error)) => match error.downcast_ref::<fleet::Refusal>() {
            Some(refusal @ fleet::Refusal::Mismatch) => {
                super::api::error(StatusCode::FORBIDDEN, refusal)
            }
            Some(refusal) => super::api::error(StatusCode::UNPROCESSABLE_ENTITY, refusal),
            None => {
                eprintln!("thirdshift Console: metrics report not saved: {error:?}");
                super::api::error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Could not save the metrics report.",
                )
            }
        },
        Err(error) => {
            eprintln!("thirdshift Console: metrics report not saved: {error:?}");
            super::api::error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not save the metrics report.",
            )
        }
    }
}
