//! Structured diagnostics shared by parse, sans-IO, IO, and adapter layers.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Severity assigned to a structured diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

impl fmt::Display for DiagnosticSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => f.write_str("info"),
            Self::Warning => f.write_str("warning"),
            Self::Error => f.write_str("error"),
        }
    }
}

/// Policy for deciding whether diagnostics should stop conversion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticPolicy {
    /// Error diagnostics stop the operation.
    #[default]
    Strict,
    /// Diagnostics are reported, but recoverable data is skipped or defaulted.
    Lenient,
}

impl DiagnosticPolicy {
    #[must_use]
    pub fn is_blocking(self, severity: DiagnosticSeverity) -> bool {
        matches!((self, severity), (Self::Strict, DiagnosticSeverity::Error))
    }
}

/// JSON-path-like location for a diagnostic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JsonPath(String);

impl JsonPath {
    #[must_use]
    pub fn root() -> Self {
        Self("$".to_owned())
    }

    #[must_use]
    pub fn child(&self, key: impl AsRef<str>) -> Self {
        let key = key.as_ref();
        let mut path = self.0.clone();
        if is_dot_safe_key(key) {
            path.push('.');
            path.push_str(key);
        } else {
            path.push_str("[\"");
            for ch in key.chars() {
                match ch {
                    '\\' => path.push_str("\\\\"),
                    '"' => path.push_str("\\\""),
                    '\n' => path.push_str("\\n"),
                    '\r' => path.push_str("\\r"),
                    '\t' => path.push_str("\\t"),
                    other => path.push(other),
                }
            }
            path.push_str("\"]");
        }
        Self(path)
    }

    #[must_use]
    pub fn index(&self, index: usize) -> Self {
        Self(format!("{}[{index}]", self.0))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for JsonPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn is_dot_safe_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// A single structured diagnostic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub path: JsonPath,
}

impl Diagnostic {
    #[must_use]
    pub fn new(
        severity: DiagnosticSeverity,
        code: impl Into<String>,
        path: JsonPath,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
            path,
        }
    }

    #[must_use]
    pub fn error(code: impl Into<String>, path: JsonPath, message: impl Into<String>) -> Self {
        Self::new(DiagnosticSeverity::Error, code, path, message)
    }

    #[must_use]
    pub fn warning(code: impl Into<String>, path: JsonPath, message: impl Into<String>) -> Self {
        Self::new(DiagnosticSeverity::Warning, code, path, message)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} at {}: {}",
            self.severity, self.code, self.path, self.message
        )
    }
}

/// Collection of diagnostics emitted during one operation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticReport {
    diagnostics: Vec<Diagnostic>,
}

impl DiagnosticReport {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub fn error(&mut self, code: impl Into<String>, path: JsonPath, message: impl Into<String>) {
        self.push(Diagnostic::error(code, path, message));
    }

    pub fn warning(&mut self, code: impl Into<String>, path: JsonPath, message: impl Into<String>) {
        self.push(Diagnostic::warning(code, path, message));
    }

    pub fn merge(&mut self, mut other: Self) {
        self.diagnostics.append(&mut other.diagnostics);
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    }

    pub fn blocking(&self, policy: DiagnosticPolicy) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(move |diagnostic| policy.is_blocking(diagnostic.severity))
    }

    #[must_use]
    pub fn is_blocked_by(&self, policy: DiagnosticPolicy) -> bool {
        self.blocking(policy).next().is_some()
    }
}

impl fmt::Display for DiagnosticReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.diagnostics.is_empty() {
            return f.write_str("no diagnostics");
        }

        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{diagnostic}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_path_uses_dot_segments_when_safe_and_escapes_other_keys() {
        let path = JsonPath::root()
            .child("extensions")
            .child("VRMC_vrm")
            .child("custom-name")
            .child("quote\"and\\slash")
            .index(3);

        assert_eq!(
            path.as_str(),
            "$.extensions.VRMC_vrm[\"custom-name\"][\"quote\\\"and\\\\slash\"][3]"
        );
    }

    #[test]
    fn strict_policy_blocks_errors_but_lenient_collects_them() {
        let mut report = DiagnosticReport::new();
        report.warning(
            "vrm.test.warning",
            JsonPath::root().child("warning"),
            "warning",
        );
        report.error("vrm.test.error", JsonPath::root().child("error"), "error");

        assert!(report.has_errors());
        assert!(report.is_blocked_by(DiagnosticPolicy::Strict));
        assert!(!report.is_blocked_by(DiagnosticPolicy::Lenient));
        assert_eq!(report.blocking(DiagnosticPolicy::Strict).count(), 1);
    }

    #[test]
    fn diagnostic_report_roundtrips_through_json() {
        let mut report = DiagnosticReport::new();
        report.error(
            "vrm.expression.invalid_shape",
            JsonPath::root().child("expressions").child("blink"),
            "invalid expression",
        );

        let json = serde_json::to_string(&report).unwrap();
        let decoded: DiagnosticReport = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, report);
    }
}
