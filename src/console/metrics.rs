//! Metrics: outcome stacks and signals over time, the twin of `metrics`.
use anyhow::Result;
use askama::Template;
use axum::Extension;
use axum::extract::RawQuery;
use axum::response::Response;

use super::auth::Viewer;
use crate::args::{MetricsArgs, MetricsWindow};
use crate::config::UserConfig;
use crate::logs::metrics::{Class, Trends};

/// Kinds the Kind filter offers: the work Commands, then Pickup runs.
const KINDS: [&str; 9] = [
    "Ticket run",
    "Spec run",
    "Base fix",
    "Review run",
    "Fix run",
    "Triage run",
    "Architect run",
    "Security run",
    "Pickup run",
];

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
}

/// The filters as asked, to fill the form again. Each shows its first value.
#[derive(Default)]
struct Form {
    window: String,
    grain: String,
    from: String,
    through: String,
    repo: String,
    kind: String,
    harness: String,
    model: String,
    account: String,
}

impl Form {
    fn of(asked: &MetricsArgs) -> Self {
        let first = |values: &[String]| values.first().cloned().unwrap_or_default();
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
            repo: first(&asked.repositories),
            kind: first(&asked.kinds),
            harness: first(&asked.harnesses),
            model: first(&asked.models),
            account: first(&asked.accounts),
        }
    }
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
                "window" | "grain" | "from" | "through" | "repo" | "kind" | "harness" | "model"
                | "account" => {
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

pub(super) async fn show(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
    RawQuery(query): RawQuery,
) -> Response {
    super::api::read(format, move || {
        let config = UserConfig::load()?;
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
        format.render(crate::logs::metrics::trends(&config, &asked)?, |trends| {
            Ok(MetricsPage {
                page: super::PageContext::new(&config, &viewer.scopes),
                trends,
                error,
                form: Form::of(&asked),
                kinds: KINDS,
                windows: MetricsWindow::NAMES,
                classes: Class::ENDED,
            }
            .render()?)
        })
    })
    .await
}
