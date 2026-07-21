//! Shared search matching used by every engine.
//!
//! A query is treated as a regular expression with **smart-case** semantics
//! (case-insensitive unless the query contains an uppercase letter). If the
//! query is not valid regex it falls back to a literal substring search, so
//! typing `(` or `[` never errors — it just searches for that text.

use regex::RegexBuilder;

/// A compiled search query.
pub struct Matcher {
    kind: Kind,
}

enum Kind {
    Regex(regex::Regex),
    /// Case-insensitive literal substring (needle pre-lowercased).
    LiteralCi(String),
    /// Case-sensitive literal substring.
    Literal(String),
}

impl Matcher {
    /// Build a matcher from a raw query string.
    pub fn new(query: &str) -> Self {
        let smart_ci = !query.chars().any(|c| c.is_uppercase());
        match RegexBuilder::new(query).case_insensitive(smart_ci).build() {
            Ok(re) => Matcher {
                kind: Kind::Regex(re),
            },
            Err(_) if smart_ci => Matcher {
                kind: Kind::LiteralCi(query.to_lowercase()),
            },
            Err(_) => Matcher {
                kind: Kind::Literal(query.to_string()),
            },
        }
    }

    /// Whether `haystack` contains a match.
    pub fn is_match(&self, haystack: &str) -> bool {
        match &self.kind {
            Kind::Regex(re) => re.is_match(haystack),
            Kind::LiteralCi(n) => haystack.to_lowercase().contains(n.as_str()),
            Kind::Literal(n) => haystack.contains(n.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_case() {
        // lowercase query -> case-insensitive
        assert!(Matcher::new("error").is_match("ERROR: boom"));
        // query with uppercase -> case-sensitive
        assert!(!Matcher::new("Error").is_match("error: boom"));
        assert!(Matcher::new("Error").is_match("Error: boom"));
    }

    #[test]
    fn regex_and_literal_fallback() {
        assert!(Matcher::new(r"\d+").is_match("abc123"));
        assert!(Matcher::new("foo|bar").is_match("a bar b"));
        // invalid regex -> literal search of the raw text
        assert!(Matcher::new("f(o").is_match("a f(o b"));
        assert!(!Matcher::new("f(o").is_match("nope"));
    }
}
