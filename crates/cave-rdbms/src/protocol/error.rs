// SPDX-License-Identifier: AGPL-3.0-or-later
//! Error response handling.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ErrorResponse {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
}

impl ErrorResponse {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            severity: "ERROR".to_string(),
            code: code.to_string(),
            message: message.to_string(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail = Some(detail.to_string());
        self
    }

    pub fn to_backend_fields(&self) -> HashMap<char, String> {
        let mut fields = HashMap::new();
        fields.insert('S', self.severity.clone());
        fields.insert('C', self.code.clone());
        fields.insert('M', self.message.clone());
        if let Some(ref d) = self.detail {
            fields.insert('D', d.clone());
        }
        fields
    }

    // PostgreSQL standard SQLSTATE codes
    pub fn syntax_error(msg: &str) -> Self {
        Self::new("42601", msg)
    }

    pub fn table_not_found(table: &str) -> Self {
        Self::new("42P01", &format!("table \"{}\" does not exist", table))
    }

    pub fn column_not_found(col: &str) -> Self {
        Self::new("42703", &format!("column \"{}\" does not exist", col))
    }

    pub fn duplicate_table(table: &str) -> Self {
        Self::new("42P07", &format!("table \"{}\" already exists", table))
    }

    pub fn unique_violation() -> Self {
        Self::new("23505", "duplicate key value violates unique constraint")
    }

    pub fn not_null_violation(col: &str) -> Self {
        Self::new("23502", &format!("null value in column \"{}\" violates not-null constraint", col))
    }

    pub fn div_by_zero() -> Self {
        Self::new("22012", "division by zero")
    }

    pub fn connection_error(msg: &str) -> Self {
        Self::new("08000", msg)
    }

    pub fn failed_transaction() -> Self {
        Self::new("25P02", "current transaction is aborted, commands ignored until end of transaction block")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_response_creation() {
        let err = ErrorResponse::new("42P01", "table not found");
        assert_eq!(err.code, "42P01");
        assert_eq!(err.message, "table not found");
    }

    #[test]
    fn test_error_response_with_detail() {
        let err = ErrorResponse::syntax_error("unexpected token")
            .with_detail("at position 10");
        assert_eq!(err.detail, Some("at position 10".to_string()));
    }

    #[test]
    fn test_error_response_to_fields() {
        let err = ErrorResponse::column_not_found("nonexistent");
        let fields = err.to_backend_fields();
        assert!(fields.contains_key(&'C'));
        assert!(fields.contains_key(&'M'));
    }
}
