// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Corefile validation HTTP surface.
//!
//! Exposes the [`crate::corefile`] parser over the REST management API so the
//! portal / cavectl can validate a Corefile before it is applied. This is the
//! HTTP counterpart of `cavectl dns corefile validate`.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::corefile;

/// A parsed server block summarised for the management API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CorefileBlockDto {
    /// Address labels preceding the block.
    pub keys: Vec<String>,
    /// Directive names captured inside the block (sorted).
    pub directives: Vec<String>,
}

/// Validation response payload.
#[derive(Debug, Serialize)]
pub struct CorefileValidateResponse {
    pub valid: bool,
    pub blocks: Vec<CorefileBlockDto>,
}

/// Parse `input` and summarise each server block.
///
/// Pure (no I/O) so it is unit-testable without an HTTP client; the handler is
/// a thin async wrapper.
pub fn analyze(input: &str) -> Result<Vec<CorefileBlockDto>, corefile::ParseError> {
    let blocks = corefile::parse(input)?;
    Ok(blocks
        .iter()
        .map(|b| CorefileBlockDto {
            keys: b.keys.clone(),
            directives: b.tokens.keys().cloned().collect(),
        })
        .collect())
}

/// POST /api/v1/corefile/validate — body is the raw Corefile text.
///
/// Returns 200 with the resolved blocks on success, or 400 with the parse
/// error message on failure.
pub async fn validate_corefile(body: String) -> Response {
    match analyze(&body) {
        Ok(blocks) => {
            Json(CorefileValidateResponse { valid: true, blocks }).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(crate::api::ApiError { error: e.to_string() }),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_reports_keys_and_directive_names() {
        let dtos = analyze(".:53 {\n    whoami\n    forward . 1.1.1.1\n}\n").expect("ok");
        assert_eq!(dtos.len(), 1);
        assert_eq!(dtos[0].keys, vec![".:53".to_string()]);
        // Directive names come from the parsed token map (sorted).
        assert_eq!(dtos[0].directives, vec!["forward".to_string(), "whoami".to_string()]);
    }

    #[test]
    fn analyze_propagates_parse_error() {
        // An unterminated block must surface as a parse error, not a panic.
        let err = analyze("example.com {\n    whoami\n").unwrap_err();
        assert!(err.message.contains('}') || err.message.contains("close"));
    }
}
