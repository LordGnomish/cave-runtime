// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Corefile validation HTTP surface.
//!
//! Exposes the [`crate::corefile`] parser over the REST management API so the
//! portal / cavectl can validate a Corefile before it is applied. This is the
//! HTTP counterpart of `cavectl dns corefile validate`.

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
