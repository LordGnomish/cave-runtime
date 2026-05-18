// SPDX-License-Identifier: AGPL-3.0-or-later
//! Syslog receiver — RFC 5424 and RFC 3164 (BSD syslog) parsers.
//!
//! Both formats are accepted on the same UDP/TCP listener. The parser
//! auto-detects the format based on the leading `<PRI>` and version field.

use std::collections::HashMap;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};

use crate::models::{Labels, LogEntry, TimestampNs};

/// A parsed syslog message.
#[derive(Debug)]
pub struct SyslogMessage {
    pub facility: u8,
    pub severity: u8,
    pub timestamp: TimestampNs,
    pub hostname: Option<String>,
    pub app_name: Option<String>,
    pub proc_id: Option<String>,
    pub msg_id: Option<String>,
    pub structured_data: HashMap<String, HashMap<String, String>>,
    pub message: String,
}

impl SyslogMessage {
    /// Convert to a `LogEntry` + `Labels` pair for storage.
    pub fn to_entry_and_labels(&self) -> (LogEntry, Labels) {
        let mut label_map: HashMap<String, String> = HashMap::new();

        if let Some(h) = &self.hostname {
            if h != "-" { label_map.insert("hostname".into(), h.clone()); }
        }
        if let Some(a) = &self.app_name {
            if a != "-" { label_map.insert("app".into(), a.clone()); }
        }
        if let Some(p) = &self.proc_id {
            if p != "-" { label_map.insert("proc_id".into(), p.clone()); }
        }
        label_map.insert("facility".into(), self.facility.to_string());
        label_map.insert("severity".into(), severity_name(self.severity).to_owned());

        let mut meta: HashMap<String, String> = HashMap::new();
        if let Some(mid) = &self.msg_id {
            if mid != "-" { meta.insert("msg_id".into(), mid.clone()); }
        }
        for (sd_id, params) in &self.structured_data {
            for (k, v) in params {
                meta.insert(format!("{}.{}", sd_id, k), v.clone());
            }
        }

        let entry = LogEntry { ts: self.timestamp, line: self.message.clone(), metadata: meta };
        (entry, Labels::new(label_map))
    }
}

fn severity_name(sev: u8) -> &'static str {
    match sev {
        0 => "emergency",
        1 => "alert",
        2 => "critical",
        3 => "error",
        4 => "warning",
        5 => "notice",
        6 => "informational",
        7 => "debug",
        _ => "unknown",
    }
}

// ── RFC 5424 ─────────────────────────────────────────────────────────────────
//
// <PRI>VERSION TIMESTAMP HOSTNAME APP-NAME PROCID MSGID STRUCTURED-DATA MSG

pub fn parse_rfc5424(line: &str) -> Option<SyslogMessage> {
    let line = line.trim();
    let (pri, rest) = parse_priority(line)?;
    let facility = (pri >> 3) as u8;
    let severity = (pri & 0x07) as u8;

    // Version field (must be "1")
    let (ver, rest) = next_field(rest)?;
    if ver != "1" { return None; }

    let (ts_str, rest) = next_field(rest)?;
    let timestamp = parse_5424_timestamp(ts_str).unwrap_or_else(|| Utc::now().timestamp_nanos_opt().unwrap_or(0));

    let (hostname, rest) = next_field(rest)?;
    let (app_name, rest) = next_field(rest)?;
    let (proc_id, rest) = next_field(rest)?;
    let (msg_id, rest) = next_field(rest)?;

    let (structured_data, rest) = parse_structured_data(rest)?;

    // BOM is optional per RFC.
    let message = rest.trim_start_matches('\u{FEFF}').trim().to_owned();

    Some(SyslogMessage {
        facility,
        severity,
        timestamp,
        hostname: Some(hostname.to_owned()),
        app_name: Some(app_name.to_owned()),
        proc_id: Some(proc_id.to_owned()),
        msg_id: Some(msg_id.to_owned()),
        structured_data,
        message,
    })
}

fn parse_priority(s: &str) -> Option<(u16, &str)> {
    if !s.starts_with('<') { return None; }
    let end = s.find('>')?;
    let pri: u16 = s[1..end].parse().ok()?;
    Some((pri, &s[end + 1..]))
}

fn next_field(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start_matches(' ');
    if s.is_empty() { return None; }
    let end = s.find(' ').unwrap_or(s.len());
    Some((&s[..end], &s[end..]))
}

fn parse_5424_timestamp(s: &str) -> Option<TimestampNs> {
    if s == "-" { return None; }
    // RFC 3339: 2023-10-05T12:34:56.123456789Z or +00:00
    let dt = DateTime::parse_from_rfc3339(s).ok()?;
    Some(dt.timestamp_nanos_opt()?)
}

fn parse_structured_data(s: &str) -> Option<(HashMap<String, HashMap<String, String>>, &str)> {
    let s = s.trim_start_matches(' ');
    if s.starts_with('-') {
        return Some((HashMap::new(), &s[1..]));
    }
    if !s.starts_with('[') { return None; }

    let mut result: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut rest = s;

    while rest.starts_with('[') {
        rest = &rest[1..]; // consume '['
        let id_end = rest.find(|c: char| c == ' ' || c == ']').unwrap_or(rest.len());
        let sd_id = rest[..id_end].to_owned();
        rest = &rest[id_end..];

        let mut params: HashMap<String, String> = HashMap::new();
        while rest.starts_with(' ') {
            rest = &rest[1..];
            if rest.starts_with(']') { break; }
            // Parse param: key="value"
            let eq = rest.find('=')?;
            let key = rest[..eq].to_owned();
            rest = &rest[eq + 1..];
            if !rest.starts_with('"') { break; }
            rest = &rest[1..]; // consume opening "
            let mut val = String::new();
            loop {
                match rest.chars().next() {
                    None => break,
                    Some('"') => { rest = &rest[1..]; break; }
                    Some('\\') => {
                        rest = &rest[1..];
                        if let Some(c) = rest.chars().next() {
                            val.push(c);
                            rest = &rest[c.len_utf8()..];
                        }
                    }
                    Some(c) => {
                        val.push(c);
                        rest = &rest[c.len_utf8()..];
                    }
                }
            }
            params.insert(key, val);
        }
        if rest.starts_with(']') { rest = &rest[1..]; }
        result.insert(sd_id, params);
    }

    Some((result, rest))
}

// ── RFC 3164 (BSD syslog) ─────────────────────────────────────────────────────
//
// <PRI>TIMESTAMP HOSTNAME TAG: MESSAGE
// TIMESTAMP = "Mmm DD HH:MM:SS" (no year)

pub fn parse_rfc3164(line: &str) -> Option<SyslogMessage> {
    let line = line.trim();
    let (pri, rest) = parse_priority(line)?;
    let facility = (pri >> 3) as u8;
    let severity = (pri & 0x07) as u8;

    // Timestamp: "Jan  1 00:00:00 " (15 chars) but allow flexible parsing.
    let rest = rest.trim_start();
    let (timestamp, rest) = if rest.len() >= 15 {
        let ts_str = &rest[..15];
        let ts = parse_3164_timestamp(ts_str)
            .unwrap_or_else(|| Utc::now().timestamp_nanos_opt().unwrap_or(0));
        (ts, rest[15..].trim_start())
    } else {
        (Utc::now().timestamp_nanos_opt().unwrap_or(0), rest)
    };

    // Hostname
    let (hostname, rest) = next_field(rest)?;
    let rest = rest.trim_start();

    // TAG (optional, ends with ':' or '[')
    let (app_name, proc_id, message_rest) = if let Some(colon) = rest.find(": ") {
        let tag = &rest[..colon];
        let (app, pid) = if let Some(bracket) = tag.find('[') {
            let a = &tag[..bracket];
            let pid_end = tag.find(']').unwrap_or(tag.len());
            (a, &tag[bracket + 1..pid_end])
        } else {
            (tag, "")
        };
        (app.to_owned(), if pid.is_empty() { None } else { Some(pid.to_owned()) }, &rest[colon + 2..])
    } else {
        (String::new(), None, rest)
    };

    Some(SyslogMessage {
        facility,
        severity,
        timestamp,
        hostname: Some(hostname.to_owned()),
        app_name: if app_name.is_empty() { None } else { Some(app_name) },
        proc_id,
        msg_id: None,
        structured_data: HashMap::new(),
        message: message_rest.to_owned(),
    })
}

fn parse_3164_timestamp(s: &str) -> Option<TimestampNs> {
    // "Jan  1 00:00:00" — no timezone, assume UTC, current year.
    let s = s.trim();
    let year = Utc::now().format("%Y").to_string();
    let full = format!("{} {} UTC", year, s);
    NaiveDateTime::parse_from_str(&full, "%Y %b %e %H:%M:%S UTC")
        .ok()
        .and_then(|ndt| Utc.from_utc_datetime(&ndt).timestamp_nanos_opt())
}

/// Auto-detect RFC 5424 vs RFC 3164 and parse.
pub fn parse_syslog(line: &str) -> Option<SyslogMessage> {
    // RFC 5424 has "<PRI>1 " (version digit after priority)
    if let Some((_, rest)) = parse_priority(line) {
        if rest.starts_with('1') || rest.starts_with("1 ") {
            if let Some(msg) = parse_rfc5424(line) {
                return Some(msg);
            }
        }
    }
    parse_rfc3164(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc5424_full() {
        let line = r#"<165>1 2023-10-05T12:34:56.123Z myhost myapp 1234 ID47 [exampleSDID@32473 iut="3" eventSource="Application"] Hello, syslog!"#;
        let msg = parse_rfc5424(line).unwrap();
        assert_eq!(msg.facility, 20);
        assert_eq!(msg.severity, 5);
        assert_eq!(msg.hostname.as_deref(), Some("myhost"));
        assert_eq!(msg.app_name.as_deref(), Some("myapp"));
        assert_eq!(msg.message, "Hello, syslog!");
        assert!(!msg.structured_data.is_empty());
    }

    #[test]
    fn rfc5424_nil_structured_data() {
        let line = "<34>1 2003-10-11T22:14:15.003Z mymachine su - ID47 - BOM'su root' failed";
        let msg = parse_rfc5424(line).unwrap();
        assert_eq!(msg.severity, 2);
        assert!(msg.structured_data.is_empty());
    }

    #[test]
    fn rfc3164_basic() {
        let line = "<13>Jan  5 00:01:02 myhost myapp[999]: connection failed";
        let msg = parse_rfc3164(line).unwrap();
        assert_eq!(msg.facility, 1);
        assert_eq!(msg.severity, 5);
        assert_eq!(msg.hostname.as_deref(), Some("myhost"));
        assert_eq!(msg.app_name.as_deref(), Some("myapp"));
        assert!(msg.message.contains("connection failed"));
    }

    #[test]
    fn auto_detect_5424() {
        let line = "<34>1 2023-01-01T00:00:00Z host app - - - test message";
        let msg = parse_syslog(line).unwrap();
        assert_eq!(msg.message, "test message");
    }

    #[test]
    fn auto_detect_3164() {
        let line = "<13>Jan  5 00:01:02 myhost tag: msg";
        let msg = parse_syslog(line).unwrap();
        assert!(msg.message.contains("msg"));
    }

    #[test]
    fn to_entry_and_labels() {
        let line = "<34>1 2023-01-01T00:00:00Z myhost myapp 123 - - test message";
        let msg = parse_rfc5424(line).unwrap();
        let (entry, labels) = msg.to_entry_and_labels();
        assert_eq!(entry.line, "test message");
        assert_eq!(labels.get("app"), Some("myapp"));
        assert_eq!(labels.get("hostname"), Some("myhost"));
    }
}
