//! Term analysis for recall's shared-database search: which terms SQL may filter on, and which it
//! must leave to the client's Unicode-aware match.

use crate::memory::partial_terms;

/// True when SQLite's ASCII-only `lower()` cannot be trusted to agree with Rust's Unicode
/// `to_lowercase()` for this term: it has a cased non-ASCII letter, or a combining mark
/// (U+0300..=U+036F). Such a term contributes no SQL condition.
pub fn sql_exempt(term: &str) -> bool {
    term.chars().any(|c| {
        ('\u{300}'..='\u{36f}').contains(&c)
            || (!c.is_ascii()
                && c.to_uppercase().collect::<String>() != c.to_lowercase().collect::<String>())
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryTerms {
    /// Whitespace-split, Unicode-lowercased terms in query order.
    pub terms: Vec<String>,
    /// The partial-match term list (`memory::partial_terms`).
    pub partial: Vec<String>,
    /// Any term in `terms` is `sql_exempt`.
    pub exempt: bool,
}

pub fn query_terms(query: &str) -> QueryTerms {
    let terms: Vec<String> = query.split_whitespace().map(|s| s.to_lowercase()).collect();
    let partial = partial_terms(&terms);
    let exempt = terms.iter().any(|t| sql_exempt(t));
    QueryTerms {
        terms,
        partial,
        exempt,
    }
}
