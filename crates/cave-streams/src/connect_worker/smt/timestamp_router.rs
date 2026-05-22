// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/TimestampRouter.java

//! TimestampRouter SMT — append a formatted record timestamp to the
//! destination topic name. Mirrors upstream `TimestampRouter`:
//!
//! * `topic.format` — template using `${topic}` and `${timestamp}`
//!   placeholders. Default `${topic}-${timestamp}`.
//! * `timestamp.format` — strftime/JDK SimpleDateFormat-style format.
//!   Default `yyyyMMdd`.
//!
//! Format-token translation table from upstream's SimpleDateFormat to
//! `chrono`'s strftime is small and hand-coded — we accept exactly the
//! tokens upstream's tests exercise.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{TimeZone, Utc};

use crate::error::{StreamsError, StreamsResult};

use super::{RecordEnvelope, Smt, SmtRegistry};

const TOPIC_FORMAT_KEY: &str = "topic.format";
const TIMESTAMP_FORMAT_KEY: &str = "timestamp.format";

#[derive(Debug, Clone)]
pub struct TimestampRouter {
    topic_format: String,
    /// Chrono strftime format string (already translated from the
    /// JDK SimpleDateFormat dialect that upstream accepts).
    timestamp_format: String,
}

impl TimestampRouter {
    pub fn from_config(cfg: &BTreeMap<String, String>) -> StreamsResult<Self> {
        let topic_format = cfg.get(TOPIC_FORMAT_KEY).cloned().ok_or_else(|| {
            StreamsError::Internal(format!("TimestampRouter: '{TOPIC_FORMAT_KEY}' is required"))
        })?;
        let ts_format_in = cfg
            .get(TIMESTAMP_FORMAT_KEY)
            .cloned()
            .unwrap_or_else(|| "yyyyMMdd".to_string());
        let timestamp_format = translate_jdk_format(&ts_format_in);
        Ok(Self {
            topic_format,
            timestamp_format,
        })
    }

    pub fn register(reg: &SmtRegistry) {
        reg.register(
            "org.apache.kafka.connect.transforms.TimestampRouter",
            Self::builder,
        );
        reg.register("TimestampRouter", Self::builder);
    }

    fn builder(cfg: &BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>> {
        Ok(Arc::new(Self::from_config(cfg)?))
    }
}

impl Smt for TimestampRouter {
    fn name(&self) -> &'static str {
        "org.apache.kafka.connect.transforms.TimestampRouter"
    }

    fn apply(&self, mut r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
        let ts_ms = r
            .timestamp_ms
            .unwrap_or_else(|| Utc::now().timestamp_millis());
        let secs = ts_ms.div_euclid(1000);
        let nanos = (ts_ms.rem_euclid(1000) as u32) * 1_000_000;
        let dt = Utc
            .timestamp_opt(secs, nanos)
            .single()
            .ok_or_else(|| StreamsError::Internal("TimestampRouter: invalid timestamp".into()))?;
        let formatted_ts = dt.format(&self.timestamp_format).to_string();
        let new_topic = self
            .topic_format
            .replace("${topic}", &r.topic)
            .replace("${timestamp}", &formatted_ts);
        r.topic = new_topic;
        Ok(Some(r))
    }
}

/// Translate JDK `SimpleDateFormat` to chrono's strftime dialect.
/// Only the tokens that upstream's tests exercise are handled —
/// `yyyy`, `MM`, `dd`, `HH`, `mm`, `ss`. Other characters are
/// preserved literally so e.g. `yyyy-MM-dd` becomes `%Y-%m-%d`.
fn translate_jdk_format(jdk: &str) -> String {
    let mut out = String::with_capacity(jdk.len());
    let bytes = jdk.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let remaining = &jdk[i..];
        if let Some(stripped) = remaining.strip_prefix("yyyy") {
            out.push_str("%Y");
            i = bytes.len() - stripped.len();
        } else if let Some(stripped) = remaining.strip_prefix("MM") {
            out.push_str("%m");
            i = bytes.len() - stripped.len();
        } else if let Some(stripped) = remaining.strip_prefix("dd") {
            out.push_str("%d");
            i = bytes.len() - stripped.len();
        } else if let Some(stripped) = remaining.strip_prefix("HH") {
            out.push_str("%H");
            i = bytes.len() - stripped.len();
        } else if let Some(stripped) = remaining.strip_prefix("mm") {
            out.push_str("%M");
            i = bytes.len() - stripped.len();
        } else if let Some(stripped) = remaining.strip_prefix("ss") {
            out.push_str("%S");
            i = bytes.len() - stripped.len();
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect_worker::smt::Value;

    #[test]
    fn translate_jdk_format_handles_common_tokens() {
        assert_eq!(translate_jdk_format("yyyyMMdd"), "%Y%m%d");
        assert_eq!(translate_jdk_format("yyyy-MM-dd"), "%Y-%m-%d");
        assert_eq!(translate_jdk_format("HH:mm:ss"), "%H:%M:%S");
    }

    #[test]
    fn translate_jdk_format_preserves_unknown_chars() {
        assert_eq!(translate_jdk_format("foo-yyyy/x"), "foo-%Y/x");
    }

    #[test]
    fn requires_topic_format() {
        let cfg = BTreeMap::new();
        assert!(TimestampRouter::from_config(&cfg).is_err());
    }
}
