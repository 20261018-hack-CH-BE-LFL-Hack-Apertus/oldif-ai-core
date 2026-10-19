use super::diarization_config::{
    DiarizationConfig, MAX_OUTPUT_BYTES, MODEL_BYTES, MODEL_SHA, TIMEOUT_SECONDS,
};
use crate::error::CoreError;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    fs,
    io::AsyncReadExt,
    process::{Child, Command},
};
use uuid::Uuid;
pub struct Scratch {
    root: PathBuf,
    pub input: PathBuf,
}
impl Scratch {
    pub async fn create(bytes: &[u8]) -> Result<Self, CoreError> {
        let root = std::env::temp_dir().join(format!("oldif-diarization-{}", Uuid::new_v4()));
        fs::create_dir(&root)
            .await
            .map_err(|_| CoreError::Storage)?;
        let scratch = Self {
            input: root.join("input.wav"),
            root,
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&scratch.root, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(|_| CoreError::Storage)?;
        }
        fs::write(&scratch.input, bytes)
            .await
            .map_err(|_| CoreError::Storage)?;
        Ok(scratch)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
pub async fn verify_model(path: &Path) -> Result<(), CoreError> {
    let mut file = fs::File::open(path).await.map_err(|_| unavailable())?;
    if file.metadata().await.map_err(|_| unavailable())?.len() != MODEL_BYTES {
        return Err(unavailable());
    }
    let (mut digest, mut buffer) = (Sha256::new(), [0u8; 65_536]);
    loop {
        let n = file.read(&mut buffer).await.map_err(|_| unavailable())?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    if format!("{:x}", digest.finalize()) != MODEL_SHA {
        return Err(unavailable());
    }
    Ok(())
}
fn command(config: &DiarizationConfig, scratch: &Scratch) -> Command {
    let mut command = Command::new(&config.binary);
    command
        .arg("diarize")
        .arg(&scratch.input)
        .arg("--model")
        .arg(&config.model)
        .args(["--device", "cpu", "--format", "json"])
        .env_clear()
        .current_dir(&scratch.root)
        .env("OMP_NUM_THREADS", "2")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}
async fn collect(child: &mut Child) -> Result<Vec<u8>, CoreError> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(unavailable)?
        .take(MAX_OUTPUT_BYTES + 1);
    let mut output = Vec::new();
    stdout
        .read_to_end(&mut output)
        .await
        .map_err(|_| unavailable())?;
    if output.len() as u64 > MAX_OUTPUT_BYTES {
        let _ = child.kill().await;
        return Err(unavailable());
    }
    let status = child.wait().await.map_err(|_| unavailable())?;
    if !status.success() {
        return Err(unavailable());
    }
    Ok(output)
}
pub async fn execute(config: &DiarizationConfig, scratch: &Scratch) -> Result<Vec<u8>, CoreError> {
    let mut child = command(config, scratch)
        .spawn()
        .map_err(|_| unavailable())?;
    tokio::time::timeout(Duration::from_secs(TIMEOUT_SECONDS), collect(&mut child))
        .await
        .map_err(|_| CoreError::Unavailable("Diarization exceeded its 90 second deadline".into()))?
}
fn unavailable() -> CoreError {
    CoreError::Unavailable("Pinned local diarization runtime or model is unavailable".into())
}
