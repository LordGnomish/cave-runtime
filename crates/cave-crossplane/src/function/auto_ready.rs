// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! function-auto-ready — marks XR Ready=True iff all composed resources
//! have a Ready=True condition.
//!
//! Upstream: function-auto-ready/fn.go

use serde_json::Value;

pub fn auto_ready_eval(composed: &[Value]) -> bool {
    !composed.is_empty()
        && composed.iter().all(|c| {
            c.get("status")
                .and_then(|s| s.get("conditions"))
                .and_then(|cs| cs.as_array())
                .map(|arr| {
                    arr.iter().any(|cd| {
                        cd.get("type").and_then(|v| v.as_str()) == Some("Ready")
                            && cd.get("status").and_then(|v| v.as_str()) == Some("True")
                    })
                })
                .unwrap_or(false)
        })
}

/// Count composed-resources reporting Ready=True.
pub fn ready_count(composed: &[Value]) -> usize {
    composed
        .iter()
        .filter(|c| {
            c.get("status")
                .and_then(|s| s.get("conditions"))
                .and_then(|cs| cs.as_array())
                .map(|arr| {
                    arr.iter().any(|cd| {
                        cd.get("type").and_then(|v| v.as_str()) == Some("Ready")
                            && cd.get("status").and_then(|v| v.as_str()) == Some("True")
                    })
                })
                .unwrap_or(false)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ready() -> Value {
        json!({"status":{"conditions":[{"type":"Ready","status":"True"}]}})
    }

    fn not_ready() -> Value {
        json!({"status":{"conditions":[{"type":"Ready","status":"False"}]}})
    }

    #[test]
    fn empty_input_not_ready() {
        assert!(!auto_ready_eval(&[]));
    }

    #[test]
    fn all_ready() {
        assert!(auto_ready_eval(&[ready(), ready()]));
    }

    #[test]
    fn one_not_ready_fails() {
        assert!(!auto_ready_eval(&[ready(), not_ready()]));
    }

    #[test]
    fn ready_count_counts() {
        assert_eq!(ready_count(&[ready(), not_ready(), ready()]), 2);
    }

    #[test]
    fn missing_status_not_ready() {
        assert!(!auto_ready_eval(&[json!({})]));
    }

    #[test]
    fn ready_count_empty_zero() {
        assert_eq!(ready_count(&[]), 0);
    }
}
