use super::diarization_config::{MAX_AUDIO_BYTES, MAX_SECONDS};
use crate::error::CoreError;
fn invalid() -> CoreError {
    CoreError::Invalid("Expected a complete PCM16 mono 16 kHz WAV of at most 90 seconds".into())
}
fn u32le(bytes: &[u8]) -> usize {
    u32::from_le_bytes(bytes.try_into().unwrap_or([0; 4])) as usize
}
fn validate_header(bytes: &[u8]) -> Result<(), CoreError> {
    if bytes.len() < 44
        || bytes.len() > MAX_AUDIO_BYTES
        || &bytes[..4] != b"RIFF"
        || &bytes[8..12] != b"WAVE"
        || u32le(&bytes[4..8]) + 8 != bytes.len()
    {
        return Err(invalid());
    }
    Ok(())
}
fn inspect_chunk(
    kind: &[u8],
    payload: &[u8],
    format: &mut bool,
    data: &mut Option<usize>,
) -> Result<(), CoreError> {
    match kind {
        b"fmt " => {
            if *format || !valid_format(payload) {
                return Err(invalid());
            }
            *format = true;
        }
        b"data" => {
            if data.is_some() || payload.is_empty() || payload.len() % 2 != 0 {
                return Err(invalid());
            }
            *data = Some(payload.len());
        }
        _ => {}
    }
    Ok(())
}
pub fn duration(bytes: &[u8]) -> Result<f64, CoreError> {
    validate_header(bytes)?;
    let (mut offset, mut format, mut data) = (12usize, false, None);
    while offset + 8 <= bytes.len() {
        let length = u32le(&bytes[offset + 4..offset + 8]);
        let start = offset + 8;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(invalid)?;
        inspect_chunk(
            &bytes[offset..offset + 4],
            &bytes[start..end],
            &mut format,
            &mut data,
        )?;
        offset = end.checked_add(length % 2).ok_or_else(invalid)?;
    }
    let seconds = data.ok_or_else(invalid)? as f64 / 32_000.0;
    if !format || offset != bytes.len() || seconds > MAX_SECONDS {
        return Err(invalid());
    }
    Ok(seconds)
}
fn valid_format(b: &[u8]) -> bool {
    b.len() >= 16
        && b[0..4] == [1, 0, 1, 0]
        && u32le(&b[4..8]) == 16_000
        && u32le(&b[8..12]) == 32_000
        && b[12..16] == [2, 0, 16, 0]
}
#[cfg(test)]
mod tests {
    use super::*;
    fn wav(seconds: u32) -> Vec<u8> {
        let length = seconds * 32_000;
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((length + 36).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend([1, 0, 1, 0]);
        b.extend(16_000u32.to_le_bytes());
        b.extend(32_000u32.to_le_bytes());
        b.extend([2, 0, 16, 0]);
        b.extend(b"data");
        b.extend(length.to_le_bytes());
        b.resize(length as usize + 44, 0);
        b
    }
    #[test]
    fn accepts_bounded_pcm_and_rejects_truncation() {
        let mut b = wav(1);
        assert_eq!(duration(&b).unwrap(), 1.0);
        b.pop();
        assert!(duration(&b).is_err());
    }
    #[test]
    fn rejects_stereo_compressed_and_overlong_audio() {
        let mut b = wav(1);
        b[22] = 2;
        assert!(duration(&b).is_err());
        b[22] = 1;
        b[20] = 3;
        assert!(duration(&b).is_err());
        assert!(duration(&wav(91)).is_err());
    }
    #[test]
    fn rejects_claimed_size_and_empty_audio() {
        let mut b = wav(1);
        b[4] = 0;
        assert!(duration(&b).is_err());
        assert!(duration(&wav(0)).is_err());
    }
}
