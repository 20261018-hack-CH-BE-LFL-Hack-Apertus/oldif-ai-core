use crate::{error::CoreError, infrastructure::diarization};
use axum::{Json, body::Bytes, http::HeaderMap};
use serde_json::Value;
pub async fn capability() -> Json<Value> {
    Json(diarization::capability())
}
pub async fn process(headers: HeaderMap, bytes: Bytes) -> Result<Json<Value>, CoreError> {
    let consent = headers
        .get("x-local-processing-confirmed")
        .and_then(|v| v.to_str().ok())
        == Some("true");
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::trim);
    if !consent {
        return Err(CoreError::Invalid(
            "Confirm permission for local audio processing before upload".into(),
        ));
    }
    if content_type != Some("audio/wav") {
        return Err(CoreError::Invalid("Content-Type must be audio/wav".into()));
    }
    diarization::run(&bytes).await.map(Json)
}
