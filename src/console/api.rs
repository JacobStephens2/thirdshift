//! The JSON representation of the Console's shared read handlers.
use anyhow::Result;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
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
