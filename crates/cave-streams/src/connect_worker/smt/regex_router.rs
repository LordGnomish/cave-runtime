// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/RegexRouter.java

//! RegexRouter SMT — rewrite the destination topic name via a
//! regular-expression pattern + replacement template. Mirrors
//! upstream's `RegexRouter`: applies one substitution to the topic
//! string; falls through to the original topic if the pattern does
//! not match (upstream behaviour, validated by
//! `RegexRouterTest.testNonMatchingPattern`).

use std::collections::BTreeMap;
use std::sync::Arc;

use regex::Regex;

use crate::error::{StreamsError, StreamsResult};

use super::{RecordEnvelope, Smt, SmtRegistry};

#[derive(Debug, Clone)]
pub struct RegexRouter {
    pattern: Regex,
    replacement: String,
}

impl RegexRouter {
    pub fn from_config(cfg: &BTreeMap<String, String>) -> StreamsResult<Self> {
        let pat = cfg
            .get("regex")
            .ok_or_else(|| StreamsError::Internal("RegexRouter: 'regex' is required".into()))?;
        let repl = cfg.get("replacement").ok_or_else(|| {
            StreamsError::Internal("RegexRouter: 'replacement' is required".into())
        })?;
        let pattern = Regex::new(pat).map_err(|e| {
            StreamsError::Internal(format!("RegexRouter: invalid pattern '{pat}': {e}"))
        })?;
        Ok(Self {
            pattern,
            replacement: repl.clone(),
        })
    }

    pub fn register(reg: &SmtRegistry) {
        reg.register(
            "org.apache.kafka.connect.transforms.RegexRouter",
            Self::builder,
        );
        reg.register("RegexRouter", Self::builder);
    }

    fn builder(cfg: &BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>> {
        Ok(Arc::new(Self::from_config(cfg)?))
    }
}

impl Smt for RegexRouter {
    fn name(&self) -> &'static str {
        "org.apache.kafka.connect.transforms.RegexRouter"
    }

    fn apply(&self, mut r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
        // Upstream contract: if the regex matches, replace the topic
        // with the templated replacement; otherwise the topic is
        // returned untouched.
        if self.pattern.is_match(&r.topic) {
            let new_topic = self.pattern.replace(&r.topic, self.replacement.as_str());
            r.topic = new_topic.into_owned();
        }
        Ok(Some(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect_worker::smt::Value;

    #[test]
    fn rename_matches_full_string() {
        let mut cfg = BTreeMap::new();
        cfg.insert("regex".into(), r"^prod\.(.*)$".into());
        cfg.insert("replacement".into(), "staging.$1".into());
        let s = RegexRouter::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("prod.orders", Value::Null);
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(out.topic, "staging.orders");
    }

    #[test]
    fn no_replacement_when_no_match() {
        let mut cfg = BTreeMap::new();
        cfg.insert("regex".into(), r"^prod\.".into());
        cfg.insert("replacement".into(), "x.".into());
        let s = RegexRouter::from_config(&cfg).unwrap();
        let r = RecordEnvelope::new("dev.orders", Value::Null);
        let out = s.apply(r).unwrap().unwrap();
        assert_eq!(out.topic, "dev.orders");
    }

    #[test]
    fn bad_regex_errors_at_config() {
        let mut cfg = BTreeMap::new();
        cfg.insert("regex".into(), "[".into());
        cfg.insert("replacement".into(), "x".into());
        assert!(RegexRouter::from_config(&cfg).is_err());
    }
}
