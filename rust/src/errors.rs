use serde_json::Value;
use std::fmt;

/// JS `CortError`: `super(\`${code}: ${JSON.stringify(detail)}\`)`,
/// `toJSON()` returns exactly `{ error, detail }` (key `error`, not `code`).
#[derive(Debug, Clone)]
pub struct CortError {
    pub code: String,
    pub detail: Value,
}

impl CortError {
    pub fn new(code: impl Into<String>, detail: Value) -> Self {
        Self {
            code: code.into(),
            detail,
        }
    }

    /// JS constructor default: `detail = null`.
    pub fn with_code(code: impl Into<String>) -> Self {
        Self::new(code, Value::Null)
    }

    /// Exactly `{ error: this.code, detail: this.detail }`.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "error": self.code,
            "detail": self.detail,
        })
    }
}

impl fmt::Display for CortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let detail = serde_json::to_string(&self.detail).unwrap_or_else(|_| "null".to_string());
        write!(f, "{}: {}", self.code, detail)
    }
}

impl std::error::Error for CortError {}
