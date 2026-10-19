use crate::error::CoreError;
use std::{env, path::PathBuf};

#[derive(Clone)]
pub struct Config {
    pub bind: String,
    pub grpc_bind: String,
    pub internal_token: String,
    pub covenant_token: Option<String>,
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
    pub execution_profile: String,
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
        let covenant_token = covenant_token(optional("CORE_COVENANT_TOKEN"), &token)?;
        let execution_profile = optional("AI_EXECUTION_PROFILE").unwrap_or("research".into());
        if !["research", "submission"].contains(&execution_profile.as_str()) {
            return Err(CoreError::Config(
                "AI_EXECUTION_PROFILE must be research or submission".into(),
            ));
        }
        let default_model = required("AI_DEFAULT_MODEL")?;
        if ![
            "apertus-v1-5-8b",
            "apertus-v1-5-8b-thinking",
            "apertus-v1-5-70b",
            "apertus-v1-5-70b-thinking",
        ]
        .contains(&default_model.as_str())
        {
            return Err(CoreError::Config(
                "The default model must be Apertus 1.5".into(),
            ));
        }
        Ok(Self {
            bind: required("AI_HTTP_BIND")?,
            grpc_bind: required("AI_GRPC_BIND")?,
            internal_token: token,
            covenant_token,
            cscs_base: required("CSCS_BASE_URL")?,
            cscs_key: optional("CSCS_API_KEY"),
            openai_base: required("OPENAI_BASE_URL")?,
            openai_key: optional("OPENAI_API_KEY"),
            default_model,
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
            execution_profile,
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

fn covenant_token(value: Option<String>, backend: &str) -> Result<Option<String>, CoreError> {
    if value
        .as_ref()
        .is_some_and(|token| token.len() < 32 || token == backend)
    {
        return Err(CoreError::Config("CORE_COVENANT_TOKEN must contain at least 32 characters and differ from INTERNAL_SERVICE_TOKEN".into()));
    }
    Ok(value)
}

#[cfg(test)]
mod scoped_token_tests {
    use super::covenant_token;
    #[test]
    fn covenant_token_is_distinct_and_optional_without_shared_fallback() {
        let backend = "b".repeat(32);
        assert!(covenant_token(None, &backend).unwrap().is_none());
        assert!(covenant_token(Some(backend.clone()), &backend).is_err());
        assert!(covenant_token(Some("short".into()), &backend).is_err());
        assert_eq!(
            covenant_token(Some("c".repeat(32)), &backend).unwrap(),
            Some("c".repeat(32))
        );
    }
}
