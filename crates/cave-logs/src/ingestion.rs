//! Log ingestion: structured/unstructured parsing, pipeline processing, batched writes.

use crate::models::{LogEntry, LogLevel, LogPipeline, ParseFormat};
use crate::LogsState;
use chrono::Utc;
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// ── Public API ───────────────────────────────────────────────────────────────

pub struct IngestRequest {
    pub raw: String,
    pub service: String,
    pub stream_id: Option<Uuid>,
    pub labels: HashMap<String, String>,
    pub pipeline_id: Option<Uuid>,
}

/// Ingest a single log line: parse, enrich via pipeline, store.
pub fn ingest_log(state: &Arc<LogsState>, req: IngestRequest) -> LogEntry {
    let mut entry = if let Ok(e) = parse_structured(&req.raw, &req.service, req.stream_id) {
        e
    } else {
        parse_unstructured(&req.raw, &req.service, req.stream_id)
    };

    entry.labels.extend(req.labels);

    if let Some(pipeline_id) = req.pipeline_id {
        let pipeline: Option<LogPipeline> = {
            let lock = state.pipelines.lock().unwrap();
            lock.get(&pipeline_id).cloned()
        };
        if let Some(pipeline) = pipeline {
            entry = process_pipeline(entry, &pipeline);
        }
    }

    {
        let mut entries = state.entries.lock().unwrap();
        // Global ring-buffer cap: 500k entries
        if entries.len() >= 500_000 {
            entries.pop_front();
        }
        entries.push_back(entry.clone());
    }

    entry
}

/// Ingest a batch of log lines.
pub fn ingest_batch(state: &Arc<LogsState>, reqs: Vec<IngestRequest>) -> Vec<LogEntry> {
    reqs.into_iter().map(|r| ingest_log(state, r)).collect()
}

// ── Parsers ───────────────────────────────────────────────────────────────────

/// Try to parse `raw` as a structured (JSON) log line.
pub fn parse_structured(
    raw: &str,
    service: &str,
    stream_id: Option<Uuid>,
) -> Result<LogEntry, ()> {
    let v: serde_json::Value = serde_json::from_str(raw).map_err(|_| ())?;
    let obj = v.as_object().ok_or(())?;

    let message = obj
        .get("message")
        .or_else(|| obj.get("msg"))
        .and_then(|v| v.as_str())
        .unwrap_or(raw)
        .to_string();

    let level = parse_level(
        obj.get("level")
            .or_else(|| obj.get("severity"))
            .and_then(|v| v.as_str())
            .unwrap_or("info"),
    );

    let timestamp = obj
        .get("timestamp")
        .or_else(|| obj.get("ts"))
        .or_else(|| obj.get("time"))
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let svc = obj
        .get("service")
        .and_then(|v| v.as_str())
        .unwrap_or(service)
        .to_string();

    let mut labels = HashMap::new();
    if let Some(l) = obj.get("labels").and_then(|v| v.as_object()) {
        for (k, val) in l {
            if let Some(s) = val.as_str() {
                labels.insert(k.clone(), s.to_string());
            }
        }
    }

    Ok(LogEntry {
        id: Uuid::new_v4(),
        stream_id,
        timestamp,
        level,
        message,
        service: svc,
        labels,
        fields: v,
        raw: Some(raw.to_string()),
    })
}

/// Parse an unstructured (plaintext) log line with heuristic level detection.
pub fn parse_unstructured(raw: &str, service: &str, stream_id: Option<Uuid>) -> LogEntry {
    LogEntry {
        id: Uuid::new_v4(),
        stream_id,
        timestamp: Utc::now(),
        level: detect_level_from_text(raw),
        message: raw.to_string(),
        service: service.to_string(),
        labels: extract_labels(raw),
        fields: serde_json::Value::Null,
        raw: Some(raw.to_string()),
    }
}

// ── Pipeline Processing ───────────────────────────────────────────────────────

/// Apply all pipeline steps (parse rules, label extraction, filters) to an entry.
pub fn process_pipeline(mut entry: LogEntry, pipeline: &LogPipeline) -> LogEntry {
    for rule in &pipeline.parse_rules {
        match rule.format {
            ParseFormat::Regex | ParseFormat::Grok => {
                if let Ok(re) = Regex::new(&rule.pattern) {
                    if let Some(caps) = re.captures(&entry.message) {
                        for label in &rule.labels {
                            if let Some(m) = caps.name(label) {
                                entry.labels.insert(label.clone(), m.as_str().to_string());
                            }
                        }
                    }
                }
            }
            ParseFormat::Json => {
                let msg = entry.message.clone();
                let svc = entry.service.clone();
                let sid = entry.stream_id;
                if let Ok(parsed) = parse_structured(&msg, &svc, sid) {
                    entry.labels.extend(parsed.labels);
                    if matches!(entry.fields, serde_json::Value::Null) {
                        entry.fields = parsed.fields;
                    }
                }
            }
            ParseFormat::Logfmt => {
                let extracted = extract_labels(&entry.message);
                for label in &rule.labels {
                    if let Some(v) = extracted.get(label) {
                        entry.labels.insert(label.clone(), v.clone());
                    }
                }
            }
        }
    }

    // Drop lines matching filter patterns by marking them
    for filter_pattern in &pipeline.filters {
        if let Ok(re) = Regex::new(filter_pattern) {
            if re.is_match(&entry.message) {
                entry.labels.insert("__filtered__".to_string(), "true".to_string());
            }
        }
    }

    for label in &pipeline.drop_labels {
        entry.labels.remove(label);
    }

    entry
}

/// Extract key=value label pairs from a log line (logfmt / nginx / syslog style).
pub fn extract_labels(text: &str) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    // Match word=value or word="value" or word='value'
    let Ok(re) = Regex::new(r#"(\w+)=["']?([^"'\s,\]]+)["']?"#) else {
        return labels;
    };
    for caps in re.captures_iter(text) {
        let key = caps[1].to_string();
        let val = caps[2].to_string();
        if key.len() <= 32 && val.len() <= 128 {
            labels.insert(key, val);
        }
    }
    labels
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn detect_level_from_text(text: &str) -> LogLevel {
    let lower = text.to_lowercase();
    if lower.contains("fatal") || lower.contains("panic") {
        LogLevel::Fatal
    } else if lower.contains("error") || lower.contains(" err ") || lower.contains("[err]") {
        LogLevel::Error
    } else if lower.contains("warn") {
        LogLevel::Warn
    } else if lower.contains("debug") {
        LogLevel::Debug
    } else if lower.contains("trace") {
        LogLevel::Trace
    } else {
        LogLevel::Info
    }
}

pub fn parse_level(s: &str) -> LogLevel {
    match s.to_lowercase().as_str() {
        "trace" => LogLevel::Trace,
        "debug" | "dbg" => LogLevel::Debug,
        "info" | "information" => LogLevel::Info,
        "warn" | "warning" => LogLevel::Warn,
        "error" | "err" => LogLevel::Error,
        "fatal" | "critical" | "crit" => LogLevel::Fatal,
        _ => LogLevel::Info,
    }
}
