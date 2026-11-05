//! The JSON representation of the Console's shared read handlers.
use anyhow::Result;
use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Serialize;

pub(super) fn is_path(path: &str) -> bool {
    path == "/api/v1" || path.starts_with("/api/v1/")
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Format {
    Html,
    Json,
}

impl Format {
    pub fn render<T: Serialize>(
        self,
        value: T,
        html: impl FnOnce(T) -> Result<String>,
    ) -> Result<Response> {
        Ok(match self {
            Self::Json => axum::Json(serde_json::to_value(value)?).into_response(),
            Self::Html => Html(html(value)?).into_response(),
        })
    }
}

pub(super) fn error(status: StatusCode, message: impl ToString) -> Response {
    (
        status,
        axum::Json(serde_json::json!({"error": message.to_string()})),
    )
        .into_response()
}

/// Selecting a Security run opens details even when the selected Session has
/// an ordinary kind (for example a dispatched fix's implementation Session).
pub(super) fn security_command(config: &crate::config::UserConfig, command: &str) -> Result<bool> {
    if command.is_empty() {
        return Ok(false);
    }
    crate::logs::stop::security_details(config, command)
}

/// Extractor and routing refusals occur before a read handler can format them.
pub(super) async fn json_errors(response: Response) -> Response {
    if !(response.status().is_client_error() || response.status().is_server_error())
        || response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|value| value == "application/json")
    {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let message = match to_bytes(body, 16 * 1024).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => "Can't read this API error; check the Console's stderr.".into(),
    };
    let (json, body) = error(parts.status, message).into_parts();
    parts.headers.remove(header::CONTENT_TYPE);
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.extend(json.headers);
    Response::from_parts(parts, body)
}

pub(super) async fn read(
    format: Format,
    work: impl FnOnce() -> Result<Response> + Send + 'static,
) -> Response {
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(response)) => response,
        Ok(Err(failure)) if format == Format::Json => {
            error(StatusCode::UNPROCESSABLE_ENTITY, failure)
        }
        Err(failure) if format == Format::Json => {
            eprintln!("thirdshift Console: API read failed: {failure}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Can't read this view; check the Console's stderr.",
            )
        }
        Ok(Err(failure)) => super::page(Ok(Err(failure))),
        Err(failure) => super::page(Err(failure)),
    }
}

/// Page actions keep their redirects; API actions return the shared CLI value.
pub(super) async fn action<T: Serialize + Send + 'static>(
    format: Format,
    location: &'static str,
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Response {
    let result = tokio::task::spawn_blocking(work).await;
    match result {
        Ok(Ok(value)) if format == Format::Json => axum::Json(value).into_response(),
        Ok(Ok(_)) => Redirect::to(location).into_response(),
        Ok(Err(failure)) if format == Format::Json => {
            error(StatusCode::UNPROCESSABLE_ENTITY, failure)
        }
        Ok(Err(failure)) => (StatusCode::BAD_REQUEST, failure.to_string()).into_response(),
        Err(failure) => {
            eprintln!("thirdshift Console: action failed: {failure}");
            error(StatusCode::INTERNAL_SERVER_ERROR, "Console action failed.")
        }
    }
}
