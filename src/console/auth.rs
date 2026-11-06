//! HTTP sign-in, session authorization and CSRF for every Console action.
use anyhow::Result;
use askama::Template;

use super::filters;
use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Extension, Form};
use serde::Deserialize;
use subtle::ConstantTimeEq;

use super::Scope;
use super::accounts::{self, Accounts, SignIn};
use super::identity::Identity;
use crate::config::UserConfig;

mod password;
mod rate_limit;
mod reply;
pub(super) use password::{code, set_password, set_password_page};
use reply::Reply;

const SESSION: &str = "__Host-thirdshift-session";
const SIGN_IN: &str = "__Host-thirdshift-sign-in";
pub(super) const PERSON_HEADER: &str = "x-thirdshift-person";

#[derive(Clone)]
pub(super) struct Access {
    pub no_sign_in: bool,
    local_csrf: String,
    limiter: rate_limit::Limiter,
}

impl Access {
    pub fn new(no_sign_in: bool) -> Self {
        Self {
            no_sign_in,
            local_csrf: accounts::random_token(),
            limiter: rate_limit::Limiter::default(),
        }
    }
}

#[derive(Clone)]
pub(super) struct Viewer {
    pub caller_label: String,
    pub scopes: Vec<Scope>,
    pub identity: Identity,
    token: Option<String>,
    pub(super) csrf: String,
}

impl Viewer {
    pub fn fleet_person(&self) -> Option<&str> {
        match &self.identity {
            Identity::Session { address } => Some(address),
            Identity::Token { person, .. } => person.as_deref(),
            Identity::Local { .. } => None,
        }
    }

    pub fn activity_actor(&self) -> crate::logs::Actor {
        let (who, holder) = match &self.identity {
            Identity::Token {
                holder,
                person: Some(person),
            } => (person.clone(), Some(holder.clone())),
            Identity::Token {
                holder,
                person: None,
            } => (format!("token {holder}"), None),
            Identity::Session { address } => (address.clone(), None),
            Identity::Local { .. } => (self.caller_label.clone(), None),
        };
        crate::logs::Actor::console(who, holder)
    }

    // Streams recheck on account changes without extending idle lifetime.
    pub fn still_allowed(&self) -> Result<bool> {
        match &self.token {
            Some(token) => {
                let Some(email) = Accounts::load()?.session(token, false)? else {
                    return Ok(false);
                };
                Ok(UserConfig::load()?.console.scopes_for(&email) == self.scopes)
            }
            None => Ok(true),
        }
    }
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .flat_map(|header| header.split(';'))
        .find_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == name && value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .then(|| value.to_owned())
        })
}

fn csrf(token: &str) -> String {
    accounts::digest(&format!("csrf:{token}"))
}

fn valid_csrf(given: &str, expected: &str) -> bool {
    bool::from(given.as_bytes().ct_eq(expected.as_bytes()))
}

fn set_cookie(response: &mut Response, name: &str, value: &str, age: i64) {
    response.headers_mut().append(
        header::SET_COOKIE,
        format!("{name}={value}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={age}")
            .parse()
            .expect("generated cookie"),
    );
}

// Only the installed loopback proxy may attest HTTPS. A direct localhost
// exception must carry no forwarded/proxy headers at all.
fn bearer_transport(request: &Request) -> bool {
    let local = request
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .is_some_and(|peer| peer.0.ip() == std::net::Ipv4Addr::LOCALHOST);
    if !local {
        return false;
    }
    let headers = request.headers();
    if headers.get_all("x-forwarded-proto").iter().count() == 1
        && headers
            .get("x-forwarded-proto")
            .is_some_and(|value| value == "https")
    {
        return true;
    }
    !headers.keys().any(|name| {
        name.as_str().starts_with("x-forwarded-")
            || matches!(
                name.as_str(),
                "forwarded" | "x-real-ip" | "via" | "proxy-connection"
            )
    })
}

async fn bearer_refused(limiter: &rate_limit::Limiter, address: std::net::IpAddr) -> Response {
    match limiter.failed(address).await {
        Some(retry) => rate_limit::refusal(retry),
        None => super::api::error(StatusCode::UNAUTHORIZED, "Sign in to the Console."),
    }
}

pub(super) async fn guard(State(access): State<Access>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let api = super::api::is_path(path);
    let unauthenticated = || {
        if api {
            super::api::error(StatusCode::UNAUTHORIZED, "Sign in to the Console.")
        } else {
            Redirect::to("/sign-in").into_response()
        }
    };
    if matches!(path, "/sign-in" | "/set-password" | "/set-password/code")
        || path.starts_with("/assets/")
    {
        return next.run(request).await;
    }
    let mut viewer = if api && request.headers().contains_key(header::AUTHORIZATION) {
        if !bearer_transport(&request) {
            return unauthenticated();
        }
        let peer = request
            .extensions()
            .get::<ConnectInfo<std::net::SocketAddr>>()
            .expect("bearer transport requires a connected peer")
            .0;
        let address = rate_limit::address(peer, request.headers());
        if let Some(retry) = access.limiter.retry_after(address) {
            return rate_limit::refusal(retry);
        }
        let headers = request.headers();
        if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
            return bearer_refused(&access.limiter, address).await;
        }
        let Some(secret) = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split_once(' '))
            .filter(|(scheme, secret)| {
                scheme.eq_ignore_ascii_case("bearer")
                    && !secret.is_empty()
                    && !secret.chars().any(char::is_whitespace)
            })
            .map(|(_, secret)| secret.to_owned())
        else {
            return bearer_refused(&access.limiter, address).await;
        };
        match tokio::task::spawn_blocking(move || super::tokens::authenticate(&secret)).await {
            Ok(Ok(Some(token))) => Viewer {
                identity: Identity::Token {
                    holder: token.holder.clone(),
                    person: None,
                },
                caller_label: token.holder,
                scopes: token.scopes,
                token: None,
                csrf: String::new(),
            },
            Ok(Ok(None)) => {
                return bearer_refused(&access.limiter, address).await;
            }
            error => {
                eprintln!("thirdshift Console: API token store read failed: {error:?}");
                return super::api::error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Can't read Console tokens.",
                );
            }
        }
    } else if access.no_sign_in {
        Viewer {
            caller_label: "localhost (no sign-in)".into(),
            scopes: Scope::ALL.to_vec(),
            identity: Identity::local(),
            token: None,
            csrf: access.local_csrf,
        }
    } else {
        let Some(token) = cookie(request.headers(), SESSION) else {
            return unauthenticated();
        };
        let selected = token.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<_> {
            Accounts::load()?
                .session(&selected, true)?
                .map(|email| {
                    let scopes = UserConfig::load()?.console.scopes_for(&email);
                    Ok((email, scopes))
                })
                .transpose()
        })
        .await;
        match result {
            Ok(Ok(Some((email, scopes)))) => Viewer {
                identity: Identity::Session {
                    address: email.clone(),
                },
                caller_label: email,
                scopes,
                csrf: csrf(&token),
                token: Some(token),
            },
            Ok(Ok(None)) => return unauthenticated(),
            error if api => {
                eprintln!("thirdshift Console: API account read failed: {error:?}");
                return super::api::error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Can't read Console accounts.",
                );
            }
            error => return failed(error),
        }
    };
    let mutation = api
        && !matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        );
    let endpoint = request.uri().path().to_owned();
    let refusal = name_person(&mut viewer, request.headers()).await.err();
    let mut response = if let Some(refusal) = refusal {
        *refusal
    } else {
        authorized(&viewer, request, next).await
    };
    if mutation {
        let actor = viewer.caller_label.clone();
        let outcome = response.status().as_u16();
        match tokio::task::spawn_blocking(move || {
            Accounts::load()?.record_api(&actor, endpoint, outcome)
        })
        .await
        {
            Ok(Ok(())) => {}
            failure => {
                eprintln!("thirdshift Console: could not audit API call: {failure:?}");
                return super::api::error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Could not record the API call's outcome.",
                );
            }
        }
    }
    if let Some(token) = viewer.token
        && !response.headers().contains_key(header::SET_COOKIE)
    {
        set_cookie(&mut response, SESSION, &token, accounts::LIFETIME);
    }
    response
}

async fn name_person(
    viewer: &mut Viewer,
    headers: &HeaderMap,
) -> std::result::Result<(), Box<Response>> {
    let Identity::Token { holder, person } = &mut viewer.identity else {
        return Ok(());
    };
    if !headers.contains_key(PERSON_HEADER) {
        return Ok(());
    }
    let selected = headers
        .get(PERSON_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && headers.get_all(PERSON_HEADER).iter().count() == 1)
        .ok_or_else(|| {
            super::api::error(
                StatusCode::FORBIDDEN,
                "Name exactly one person in X-Thirdshift-Person.",
            )
        })
        .map_err(Box::new)?
        .to_owned();
    let email = selected.clone();
    let scopes = tokio::task::spawn_blocking(move || -> Result<_> {
        let console = UserConfig::load()?.console;
        Ok(console
            .allowed_emails
            .contains(&email)
            .then(|| console.scopes_for(&email)))
    })
    .await;
    let scopes = match scopes {
        Ok(Ok(Some(scopes))) => scopes,
        Ok(Ok(None)) => {
            return Err(Box::new(super::api::error(
                StatusCode::FORBIDDEN,
                "Named person is not on this machine's Console sign-in list.",
            )));
        }
        error => {
            eprintln!("thirdshift Console: address scopes read failed: {error:?}");
            return Err(Box::new(super::api::error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Can't read Console address scopes.",
            )));
        }
    };
    viewer.scopes.retain(|scope| scopes.contains(scope));
    viewer.caller_label = format!("{selected} from the Console on {holder}");
    *person = Some(selected);
    Ok(())
}

// Attach to routed methods, so unsupported methods keep the router's 404/405.
pub(super) async fn csrf_guard(
    viewer: Option<Extension<Viewer>>,
    mut request: Request,
    next: Next,
) -> Response {
    // Anonymous sign-in/password forms validate their own sign-in cookie.
    let Some(Extension(viewer)) = viewer else {
        return next.run(request).await;
    };
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) && !matches!(viewer.identity, Identity::Token { .. })
    {
        let supplied = request
            .headers()
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let (parts, body) = request.into_parts();
        let Ok(bytes) = to_bytes(body, 16 * 1024).await else {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        };
        let form =
            serde_urlencoded::from_bytes::<Vec<(String, String)>>(&bytes).unwrap_or_default();
        let supplied = supplied.or_else(|| {
            form.into_iter()
                .find(|(key, _)| key == "csrf")
                .map(|(_, value)| value)
        });
        if !supplied.is_some_and(|token| valid_csrf(&token, &viewer.csrf)) {
            return StatusCode::FORBIDDEN.into_response();
        }
        request = Request::from_parts(parts, Body::from(bytes));
    }
    next.run(request).await
}

async fn authorized(viewer: &Viewer, mut request: Request, next: Next) -> Response {
    let api = super::api::is_path(request.uri().path());
    let withhold = api && !viewer.scopes.contains(&Scope::Security);
    request.extensions_mut().insert(viewer.clone());
    let mut response = next.run(request).await;
    // Read routes embed the same Command records as `/api/v1/security`.
    // Callers without that scope keep the rest of the CLI JSON, minus the
    // finding records ADR 0026 reserves for `security`.
    if withhold {
        response = withhold_security_records(response).await;
    }
    response
}

const SECURITY_RECORD_LIMIT: usize = 32 * 1024 * 1024;

async fn withhold_security_records(response: Response) -> Response {
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, SECURITY_RECORD_LIMIT).await else {
        return super::api::error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Can't scope this response.",
        );
    };
    let replacement = match redacted_security_records(&bytes) {
        Redaction::Unchanged => return Response::from_parts(parts, Body::from(bytes)),
        Redaction::Unreadable => {
            return super::api::error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Can't scope this response.",
            );
        }
        Redaction::Rewritten(body) => body,
    };
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_LENGTH,
        replacement
            .len()
            .to_string()
            .parse()
            .expect("content length"),
    );
    Response::from_parts(parts, Body::from(replacement))
}

enum Redaction {
    Unchanged,
    Rewritten(Vec<u8>),
    Unreadable,
}

fn redacted_security_records(bytes: &[u8]) -> Redaction {
    let mut value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => {
            return if security_record_marker(bytes) {
                Redaction::Unreadable
            } else {
                Redaction::Unchanged
            };
        }
    };
    if !strip_security_records(&mut value) {
        return Redaction::Unchanged;
    }
    match serde_json::to_vec(&value) {
        Ok(body) => Redaction::Rewritten(body),
        Err(_) => Redaction::Unreadable,
    }
}

fn security_record_marker(bytes: &[u8]) -> bool {
    const MARKERS: &[&[u8]] = &[
        b"\"security_findings\"",
        b"\"security_fix_findings\"",
        b"\"security_findings_waiting\"",
    ];
    MARKERS
        .iter()
        .any(|marker| bytes.windows(marker.len()).any(|window| window == *marker))
}

fn strip_security_records(value: &mut serde_json::Value) -> bool {
    let mut changed = false;
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                changed |= strip_security_records(item);
            }
        }
        serde_json::Value::Object(map) => {
            changed |= map.remove("security_findings").is_some();
            changed |= map.remove("security_fix_findings").is_some();
            if map
                .get("security_findings_waiting")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                map.insert(
                    "security_findings_waiting".to_string(),
                    serde_json::Value::Bool(false),
                );
                changed = true;
            }
            for child in map.values_mut() {
                changed |= strip_security_records(child);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
    changed
}

pub(super) async fn headers(request: Request, next: Next) -> Response {
    let api = super::api::is_path(request.uri().path());
    let mut response = next.run(request).await;
    if api {
        response = super::api::json_errors(response).await;
    }
    for (name, value) in [
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
        (header::CACHE_CONTROL, "no-store"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "same-origin"),
    ] {
        response.headers_mut().insert(name, value.parse().unwrap());
    }
    response
}

pub(super) async fn require_scope(
    State(scope): State<Scope>,
    viewer: Option<Extension<Viewer>>,
    request: Request,
    next: Next,
) -> Response {
    // The guard allows anonymous sign-in and password recovery forms.
    let Some(Extension(viewer)) = viewer else {
        return next.run(request).await;
    };
    if !viewer.scopes.contains(&scope) {
        return super::api::error(
            StatusCode::FORBIDDEN,
            format!("Missing scope: {}", scope.name()),
        );
    }
    next.run(request).await
}

#[derive(Template)]
#[template(path = "console/sign_in.html")]
struct SignInPage {
    csrf: String,
    theme: String,
    email_enabled: bool,
}

pub(super) async fn sign_in_page(
    Extension(access): Extension<Access>,
    headers: HeaderMap,
) -> Response {
    anonymous_page(
        access,
        headers,
        |AnonymousForm {
             csrf,
             theme,
             email_enabled,
         }| {
            Ok(SignInPage {
                csrf,
                theme,
                email_enabled,
            }
            .render()?)
        },
    )
    .await
}

struct AnonymousForm {
    csrf: String,
    theme: String,
    email_enabled: bool,
}

async fn anonymous_page(
    access: Access,
    headers: HeaderMap,
    render: impl FnOnce(AnonymousForm) -> Result<String> + Send + 'static,
) -> Response {
    if access.no_sign_in {
        return StatusCode::NOT_FOUND.into_response();
    }
    let token = cookie(&headers, SIGN_IN).unwrap_or_else(accounts::random_token);
    let form_token = csrf(&token);
    let mut response = super::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            render(AnonymousForm {
                csrf: form_token,
                theme: UserConfig::load()?.console.theme,
                email_enabled: crate::resend_key::ResendKey::find()?.is_some(),
            })
        })
        .await,
    );
    if response.status().is_success() {
        set_cookie(&mut response, SIGN_IN, &token, 15 * 60);
    }
    response
}

#[derive(Deserialize)]
pub(super) struct SignInForm {
    email: String,
    password: String,
    #[serde(default)]
    csrf: String,
}

pub(super) async fn sign_in(
    Extension(access): Extension<Access>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<SignInForm>,
) -> Response {
    if access.no_sign_in {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(token) = cookie(&headers, SIGN_IN) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !valid_csrf(&form.csrf, &csrf(&token)) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let address = rate_limit::address(peer, &headers);
    if let Some(retry) = access.limiter.retry_after(address) {
        return sign_in_limited(retry).await;
    }
    let result =
        tokio::task::spawn_blocking(move || Accounts::load()?.sign_in(&form.email, &form.password))
            .await;
    match result {
        Ok(Ok(SignIn::SignedIn(token))) => {
            let mut response = Redirect::to("/").into_response();
            set_cookie(&mut response, SESSION, &token, accounts::LIFETIME);
            response
        }
        Ok(Ok(SignIn::Refused)) => {
            if let Some(retry) = access.limiter.failed(address).await {
                return sign_in_limited(retry).await;
            }
            Reply::SignInRefused.response().await
        }
        Ok(Ok(SignIn::Slowdown)) => Reply::SignInSlowdown.response().await,
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error),
    }
}

async fn sign_in_limited(retry: u64) -> Response {
    let mut response = Reply::SignInSlowdown.response().await;
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, retry.into());
    response
}

#[derive(Template)]
#[template(path = "console/account.html")]
struct AccountPage {
    page: super::PageContext,
    email: String,
    csrf: String,
    signed_in: bool,
    theme: String,
    resources: super::resources::Status,
}

pub(super) async fn account(Extension(viewer): Extension<Viewer>) -> Response {
    super::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            let config = UserConfig::load()?;
            Ok(AccountPage {
                page: super::PageContext::new(&config, &viewer.scopes),
                email: viewer.caller_label,
                csrf: viewer.csrf,
                signed_in: viewer.token.is_some(),
                resources: super::resources::Status::new(&config),
                theme: config.console.theme,
            }
            .render()?)
        })
        .await,
    )
}

pub(super) async fn sign_out(Extension(viewer): Extension<Viewer>) -> Response {
    let Some(token) = viewer.token else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let result = tokio::task::spawn_blocking(move || {
        Accounts::load()?.sign_out(&viewer.caller_label, Some(&token))
    })
    .await;
    match result {
        Ok(Ok(())) => {
            let mut response = Redirect::to("/sign-in").into_response();
            set_cookie(&mut response, SESSION, "", 0);
            response
        }
        error => failed(error),
    }
}

#[derive(Template)]
#[template(path = "console/account_log.html")]
struct LogPage {
    page: super::PageContext,
    events: Vec<accounts::Event>,
    theme: String,
    resources: super::resources::Status,
}

pub(super) async fn log(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
) -> Response {
    super::api::read(format, move || {
        let config = UserConfig::load()?;
        format.render(Accounts::load()?.events()?, |events| {
            Ok(LogPage {
                page: super::PageContext::new(&config, &viewer.scopes),
                events,
                resources: super::resources::Status::new(&config),
                theme: config.console.theme,
            }
            .render()?)
        })
    })
    .await
}

fn failed(error: impl std::fmt::Debug) -> Response {
    eprintln!("thirdshift Console: account operation failed: {error:?}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Can't read Console accounts; check the Console's stderr.",
    )
        .into_response()
}
