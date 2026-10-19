use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub title: String,
    pub text: String,
    pub url: String,
    pub page: Option<u32>,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_hash_scope: Option<String>,
    pub licence: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RunRequest {
    pub question: String,
    pub locale: String,
    pub model: String,
    pub sources: Vec<Source>,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub history: Vec<Value>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Event {
    pub run_id: String,
    pub sequence: u64,
    pub kind: String,
    pub agent: String,
    pub data: Value,
    pub created_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RunSnapshot {
    pub run_id: String,
    pub status: String,
    pub request: RunRequest,
    pub events: Vec<Event>,
    pub result: Option<Value>,
}
#[derive(Deserialize)]
pub struct Steering {
    pub message: String,
}
#[derive(Deserialize, Default)]
pub struct EventQuery {
    #[serde(default)]
    pub after: u64,
}
