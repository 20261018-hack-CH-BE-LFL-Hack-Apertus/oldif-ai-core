use crate::error::CoreError;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
#[derive(Deserialize)]
struct NativeResult {
    segments: Vec<NativeSegment>,
}
#[derive(Deserialize)]
struct NativeSegment {
    start: f64,
    end: f64,
    speaker: u8,
}
pub fn parse(bytes: &[u8], seconds: f64) -> Result<(Vec<Value>, usize), CoreError> {
    let result: NativeResult = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if result.segments.len() > 1024 {
        return Err(invalid());
    }
    let (mut previous, mut speakers, mut segments) = (0.0, BTreeSet::new(), Vec::new());
    for s in result.segments {
        if !s.start.is_finite()
            || !s.end.is_finite()
            || s.start < previous
            || s.start < 0.0
            || s.end <= s.start
            || s.end > seconds + 0.02
            || !(1..=8).contains(&s.speaker)
        {
            return Err(invalid());
        }
        previous = s.start;
        speakers.insert(s.speaker);
        segments
            .push(json!({"start":s.start,"end":s.end,"speaker":format!("speaker_{}",s.speaker)}));
    }
    Ok((segments, speakers.len()))
}
fn invalid() -> CoreError {
    CoreError::Unavailable("Diarization output failed validation".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_anonymous_labels_and_overlap() {
        let(s,n)=parse(br#"{"segments":[{"start":0.0,"end":2.0,"speaker":1},{"start":1.0,"end":3.0,"speaker":2}]}"#,3.0).unwrap();
        assert_eq!(n, 2);
        assert_eq!(s[0]["speaker"], "speaker_1");
    }
    #[test]
    fn rejects_outside_audio_and_invalid_speaker() {
        for input in [
            br#"{"segments":[{"start":0.0,"end":9.0,"speaker":1}]}"#.as_slice(),
            br#"{"segments":[{"start":0.0,"end":1.0,"speaker":9}]}"#.as_slice(),
        ] {
            assert!(parse(input, 3.0).is_err());
        }
    }
    #[test]
    fn empty_speech_is_not_a_claim_of_identity() {
        let (s, n) = parse(br#"{"segments":[]}"#, 1.0).unwrap();
        assert!(s.is_empty());
        assert_eq!(n, 0);
    }
}
