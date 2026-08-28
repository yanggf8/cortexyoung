//! Token budget. JS `src/budget.js` — `Math.ceil(String(text ?? '').length / 4)`.
//! JS `.length` is UTF-16 code units, so we count `encode_utf16()`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetResult<T> {
    pub kept: Vec<T>,
    pub truncated: bool,
}

pub fn estimate_tokens(text: &str) -> usize {
    text.encode_utf16().count().div_ceil(4)
}

pub fn apply_budget<T, R: AsRef<str>>(
    items: impl IntoIterator<Item = T>,
    budget_tokens: usize,
    render: impl Fn(&T) -> R,
) -> BudgetResult<T> {
    let mut kept = Vec::new();
    let mut used = 0usize;
    for item in items {
        let cost = estimate_tokens(render(&item).as_ref());
        // First item is always kept, even when it already exceeds the budget.
        if !kept.is_empty() && used.saturating_add(cost) > budget_tokens {
            return BudgetResult {
                kept,
                truncated: true,
            };
        }
        kept.push(item);
        used = used.saturating_add(cost);
    }
    BudgetResult {
        kept,
        truncated: false,
    }
}
