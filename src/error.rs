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
    #[error("Provider request exceeded the configured deadline")]
    Timeout,
    #[error("{0}")]
    Backpressure(String),
    #[error("{0}")]
    Policy(String),
    #[error("Run not found")]
    NotFound,
    #[error("Run cancelled")]
    Cancelled,
    #[error("Run store failed")]
    Storage,
    #[error("Internal authentication required")]
    Unauthorized,
}
impl CoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Timeout => "TIMEOUT",
            Self::Cancelled => "CANCELLED",
            Self::Backpressure(_) => "BACKPRESSURE",
            Self::Policy(_) => "POLICY_RESTRICTED",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Invalid(_) => "INVALID_REQUEST",
            Self::NotFound => "NOT_FOUND",
            Self::Provider(_) => "PROVIDER_ERROR",
            Self::Transport => "TRANSPORT_ERROR",
            Self::Storage => "STORAGE_ERROR",
            Self::Config(_) => "CONFIGURATION_ERROR",
            Self::Unavailable(_) => "UNAVAILABLE",
        }
    }
}
impl From<reqwest::Error> for CoreError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else {
            Self::Transport
        }
    }
}
impl IntoResponse for CoreError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Unavailable(_) | Self::Provider(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Cancelled => StatusCode::CONFLICT,
            Self::Timeout => StatusCode::GATEWAY_TIMEOUT,
            Self::Backpressure(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Policy(_) => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(json!({"error":self.to_string(),"code":self.code()})),
        )
            .into_response()
    }
}
