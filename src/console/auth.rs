//! HTTP sign-in, session authorization and CSRF for every Console action.
use anyhow::Result;
use askama::Template;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Extension, Form};
use serde::Deserialize;
use subtle::ConstantTimeEq;

use super::accounts::{self, Accounts, SignIn};
use crate::config::UserConfig;

mod password;
pub(super) use password::{code, set_password, set_password_page};

const SESSION: &str = "__Host-thirdshift-session";
const SIGN_IN: &str = "__Host-thirdshift-sign-in";

#[derive(Clone)]
pub(super) struct Access {
    pub no_sign_in: bool,
    local_csrf: String,
}

impl Access {
    pub fn new(no_sign_in: bool) -> Self {
        Self {
            no_sign_in,
            local_csrf: accounts::random_token(),
        }
    }
}

#[derive(Clone)]
pub(super) struct Viewer {
    pub email: String,
    token: Option<String>,
    pub(super) csrf: String,
}

impl Viewer {
    // Streams recheck on account changes without extending idle lifetime.
    pub fn still_allowed(&self) -> Result<bool> {
        match &self.token {
            Some(token) => Ok(Accounts::load()?.session(token, false)?.is_some()),
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

pub(super) async fn guard(
    State(access): State<Access>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if matches!(path, "/sign-in" | "/set-password" | "/set-password/code")
        || path.starts_with("/assets/")
    {
        return next.run(request).await;
    }
    let viewer = if access.no_sign_in {
        Viewer {
            email: "localhost (no sign-in)".into(),
            token: None,
            csrf: access.local_csrf,
        }
    } else {
        let Some(token) = cookie(request.headers(), SESSION) else {
            return Redirect::to("/sign-in").into_response();
        };
        let selected = token.clone();
        let result =
            tokio::task::spawn_blocking(move || Accounts::load()?.session(&selected, true)).await;
        match result {
            Ok(Ok(Some(email))) => Viewer {
                email,
                csrf: csrf(&token),
                token: Some(token),
            },
            Ok(Ok(None)) => return Redirect::to("/sign-in").into_response(),
            error => return failed(error),
        }
    };
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
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
    request.extensions_mut().insert(viewer.clone());
    let mut response = next.run(request).await;
    if let Some(token) = viewer.token
        && !response.headers().contains_key(header::SET_COOKIE)
    {
        set_cookie(&mut response, SESSION, &token, accounts::LIFETIME);
    }
    response
}

pub(super) async fn headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
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
    let result =
        tokio::task::spawn_blocking(move || Accounts::load()?.sign_in(&form.email, &form.password))
            .await;
    match result {
        Ok(Ok(SignIn::SignedIn(token))) => {
            let mut response = Redirect::to("/").into_response();
            set_cookie(&mut response, SESSION, &token, accounts::LIFETIME);
            response
        }
        Ok(Ok(SignIn::Refused)) => (
            StatusCode::UNAUTHORIZED,
            axum::response::Html("Sign-in failed. <a href=\"/sign-in\">Try again</a>."),
        )
            .into_response(),
        Ok(Ok(SignIn::Slowdown)) => (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            axum::response::Html(
                "Sign-in failed. Wait one minute, then <a href=\"/sign-in\">try again</a>.",
            ),
        )
            .into_response(),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error),
    }
}

#[derive(Template)]
#[template(path = "console/account.html")]
struct AccountPage {
    email: String,
    csrf: String,
    signed_in: bool,
    theme: String,
}

pub(super) async fn account(Extension(viewer): Extension<Viewer>) -> Response {
    super::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            Ok(AccountPage {
                email: viewer.email,
                csrf: viewer.csrf,
                signed_in: viewer.token.is_some(),
                theme: UserConfig::load()?.console.theme,
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
        Accounts::load()?.sign_out(&viewer.email, Some(&token))
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
    events: Vec<accounts::Event>,
    theme: String,
}

pub(super) async fn log() -> Response {
    super::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            Ok(LogPage {
                events: Accounts::load()?.events()?,
                theme: UserConfig::load()?.console.theme,
            }
            .render()?)
        })
        .await,
    )
}

fn failed(error: impl std::fmt::Debug) -> Response {
    eprintln!("thirdshift Console: account operation failed: {error:?}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Can't read Console accounts; check the Console's stderr.",
    )
        .into_response()
}
