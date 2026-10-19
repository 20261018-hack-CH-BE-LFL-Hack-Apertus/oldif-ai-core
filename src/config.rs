use crate::error::CoreError;
use std::{env, path::PathBuf};

#[derive(Clone)]
pub struct Config {
    pub bind: String,
    pub grpc_bind: String,
    pub internal_token: String,
    pub cscs_base: String,
    pub cscs_key: Option<String>,
    pub openai_base: String,
    pub openai_key: Option<String>,
    pub default_model: String,
    pub reference_model: String,
    pub realtime_model: String,
    pub transcription_model: String,
    pub voice: String,
    pub run_dir: PathBuf,
    pub timeout_secs: u64,
    pub output_tokens: u32,
    pub max_sources: usize,
    pub max_input_bytes: usize,
    pub max_tool_calls: usize,
    pub max_concurrency: usize,
    pub max_runs: usize,
    pub voice_enabled: bool,
}
impl Config {
    pub fn from_env() -> Result<Self, CoreError> {
        let _ = dotenvy::from_filename(".env");
        let token = required("INTERNAL_SERVICE_TOKEN")?;
        if token.len() < 32 {
            return Err(CoreError::Config(
                "INTERNAL_SERVICE_TOKEN must be at least 32 characters".into(),
            ));
        }
        Ok(Self {
            bind: required("AI_HTTP_BIND")?,
            grpc_bind: required("AI_GRPC_BIND")?,
            internal_token: token,
            cscs_base: required("CSCS_BASE_URL")?,
            cscs_key: optional("CSCS_API_KEY"),
            openai_base: required("OPENAI_BASE_URL")?,
            openai_key: optional("OPENAI_API_KEY"),
            default_model: required("AI_DEFAULT_MODEL")?,
            reference_model: required("AI_REFERENCE_MODEL")?,
            realtime_model: required("AI_REALTIME_MODEL")?,
            transcription_model: required("AI_TRANSCRIPTION_MODEL")?,
            voice: required("AI_VOICE")?,
            run_dir: PathBuf::from(required("AI_RUN_DIR")?),
            timeout_secs: number("AI_REQUEST_TIMEOUT_SECONDS", 120)?,
            output_tokens: number("AI_MAX_OUTPUT_TOKENS", 2048)?,
            max_sources: number("AI_MAX_SOURCES", 30)?,
            max_input_bytes: number("AI_MAX_INPUT_BYTES", 120000)?,
            max_tool_calls: number("AI_MAX_TOOL_CALLS", 4)?,
            max_concurrency: number("AI_MAX_CONCURRENCY", 2)?,
            max_runs: number("AI_MAX_RUNS", 1000)?,
            voice_enabled: env::var("AI_VOICE_ENABLED").as_deref() == Ok("true"),
        })
    }
}
fn required(name: &str) -> Result<String, CoreError> {
    optional(name)
        .ok_or_else(|| CoreError::Config(format!("Set {name} in the service environment")))
}
fn optional(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .filter(|s| !s.trim().is_empty() && !s.starts_with("replace-") && s != "changeme")
}
fn number<T: std::str::FromStr>(name: &str, default: T) -> Result<T, CoreError> {
    match env::var(name) {
        Ok(s) => s
            .parse()
            .map_err(|_| CoreError::Config(format!("Invalid {name}"))),
        Err(_) => Ok(default),
    }
}
