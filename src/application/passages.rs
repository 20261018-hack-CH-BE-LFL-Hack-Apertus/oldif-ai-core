use crate::{api::dto::Source, error::CoreError};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Clone, Serialize)]
pub struct Passage {
    pub passage_id: String,
    pub char_start: usize,
    pub char_end: usize,
    pub text: String,
}

/// Stable clauses with offsets in Unicode scalar values, not UTF-8 bytes.
/// After 255 clauses the final passage preserves the entire remaining tail.
pub fn collect(text: &str) -> Vec<Passage> {
    let mut passages = vec![];
    let (mut byte_start, mut char_start) = (0, 0);
    for (char_index, (byte_index, c)) in text.char_indices().enumerate() {
        if ",.;:!?\n".contains(c) {
            append(&mut passages, &text[byte_start..byte_index], char_start);
            byte_start = byte_index + c.len_utf8();
            char_start = char_index + 1;
            if passages.len() == 255 {
                break;
            }
        }
    }
    append(&mut passages, &text[byte_start..], char_start);
    passages
}
fn append(passages: &mut Vec<Passage>, raw: &str, char_start: usize) {
    let text = raw.trim();
    if text.is_empty() {
        return;
    }
    let start = char_start + raw.chars().take_while(|c| c.is_whitespace()).count();
    passages.push(Passage {
        passage_id: format!("p{:03}", passages.len() + 1),
        char_start: start,
        char_end: start + text.chars().count(),
        text: text.into(),
    });
}
pub fn source_view(source: &Source) -> Value {
    let mut view = json!(source);
    // The original request retains the complete text and record hash unchanged.
    view.as_object_mut().unwrap().remove("text");
    view["passages"] = json!(collect(&source.text));
    view["text_sha256"] = json!(super::validation::source_text_hash(source));
    view["offset_unit"] = json!("unicode_scalar_value");
    view["text_representation"] = json!("exact_passages");
    view
}
pub fn resolve(source: &Source, citation: &Value) -> Result<Option<Passage>, CoreError> {
    let Some(value) = citation.get("passage_id") else {
        return Ok(None);
    };
    let passage = value
        .as_str()
        .and_then(|id| {
            collect(&source.text)
                .into_iter()
                .find(|p| p.passage_id == id)
        })
        .ok_or_else(|| {
            CoreError::Invalid("Citation passage_id is absent from its source".into())
        })?;
    if citation
        .get("quote")
        .is_some_and(|q| q.as_str() != Some(&passage.text))
    {
        return Err(CoreError::Invalid(
            "Citation quote conflicts with its exact source passage".into(),
        ));
    }
    Ok(Some(passage))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offsets_recover_exact_unicode_text_and_preserve_spelling() {
        let text = "  Äpfel, läsen und rechnen. école!";
        for passage in collect(text) {
            let recovered: String = text
                .chars()
                .skip(passage.char_start)
                .take(passage.char_end - passage.char_start)
                .collect();
            assert_eq!(passage.text, recovered);
        }
        assert_eq!(collect(text)[1].text, "läsen und rechnen");
    }
    #[test]
    fn bounded_passage_count_preserves_unsplit_tail() {
        let text = "part,".repeat(1000);
        let passages = collect(&text);
        assert_eq!(passages.len(), 256);
        let last = passages.last().unwrap();
        assert_eq!(last.char_end, text.chars().count());
        assert!(last.text.ends_with("part,"));
    }
    #[test]
    fn alpthal_passage_resolves_without_model_copying() {
        let source: Source =
            serde_json::from_str(include_str!("../../evals/fixtures/stapfer-2027.json")).unwrap();
        let passage = resolve(&source, &json!({"passage_id":"p048"}))
            .unwrap()
            .unwrap();
        assert_eq!(
            passage.text,
            "diese sollen im schreiben läsen und rechnen unterrichtet werden"
        );
        let request = serde_json::from_value(json!({"question":"Subjects?","locale":"en","model":"apertus-v1-5-8b","sources":[source],"session_id":"fixture","user_id":"fixture"})).unwrap();
        let result = super::super::validation::ground(json!({"answer":"Intended writing, reading and arithmetic","citations":[{"source_id":"stapfer-2027","passage_id":"p048","char_start":0,"char_end":1}],"abstained":false}), &request).unwrap();
        assert_eq!(result["citations"][0]["quote"], passage.text);
        assert_eq!(result["citations"][0]["sha256"], request.sources[0].sha256);
        assert_eq!(result["citations"][0]["char_start"], 2410);
        assert_eq!(result["citations"][0]["char_end"], 2473);
    }
    #[test]
    fn unknown_and_conflicting_citations_are_rejected() {
        let source: Source =
            serde_json::from_str(include_str!("../../evals/fixtures/stapfer-2027.json")).unwrap();
        assert!(resolve(&source, &json!({"passage_id":"p999"})).is_err());
        assert!(resolve(&source, &json!({"passage_id":null})).is_err());
        assert!(resolve(&source, &json!({"passage_id":"p048","quote":"lesen"})).is_err());
        assert!(
            resolve(&source, &json!({"quote":"läsen"}))
                .unwrap()
                .is_none()
        );
    }
}
