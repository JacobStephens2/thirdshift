//! The machine's Console. Async execution is confined to this module.
mod account_commands;
mod accounts;
mod auth;
mod conversation;
pub use account_commands::{password, print_log, sign_out};
mod install;
pub use install::install;
mod live;
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
use axum::extract::{DefaultBodyLimit, Path, RawQuery};
use axum::http::{Method, StatusCode, header};
use axum::middleware;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use include_dir::{Dir, include_dir};

use crate::config::UserConfig;
use crate::logs::status::{self, Inbox, Row};

static ASSETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/console/assets");

// All page/action routes are registered here, beside their CLI equivalent.
// Static resources carry no view or action and have their own fallback route.
struct PageRoute {
    path: &'static str,
    cli: &'static [&'static str],
    method: Method,
    view: View,
}

enum View {
    Inbox,
    Inspect,
    Log,
    Changes,
    Logs,
    CommandLogs,
    Reader,
    LogStream,
    ConsoleLog,
    Account,
    SignOut,
    SignInPage,
    SignIn,
    SetPasswordPage,
    SendCode,
    SetPassword,
}

const PAGES: &[PageRoute] = &[
    PageRoute {
        path: "/set-password",
        cli: &["thirdshift console password"],
        method: Method::GET,
        view: View::SetPasswordPage,
    },
    PageRoute {
        path: "/set-password/code",
        cli: &["thirdshift console password"],
        method: Method::POST,
        view: View::SendCode,
    },
    PageRoute {
        path: "/set-password",
        cli: &["thirdshift console password"],
        method: Method::POST,
        view: View::SetPassword,
    },
    PageRoute {
        path: "/waterfall",
        cli: &[
            "thirdshift show widgets#7 --json",
            "thirdshift logs widgets#7",
        ],
        method: Method::GET,
        view: View::Inbox,
    },
    PageRoute {
        path: "/lanes",
        cli: &["thirdshift status --all --json"],
        method: Method::GET,
        view: View::Inbox,
    },
    PageRoute {
        path: "/stages",
        cli: &["thirdshift status --all --json"],
        method: Method::GET,
        view: View::Inbox,
    },
    PageRoute {
        path: "/",
        cli: &["thirdshift status"],
        method: Method::GET,
        view: View::Inbox,
    },
    PageRoute {
        path: "/events",
        cli: &["thirdshift status"],
        method: Method::GET,
        view: View::Changes,
    },
    PageRoute {
        path: "/history",
        cli: &["thirdshift status --all --json"],
        method: Method::GET,
        view: View::Inbox,
    },
    PageRoute {
        path: "/security",
        cli: &["thirdshift status --all --json"],
        method: Method::GET,
        view: View::Inbox,
    },
    PageRoute {
        path: "/peek",
        cli: &["thirdshift show widgets#7 --json"],
        method: Method::GET,
        view: View::Inspect,
    },
    PageRoute {
        path: "/log",
        cli: &["thirdshift logs widgets#7"],
        method: Method::GET,
        view: View::Log,
    },
    PageRoute {
        path: "/logs",
        cli: &["thirdshift logs --json"],
        method: Method::GET,
        view: View::Logs,
    },
    PageRoute {
        path: "/logs/command/{id}",
        cli: &["thirdshift logs --command id --json"],
        method: Method::GET,
        view: View::CommandLogs,
    },
    PageRoute {
        path: "/logs/{id}",
        cli: &["thirdshift logs --file id"],
        method: Method::GET,
        view: View::Reader,
    },
    PageRoute {
        path: "/logs/{id}/stream/{offset}",
        cli: &["thirdshift logs --file id --follow"],
        method: Method::GET,
        view: View::LogStream,
    },
    PageRoute {
        path: "/console-log",
        cli: &["thirdshift console log"],
        method: Method::GET,
        view: View::ConsoleLog,
    },
    PageRoute {
        path: "/account",
        cli: &["thirdshift console sign-out"],
        method: Method::GET,
        view: View::Account,
    },
    PageRoute {
        path: "/sign-out",
        cli: &["thirdshift console sign-out"],
        method: Method::POST,
        view: View::SignOut,
    },
    // CLI account access is authorized by the terminal's Unix account. HTTP
    // establishes that access with the password set by this paired command.
    PageRoute {
        path: "/sign-in",
        cli: &["thirdshift console password"],
        method: Method::GET,
        view: View::SignInPage,
    },
    PageRoute {
        path: "/sign-in",
        cli: &["thirdshift console password"],
        method: Method::POST,
        view: View::SignIn,
    },
];

pub fn serve(no_sign_in: bool) -> Result<()> {
    let config = UserConfig::load()?;
    ensure!(
        no_sign_in || !config.console.allowed_emails.is_empty(),
        "console.allowed_emails is empty; configure allowed addresses or use --no-sign-in on localhost"
    );
    if !no_sign_in {
        accounts::Accounts::load()?;
    }
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
        let (changes, observer) = live::observe(render_inbox("thirdshift status").await?);
        axum::serve(
            listener,
            router(no_sign_in, changes).layer(axum::Extension(executable.clone())),
        )
        .with_graceful_shutdown(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                if watched.changed() {
                    break;
                }
                // Revoke removed accounts even when nobody makes a request,
                // so re-adding an address cannot revive its old sessions.
                if !no_sign_in {
                    match tokio::task::spawn_blocking(|| accounts::Accounts::load().map(drop)).await
                    {
                        Ok(Ok(())) => {}
                        error => {
                            eprintln!("thirdshift Console: can't reconcile allowlist: {error:?}")
                        }
                    }
                }
            }
            // Dropping the sole sender closes every SSE response before
            // graceful shutdown waits for the connections to finish.
            observer.abort();
            let _ = observer.await;
        })
        .await
        .context("Console server failed")?;
        Ok::<_, anyhow::Error>(())
    })?;
    // Drop the runtime (and its sockets) before replacing the process.
    drop(runtime);
    executable.exec()
}

fn router(no_sign_in: bool, changes: tokio::sync::watch::Receiver<u64>) -> Router {
    let mut router = Router::new();
    for page in PAGES {
        let cli = page.cli[0];
        let path = page.path;
        let handler = match (&page.view, page.method.clone()) {
            (View::Inbox, Method::GET) => get(move |query| inbox(cli, path, query)),
            (View::Inspect, Method::GET) => get(inspect),
            (View::Log, Method::GET) => get(log),
            (View::Changes, Method::GET) => get(live::events),
            (View::Logs, Method::GET) => get(logs::index),
            (View::CommandLogs, Method::GET) => get(logs::command),
            (View::Reader, Method::GET) => get(logs::reader),
            (View::LogStream, Method::GET) => get(logs::stream),
            (View::ConsoleLog, Method::GET) => get(auth::log),
            (View::Account, Method::GET) => get(auth::account),
            (View::SignOut, Method::POST) => post(auth::sign_out),
            (View::SignInPage, Method::GET) => get(auth::sign_in_page),
            (View::SignIn, Method::POST) => post(auth::sign_in),
            (View::SetPasswordPage, Method::GET) => get(auth::set_password_page),
            (View::SendCode, Method::POST) => post(auth::code),
            (View::SetPassword, Method::POST) => post(auth::set_password),
            _ => unreachable!("unknown Console page method"),
        };
        router = router.route(page.path, handler);
    }
    router = router.route("/assets/{*path}", get(asset));
    let access = auth::Access::new(no_sign_in);
    router
        .with_state(changes)
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn_with_state(access.clone(), auth::guard))
        .layer(axum::Extension(access))
        .layer(middleware::from_fn(auth::headers))
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
    fn needs_you(&self) -> usize {
        self.inbox.sections[0].rows.len()
    }

    fn urgent(&self) -> bool {
        self.inbox.sections[0]
            .rows
            .iter()
            .any(|row| row.state != "Slow")
    }

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
    let mut requested_page = query_value(&query, "page");
    if requested_page.is_empty() {
        requested_page = path.trim_start_matches('/').into();
    }
    if matches!(requested_page.as_str(), "logs" | "6") {
        return logs::index().await;
    }
    if matches!(requested_page.as_str(), "waterfall" | "4") {
        return waterfall::show(query).await;
    }
    if matches!(requested_page.as_str(), "lanes" | "2" | "stages" | "3") {
        return time::show(matches!(requested_page.as_str(), "stages" | "3"), query).await;
    }
    let archive_page = match requested_page.as_str() {
        "history" | "5" => Some(status::ArchivePage::History),
        "security" | "7" => Some(status::ArchivePage::Security),
        "" | "inbox" | "1" => None,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    page(Ok(render_page(cli, archive_page, query).await))
}

async fn render_page(
    cli: &'static str,
    archive_page: Option<status::ArchivePage>,
    query: Option<String>,
) -> Result<String> {
    // The shared observations and templates stay blocking, off the async executor.
    tokio::task::spawn_blocking(move || -> Result<String> {
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
    .await?
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
        Ok(Ok(html)) => Html(html).into_response(),
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

fn page(result: std::result::Result<Result<String>, tokio::task::JoinError>) -> Response {
    match result {
        Ok(Ok(html)) => Html(html).into_response(),
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

async fn render_inbox(cli: &'static str) -> Result<String> {
    render_page(cli, None, None).await
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
        Some("svg") => "image/svg+xml",
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
        routing::{Route, get, post},
    };
    use std::sync::{Arc, Mutex};
    use tower_layer::Layer;

    fn router() -> Router {
        let (_, changes) = tokio::sync::watch::channel(0);
        super::router(false, changes)
    }

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
        // interface after attaching state, as the production router does, so
        // this does not depend on its implicit HEAD/fallback count.
        let (_, changes) = tokio::sync::watch::channel(0_u64);
        let get_methods = registered_methods(
            Router::<tokio::sync::watch::Receiver<u64>>::new()
                .route("/known", get(|| async {}))
                .with_state(changes.clone()),
        );
        let post_methods = registered_methods(
            Router::<tokio::sync::watch::Receiver<u64>>::new()
                .route("/known", post(|| async {}))
                .with_state(changes),
        );
        // Each unique path has one fallback; merging GET and POST at sign-in
        // and set-password adds only the POST services, not more fallbacks.
        let fallback = 1;
        let expected = super::PAGES
            .iter()
            .map(|page| {
                if page.method == axum::http::Method::GET {
                    get_methods
                } else {
                    post_methods
                }
            })
            .sum::<usize>()
            + get_methods
            - 2 * fallback;
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
        assert_cli_parity(router());
    }

    #[test]
    #[should_panic(expected = "registered routes must all have a CLI pair")]
    fn parity_rejects_a_route_registered_without_a_cli_pair() {
        assert_cli_parity(router().route("/unpaired", get(|| async { "page with no CLI pair" })));
    }
}
