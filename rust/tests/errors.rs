//! CortError envelope. Spec §1.1; B-3 locks `{ error, detail }` (no dedicated JS file).

use cort::errors::CortError;
use serde_json::json;

#[test]
fn to_json_is_error_and_detail_not_code() {
    let err = CortError::new(
        "ast_grep_version_mismatch",
        json!({ "found": "0.44.9", "expected": "0.45.2" }),
    );
    assert_eq!(
        err.to_json(),
        json!({
            "error": "ast_grep_version_mismatch",
            "detail": { "found": "0.44.9", "expected": "0.45.2" },
        })
    );
}

#[test]
fn display_matches_js_super_message() {
    let err = CortError::new("empty_query", json!({ "raw": "   " }));
    assert_eq!(err.to_string(), r#"empty_query: {"raw":"   "}"#);
}

#[test]
fn default_detail_is_json_null() {
    let err = CortError::with_code("missing_pattern");
    assert_eq!(
        err.to_json(),
        json!({ "error": "missing_pattern", "detail": null })
    );
    assert_eq!(err.to_string(), "missing_pattern: null");
}
