use crate::{api::dto::Source, error::CoreError};
use serde_json::{Value, json};

pub fn validate(source: &Source) -> Result<(), CoreError> {
    if source.page == Some(0)
        || source.file_page_index.is_some_and(|index| source.page.and_then(|page| page.checked_sub(1)) != Some(index))
        || source.printed_page_label.as_ref().is_some_and(|label| label.is_empty() || label.len() > 100)
        || source.printed_page_label_missing_reason.as_ref().is_some_and(|reason| reason.len() > 200)
    {
        return Err(CoreError::Invalid("Source page locators are inconsistent or exceed their bounds".into()));
    }
    Ok(())
}

pub fn apply(citation: &mut Value, source: &Source) {
    citation["file_page_index"] = json!(source.file_page_index.or_else(|| source.page.and_then(|page| page.checked_sub(1))));
    citation["printed_page_label"] = json!(source.printed_page_label);
    citation["printed_page_label_missing_reason"] = if source.printed_page_label.is_some() { Value::Null } else {
        json!(source.printed_page_label_missing_reason.as_deref().unwrap_or("not_assessed"))
    };
    citation["page_missing_reason"] = if source.page.is_none() { json!("physical_page_not_supplied; record_level_evidence") } else { Value::Null };
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(page: Option<u32>) -> Source {
        serde_json::from_value(json!({"id":"source","title":"fixture","text":"original","url":"https://example.org/source","page":page,"sha256":"a".repeat(64),"licence":"synthetic"})).unwrap()
    }
    #[test]
    fn physical_indices_are_separate_and_printed_labels_are_never_invented() {
        let s=source(Some(1));
        let mut citation=json!({"printed_page_label":"invented","file_page_index":99});
        validate(&s).unwrap();apply(&mut citation,&s);
        assert_eq!(citation["file_page_index"],0);
        assert!(citation["printed_page_label"].is_null());
        assert_eq!(citation["printed_page_label_missing_reason"],"not_assessed");
        let mut s=source(None);apply(&mut citation,&s);
        assert!(citation["file_page_index"].is_null());
        assert!(citation["page_missing_reason"].as_str().unwrap().contains("record_level"));
        s.file_page_index=Some(0);assert!(validate(&s).is_err());
        assert!(validate(&source(Some(0))).is_err());
    }
}
