//! One error type for all HTTP handlers.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use domain::api::{ApiError, ApiErrorBody};

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    /// Machine-readable code; the web app translates it (`error.<code>`).
    pub code: &'static str,
    pub message: String,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        AppError { status, code, message: message.into() }
    }

    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::bad_request("invalid_input", message)
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "not found")
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "sign-in required")
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    pub fn rate_limited() -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", "too many requests")
    }

    /// Log the real cause, tell the client nothing about our internals.
    pub fn internal(cause: impl std::fmt::Display) -> Self {
        tracing::error!(error = %cause, "internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "generic", "internal error")
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = ApiError { error: ApiErrorBody { code: self.code.to_string(), message: self.message } };
        (self.status, Json(body)).into_response()
    }
}

// These `From` impls let handlers use `?` on database, storage and render
// errors; all of them become an opaque 500.
impl From<db::Error> for AppError {
    fn from(e: db::Error) -> Self {
        AppError::internal(e)
    }
}

impl From<storage::StorageError> for AppError {
    fn from(e: storage::StorageError) -> Self {
        AppError::internal(e)
    }
}

impl From<renderer::RenderError> for AppError {
    fn from(e: renderer::RenderError) -> Self {
        AppError::internal(e)
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(e: tokio::task::JoinError) -> Self {
        AppError::internal(e)
    }
}
