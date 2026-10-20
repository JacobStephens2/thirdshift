//! The machine's Console. Async execution is confined to this module.
mod conversation;
mod logs;
mod peek;
mod restart;
mod time;
mod waterfall;

use std::io::Write;
use std::net::Ipv4Addr;

use anyhow::{Context, Result, ensure};
use askama::Template;
use axum::Router;
use axum::extract::{Path, RawQuery};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use include_dir::{Dir, include_dir};

use crate::config::UserConfig;
use crate::logs::status::{self, Inbox, Row};

static ASSETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/console/assets");

// All page/action routes are registered here, beside their CLI equivalent.
// Static resources carry no view or action and have their own fallback route.
struct PageRoute {
    path: &'static str,
    cli: &'static [&'static str],
}

const PAGES: &[PageRoute] = &[
    PageRoute {
        path: "/waterfall",
        cli: &[
            "thirdshift show widgets#7 --json",
            "thirdshift logs widgets#7",
        ],
    },
    PageRoute {
        path: "/lanes",
        cli: &["thirdshift status --all --json"],
    },
    PageRoute {
        path: "/stages",
        cli: &["thirdshift status --all --json"],
    },
    PageRoute {
        path: "/",
        cli: &["thirdshift status"],
    },
    PageRoute {
        path: "/history",
        cli: &["thirdshift status --all --json"],
    },
    PageRoute {
        path: "/security",
        cli: &["thirdshift status --all --json"],
    },
    PageRoute {
        path: "/peek",
        cli: &["thirdshift show widgets#7 --json"],
    },
    PageRoute {
        path: "/log",
        cli: &["thirdshift logs widgets#7"],
    },
    PageRoute {
        path: "/logs",
        cli: &["thirdshift logs --json"],
    },
    PageRoute {
        path: "/logs/command/{id}",
        cli: &["thirdshift logs --command id --json"],
    },
    PageRoute {
        path: "/logs/{id}",
        cli: &["thirdshift logs --file id"],
    },
    PageRoute {
        path: "/logs/{id}/stream/{offset}",
        cli: &["thirdshift logs --file id --follow"],
    },
];

pub fn serve(no_sign_in: bool) -> Result<()> {
    ensure!(
        no_sign_in,
        "Console sign-in is not available yet; use thirdshift console --no-sign-in on localhost"
    );
    let config = UserConfig::load()?;
    let executable = restart::Executable::current()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, config.console.port))
            .await
            .context("can't listen for the Console on localhost")?;
        println!("http://{}", listener.local_addr()?);
        std::io::stdout().flush()?;
        let watched = executable.clone();
        axum::serve(
            listener,
            router().layer(axum::Extension(executable.clone())),
        )
        .with_graceful_shutdown(async move { watched.replaced().await })
        .await
        .context("Console server failed")?;
        Ok::<_, anyhow::Error>(())
    })?;
    // Drop the runtime (and its sockets) before replacing the process.
    drop(runtime);
    executable.exec()
}

fn router() -> Router {
    let mut router = Router::new();
    for page in PAGES {
        let cli = page.cli[0];
        router = match page.path {
            "/peek" => router.route(page.path, get(inspect)),
            "/log" => router.route(page.path, get(log)),
            "/logs" => router.route(page.path, get(logs::index)),
            "/logs/command/{id}" => router.route(page.path, get(logs::command)),
            "/logs/{id}" => router.route(page.path, get(logs::reader)),
            "/logs/{id}/stream/{offset}" => router.route(page.path, get(logs::stream)),
            path => router.route(path, get(move |query| inbox(cli, path, query))),
        };
    }
    router = router.route("/assets/{*path}", get(asset));
    router
}

#[derive(Template)]
#[template(path = "console/inbox.html")]
struct InboxPage {
    hostname: String,
    consoles: Vec<String>,
    theme: String,
    cli: &'static str,
    inbox: Inbox,
    title: &'static str,
    archive: Option<status::Archive>,
    outcome: String,
    repository: String,
}

impl InboxPage {
    fn row<'a>(&self, row: &'a Row) -> RowTemplate<'a> {
        RowTemplate { row }
    }
}

#[derive(Template)]
#[template(path = "console/row.html")]
struct RowTemplate<'a> {
    row: &'a Row,
}

impl RowTemplate<'_> {
    fn child<'a>(&self, row: &'a Row) -> RowTemplate<'a> {
        RowTemplate { row }
    }
}

fn query_value(query: &Option<String>, key: &str) -> String {
    let url = reqwest::Url::parse(&format!(
        "http://localhost/?{}",
        query.as_deref().unwrap_or_default()
    ))
    .expect("fixed URL");
    url.query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}

async fn inbox(cli: &'static str, path: &'static str, RawQuery(query): RawQuery) -> Response {
    let mut page = query_value(&query, "page");
    if page.is_empty() {
        page = path.trim_start_matches('/').into();
    }
    if matches!(page.as_str(), "logs" | "6") {
        return logs::index().await;
    }
    if matches!(page.as_str(), "waterfall" | "4") {
        return waterfall::show(query).await;
    }
    if matches!(page.as_str(), "lanes" | "2" | "stages" | "3") {
        return time::show(matches!(page.as_str(), "stages" | "3"), query).await;
    }
    let archive_page = match page.as_str() {
        "history" | "5" => Some(status::ArchivePage::History),
        "security" | "7" => Some(status::ArchivePage::Security),
        "" | "inbox" | "1" => None,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    // The shared observations and templates stay blocking, off the async executor.
    match tokio::task::spawn_blocking(move || -> Result<String> {
        let config = UserConfig::load()?;
        let outcome = query_value(&query, "outcome");
        let repository = query_value(&query, "repository");
        let (title, cli) = match archive_page {
            Some(status::ArchivePage::History) => ("History", "thirdshift status --all --json"),
            Some(status::ArchivePage::Security) => (
                "Security",
                "thirdshift status --all --json · thirdshift show <command> --json",
            ),
            None => ("Inbox", cli),
        };
        Ok(InboxPage {
            hostname: crate::host::name().unwrap_or_else(|| "unknown".into()),
            consoles: config.console.other_consoles.clone(),
            theme: config.console.theme.clone(),
            cli,
            inbox: status::inbox(&config)?,
            title,
            archive: archive_page
                .map(|page| status::archive(&config, page, &outcome, &repository))
                .transpose()?,
            outcome,
            repository,
        }
        .render()?)
    })
    .await
    {
        Ok(Ok(html)) => html_response(html),
        error => {
            eprintln!("thirdshift Console: can't read Inbox: {error:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Can't read the Inbox; check the Console's stderr.",
            )
                .into_response()
        }
    }
}

async fn inspect(RawQuery(query): RawQuery) -> Response {
    let command = query_value(&query, "command");
    match tokio::task::spawn_blocking(move || -> Result<String> {
        let config = UserConfig::load()?;
        let details = status::inspect::details(&config, &command)?;
        Ok(peek::Peek::new(command, details).render()?)
    })
    .await
    {
        Ok(Ok(html)) => html_response(html),
        _ => (StatusCode::NOT_FOUND, "Command details are unavailable.").into_response(),
    }
}

async fn log(RawQuery(query): RawQuery) -> Response {
    let command = query_value(&query, "command");
    let session = query_value(&query, "session");
    match tokio::task::spawn_blocking(move || {
        let config = UserConfig::load()?;
        let path = status::inspect::log_path(
            &config,
            &command,
            (!session.is_empty()).then_some(session.as_str()),
        )?;
        crate::logs::catalog::load(&config, None)?
            .into_iter()
            .find(|log| log.path == path)
            .map(|log| log.id)
            .context("log is unavailable")
    })
    .await
    {
        Ok(Ok(id)) => logs::reader(Path(id)).await,
        _ => (StatusCode::NOT_FOUND, "Log is unavailable.").into_response(),
    }
}

fn html_response(html: String) -> Response {
    (
        [(header::CONTENT_SECURITY_POLICY, "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"),
         (header::CACHE_CONTROL, "no-store"),
         (header::X_CONTENT_TYPE_OPTIONS, "nosniff")], Html(html)
    ).into_response()
}

fn page(result: std::result::Result<Result<String>, tokio::task::JoinError>) -> Response {
    match result {
        Ok(Ok(html)) => html_response(html),
        error => {
            eprintln!("thirdshift Console: can't read page: {error:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Can't read this page; check the Console's stderr.",
            )
                .into_response()
        }
    }
}

async fn asset(Path(path): Path<String>) -> Response {
    let Some(file) = ASSETS.get_file(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = match std::path::Path::new(&path)
        .extension()
        .and_then(|ext| ext.to_str())
    {
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("woff2") => "font/woff2",
        _ => "text/plain; charset=utf-8",
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        file.contents(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        routing::{Route, get},
    };
    use std::sync::{Arc, Mutex};
    use tower_layer::Layer;

    // Router::route_layer walks every registered method service, including
    // routes added directly instead of through PAGES. Nothing is dispatched.
    #[derive(Clone)]
    struct Routes(Arc<Mutex<Vec<Route>>>);

    impl Layer<Route> for Routes {
        type Service = Route;
        fn layer(&self, route: Route) -> Route {
            self.0.lock().unwrap().push(route.clone());
            route
        }
    }

    fn registered_methods(router: Router) -> usize {
        let routes = Routes(Arc::default());
        let _router = router.route_layer(routes.clone());
        routes.0.lock().unwrap().len()
    }

    fn assert_cli_parity(router: Router) {
        // Measure the GET registration footprint through axum's public layer
        // interface, so this does not depend on its implicit HEAD/fallback count.
        let get_methods = registered_methods(Router::new().route("/known", get(|| async {})));
        let expected = (super::PAGES.len() + 1) * get_methods; // one static-resource route
        assert_eq!(
            registered_methods(router),
            expected,
            "registered routes must all have a CLI pair; only static resources are exempt"
        );
        for route in super::PAGES {
            assert!(
                !route.cli.is_empty(),
                "{} has no paired CLI command",
                route.path
            );
            for cli in route.cli {
                assert!(cli.starts_with("thirdshift "), "{cli}");
                let arguments = cli
                    .split_whitespace()
                    .skip(1)
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                assert!(
                    crate::args::parse(&arguments).is_ok(),
                    "{} is not a CLI command",
                    cli
                );
            }
        }
    }

    #[test]
    fn every_console_route_has_a_cli_equivalent() {
        assert_cli_parity(super::router());
    }

    #[test]
    #[should_panic(expected = "registered routes must all have a CLI pair")]
    fn parity_rejects_a_route_registered_without_a_cli_pair() {
        assert_cli_parity(
            super::router().route("/unpaired", get(|| async { "page with no CLI pair" })),
        );
    }
}
