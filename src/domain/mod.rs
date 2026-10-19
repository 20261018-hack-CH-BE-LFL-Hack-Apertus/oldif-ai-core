pub fn valid_locale(locale: &str) -> bool {
    ["en", "es", "de", "de-CH", "fr", "it", "rm"].contains(&locale)
}
pub fn supports_tools(model: &str) -> bool {
    !model.ends_with("-thinking")
}
pub fn cited_quote_exists(source: &str, quote: &str) -> bool {
    !quote.trim().is_empty() && source.contains(quote)
}
pub fn terminal(status: &str) -> bool {
    ["needs_review", "failed", "cancelled"].contains(&status)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thinking_disables_tools() {
        assert!(!supports_tools("apertus-v1-5-70b-thinking"));
        assert!(supports_tools("apertus-v1-5-8b"));
    }
    #[test]
    fn rejects_fabricated_and_empty_quotes() {
        assert!(!cited_quote_exists("1799 survey", "1798"));
        assert!(!cited_quote_exists("text", ""));
        assert!(cited_quote_exists("1799 survey", "1799"));
    }
    #[test]
    fn supported_languages() {
        assert!(valid_locale("de-CH"));
        assert!(!valid_locale("unknown"));
    }
}
