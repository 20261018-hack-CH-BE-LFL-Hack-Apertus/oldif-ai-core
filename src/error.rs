use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("Provider returned HTTP {0}; inspect service access or quota")]
    Provider(u16),
    #[error("Provider request failed or exceeded the configured deadline")]
    Transport,
    #[error("Run not found")]
    NotFound,
    #[error("Run cancelled")]
    Cancelled,
    #[error("Run store failed")]
    Storage,
    #[error("Internal authentication required")]
    Unauthorized,
}
impl IntoResponse for CoreError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Unavailable(_) | Self::Provider(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Cancelled => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({"error":self.to_string()}))).into_response()
    }
}
