use cort::memory_search::{query_terms, sql_exempt};

#[test]
fn exempt_terms() {
    // Cased non-ASCII letters, and both ends of the combining-mark range.
    for t in [
        "café",
        "ｄｅｐｌｏｙ",
        "i\u{307}stanbul",
        "Ωmega",
        "e\u{300}",
        "a\u{36f}",
    ] {
        assert!(sql_exempt(t), "{t}");
    }
    // CJK, ASCII with SQL-looking characters, uncased non-ASCII, and just outside the range.
    for t in ["部署", "deploy-v2", "50%_done", "ª", "١٢", "a\u{2ff}"] {
        assert!(!sql_exempt(t), "{t}");
    }
}

#[test]
fn query_terms_lowercases_and_flags_exempt() {
    let q = query_terms("The Deploy 部署 deploy");
    assert_eq!(q.terms, vec!["the", "deploy", "部署", "deploy"]);
    assert_eq!(q.partial, vec!["deploy", "部署"]);
    assert!(!q.exempt);
    let q = query_terms("Café x");
    assert_eq!(q.terms, vec!["café", "x"]);
    assert!(q.exempt);
}
