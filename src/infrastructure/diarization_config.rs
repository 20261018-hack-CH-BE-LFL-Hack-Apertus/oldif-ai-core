use crate::error::CoreError;
use std::{env, path::PathBuf};
pub const MAX_AUDIO_BYTES: usize = 2_900_000;
pub const MAX_SECONDS: f64 = 90.0;
pub const MODEL_BYTES: u64 = 107_012_128;
pub const MODEL_SHA: &str = "08456d9e22cd9a323c0364d98375f3746d6e68507ebb705cd46438c534c7a3a1";
pub const MODEL_REVISION: &str = "f667ed73aee57d40cc39428eb768b4fd87a0a29e";
pub const MAX_OUTPUT_BYTES: u64 = 65_536;
pub const TIMEOUT_SECONDS: u64 = 90;
pub struct DiarizationConfig {
    pub binary: PathBuf,
    pub model: PathBuf,
}
impl DiarizationConfig {
    pub fn from_env() -> Result<Self, CoreError> {
        Ok(Self {
            binary: path("AI_DIARIZATION_BIN")?,
            model: path("AI_DIARIZATION_MODEL")?,
        })
    }
}
fn path(name: &str) -> Result<PathBuf, CoreError> {
    let value = env::var(name)
        .map_err(|_| CoreError::Unavailable("Local diarization is not configured".into()))?;
    let path = PathBuf::from(value);
    if !path.is_absolute() || !path.is_file() {
        return Err(CoreError::Unavailable(
            "Local diarization artifact is unavailable".into(),
        ));
    }
    Ok(path)
}
