//! Small, stable diagnostics objects. This module deliberately accepts JSON
//! values so the companion can redact an evolving set of subsystem fields
//! without copying secrets into a second typed model.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub const DIAGNOSTICS_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSnapshot {
    pub protocol_version: u32,
    pub uptime_ms: u128,
    pub receivers: usize,
    pub sessions: usize,
    pub warnings: Vec<String>,
    pub last_error: Option<String>,
}

impl DiagnosticSnapshot {
    pub fn new(uptime_ms: u128, receivers: usize, sessions: usize) -> Self {
        Self {
            protocol_version: DIAGNOSTICS_VERSION,
            uptime_ms,
            receivers,
            sessions,
            warnings: Vec::new(),
            last_error: None,
        }
    }

    pub fn warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(truncate(warning.into(), 256));
        self
    }
}

#[derive(Debug, Clone)]
pub struct Redactor {
    secret_keys: BTreeSet<String>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new([
            "authorization",
            "cookie",
            "password",
            "secret",
            "token",
            "accessToken",
            "refreshToken",
            "privateKey",
        ])
    }
}

impl Redactor {
    pub fn new<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            secret_keys: keys
                .into_iter()
                .map(|key| key.into().to_ascii_lowercase())
                .collect(),
        }
    }

    pub fn redact(&self, value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut redacted = Map::new();
                for (key, value) in object {
                    if self.secret_keys.contains(&key.to_ascii_lowercase())
                        || key.to_ascii_lowercase().contains("token")
                        || key.to_ascii_lowercase().contains("password")
                    {
                        redacted.insert(key.clone(), Value::String("[redacted]".into()));
                    } else {
                        redacted.insert(key.clone(), self.redact(value));
                    }
                }
                Value::Object(redacted)
            }
            Value::Array(values) => {
                Value::Array(values.iter().map(|value| self.redact(value)).collect())
            }
            _ => value.clone(),
        }
    }
}

pub fn truncate(value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    if max_bytes <= 3 {
        return ".".repeat(max_bytes);
    }
    let mut end = max_bytes.saturating_sub(3);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_nested_credentials_and_preserves_shape() {
        let redacted = Redactor::default().redact(&json!({
            "name": "receiver",
            "auth": {"accessToken": "secret", "label": "safe"},
            "items": [{"cookie": "secret"}]
        }));
        assert_eq!(
            redacted,
            json!({
                "name": "receiver",
                "auth": {"accessToken": "[redacted]", "label": "safe"},
                "items": [{"cookie": "[redacted]"}]
            })
        );
    }

    #[test]
    fn warnings_are_bounded() {
        let snapshot = DiagnosticSnapshot::new(0, 0, 0).warning("x".repeat(300));
        assert!(snapshot.warnings[0].len() <= 256);
    }

    #[test]
    fn truncation_respects_every_small_byte_limit() {
        for max_bytes in 0..=6 {
            assert!(truncate("éééé".into(), max_bytes).len() <= max_bytes);
        }
    }
}
