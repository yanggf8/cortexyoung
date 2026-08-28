//! B-6..B-9 — budget.test.js

use cort::budget::{apply_budget, estimate_tokens};

/// B-6
#[test]
fn token_estimate_is_four_characters_per_token_rounded_up() {
    assert_eq!(estimate_tokens(""), 0);
    assert_eq!(estimate_tokens("abcd"), 1);
    assert_eq!(estimate_tokens("abcde"), 2);
}

/// B-7
#[test]
fn apply_budget_keeps_items_while_the_cumulative_rendered_size_fits() {
    let items = [1, 2, 3];
    let render = |_: &i32| "x".repeat(40); // 10 tokens each
    let r = apply_budget(items, 25, render);
    assert_eq!(r.kept.len(), 2);
    assert!(r.truncated);
}

/// B-8
#[test]
fn apply_budget_reports_no_truncation_when_everything_fits() {
    let r = apply_budget([1], 1000, |_| "short");
    assert_eq!(r.kept.len(), 1);
    assert!(!r.truncated);
}

/// B-9
#[test]
fn apply_budget_always_keeps_at_least_one_item_so_the_answer_is_never_empty() {
    let r = apply_budget([1, 2], 1, |_| "x".repeat(400));
    assert_eq!(r.kept.len(), 1);
    assert!(r.truncated);
}
