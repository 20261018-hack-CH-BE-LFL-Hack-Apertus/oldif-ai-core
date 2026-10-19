use super::{
    diarization_config::*,
    diarization_process::{self, Scratch},
    diarization_result, diarization_wav,
};
use crate::error::CoreError;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use uuid::Uuid;
static PERMIT: Semaphore = Semaphore::const_new(1);
pub fn capability() -> Value {
    json!({"available":DiarizationConfig::from_env().is_ok(),"model":"nvidia/Nemotron-3-Diarization","runtime":"NVIDIA/NeMo-Speech.cpp","max_audio_seconds":MAX_SECONDS,"max_audio_bytes":MAX_AUDIO_BYTES,"format":"PCM16 mono16kHz WAV","local_processing_confirmation_required":true,"identity_inference":false})
}
pub async fn run(bytes: &[u8]) -> Result<Value, CoreError> {
    let seconds = diarization_wav::duration(bytes)?;
    let _permit = PERMIT.try_acquire().map_err(|_| {
        CoreError::Unavailable(
            "Diarization is busy; retry after the active request completes".into(),
        )
    })?;
    let config = DiarizationConfig::from_env()?;
    diarization_process::verify_model(&config.model).await?;
    let scratch = Scratch::create(bytes).await?;
    let output = diarization_process::execute(&config, &scratch).await?;
    let (segments, speaker_count) = diarization_result::parse(&output, seconds)?;
    Ok(
        json!({"status":"completed","execution_id":Uuid::new_v4().to_string(),"model":"nvidia/Nemotron-3-Diarization","model_revision":MODEL_REVISION,"model_sha256":MODEL_SHA,"runtime":"NVIDIA/NeMo-Speech.cpp","runtime_version":"0.2.0","license":"OpenMDW-1.1","input_sha256":format!("{:x}",Sha256::digest(bytes)),"audio_seconds":seconds,"segments":segments,"speaker_count":speaker_count,"identity_inference":false,"transcription_performed":false,"audio_retained":false}),
    )
}
