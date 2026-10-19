use crate::{api::dto::Source, error::CoreError};
use serde_json::{Value, json};
pub fn definitions() -> Value {
    json!([
     {"type":"function","function":{"name":"search_sources","description":"Search only the approved source texts in this run. Returns exact passages with provenance.","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}}},
     {"type":"function","function":{"name":"read_source","description":"Read one approved archival source by its supplied ID.","parameters":{"type":"object","properties":{"source_id":{"type":"string"}},"required":["source_id"],"additionalProperties":false}}}
    ])
}
pub fn for_sources(sources: &[Source], read_only: bool) -> Value {
    let mut functions = definitions().as_array().unwrap().clone();
    functions[1]["function"]["parameters"]["properties"]["source_id"]["enum"] =
        json!(sources.iter().map(|source| &source.id).collect::<Vec<_>>());
    if read_only {
        functions.remove(0);
    }
    json!(functions)
}
pub fn execute(name: &str, args: &Value, sources: &[Source]) -> Result<Value, CoreError> {
    match name {
        "read_source" => {
            let id = args["source_id"]
                .as_str()
                .ok_or_else(|| CoreError::Invalid("Tool source_id must be a string".into()))?;
            let s = sources.iter().find(|s| s.id == id).ok_or_else(|| {
                CoreError::Invalid("Tool source is outside this run's scope".into())
            })?;
            Ok(super::passages::source_view(s))
        }
        "search_sources" => search(args, sources),
        _ => Err(CoreError::Invalid(
            "Tool denied: only search_sources and read_source are permitted".into(),
        )),
    }
}
fn search(args: &Value, sources: &[Source]) -> Result<Value, CoreError> {
    let query = args["query"]
        .as_str()
        .filter(|q| !q.is_empty() && q.len() <= 500)
        .ok_or_else(|| CoreError::Invalid("Tool query requires 1–500 bytes".into()))?;
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut ranked: Vec<_> = sources
        .iter()
        .map(|s| {
            let text = format!("{} {}", s.title, s.text).to_lowercase();
            let score = terms.iter().filter(|t| text.contains(t.as_str())).count();
            (score, s)
        })
        .filter(|(score, _)| *score > 0)
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    Ok(
        json!({"query":query,"retrieval":"lexical","matches":ranked.into_iter().take(8).map(|(score,s)|json!({"score":score,"source":s})).collect::<Vec<_>>()}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arbitrary_tools_are_denied() {
        assert!(execute("shell", &json!({"command":"echo unsafe"}), &[]).is_err());
    }
    #[test]
    fn cross_scope_source_denied() {
        assert!(execute("read_source", &json!({"source_id":"other-tenant"}), &[]).is_err());
    }
}
