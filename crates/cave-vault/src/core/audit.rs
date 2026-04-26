use ring::hmac;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub time: String,
    pub audit_type: String,
    pub request: AuditRequest,
    pub auth: Option<AuditAuth>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuditRequest {
    pub id: String,
    pub operation: String,
    pub mount_type: String,
    pub path: String,
    pub remote_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuditAuth {
    pub client_token: String,
    pub accessor: String,
    pub display_name: String,
    pub policies: Vec<String>,
    pub token_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditBackend {
    pub path: String,
    pub backend_type: AuditBackendType,
    pub description: String,
    pub options: HashMap<String, String>,
    pub local: bool,
    pub seal_wrap: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuditBackendType {
    File,
    Syslog,
    Socket,
}

impl AuditBackend {
    /// Cite: openbao `builtin/audit/file/backend.go::Factory` — the `file`
    /// backend reads its target path from `options["file_path"]`.
    pub fn file_path(&self) -> Option<PathBuf> {
        self.options.get("file_path").map(PathBuf::from)
    }

    /// Cite: openbao `builtin/audit/syslog/backend.go::Factory` — syslog
    /// uses `options["facility"]` (default LOCAL0) + `options["tag"]`
    /// (default "vault"). cave's syslog "wire format" is a single line
    /// shaped like a real syslog message:
    /// `<priority> tag: {json}`.
    pub fn syslog_format(&self, json: &str) -> Option<String> {
        if self.backend_type != AuditBackendType::Syslog {
            return None;
        }
        let facility = self.options.get("facility")
            .map(String::as_str).unwrap_or("LOCAL0");
        let tag = self.options.get("tag")
            .map(String::as_str).unwrap_or("vault");
        // openbao maps facility×severity to the standard syslog priority
        // (facility * 8 + severity). LOCAL0 = 16, INFO severity = 6 ⇒ 134.
        let pri = match facility {
            "KERN"   =>  6, "USER"   => 14, "MAIL"   => 22, "DAEMON" => 30,
            "AUTH"   => 38, "LPR"    => 54, "NEWS"   => 62, "UUCP"   => 70,
            "CRON"   => 78, "AUTHPRIV" => 86, "FTP" => 94, "LOCAL0" => 134,
            "LOCAL1" => 142, "LOCAL2" => 150, "LOCAL3" => 158, "LOCAL4" => 166,
            "LOCAL5" => 174, "LOCAL6" => 182, "LOCAL7" => 190,
            _ => 134,
        };
        Some(format!("<{}> {}: {}", pri, tag, json))
    }
}

pub struct AuditLogger {
    hmac_key: Vec<u8>,
    backends: Mutex<HashMap<String, AuditBackend>>,
    log_buffer: Mutex<Vec<AuditEntry>>,
}

/// Cite: openbao `audit/format_json.go::JSONFormatWriter.WriteRequest`
/// + the integrator pattern used by signed-log forwarders. Pairs the
/// canonical JSON envelope with an HMAC tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedAuditEnvelope {
    pub json: String,
    pub signature: String,
}

impl AuditLogger {
    pub fn new(hmac_key: Vec<u8>) -> Self {
        Self {
            hmac_key,
            backends: Mutex::new(HashMap::new()),
            log_buffer: Mutex::new(Vec::new()),
        }
    }

    pub fn hmac_value(&self, value: &str) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.hmac_key);
        let tag = hmac::sign(&key, value.as_bytes());
        hex::encode(tag.as_ref())
    }

    pub fn log(&self, mut entry: AuditEntry) {
        if let Some(ref mut auth) = entry.auth {
            auth.client_token = self.hmac_value(&auth.client_token);
            auth.accessor = self.hmac_value(&auth.accessor);
        }
        let json = serde_json::to_string(&entry).unwrap_or_default();
        tracing::debug!(audit = %json, "vault audit");
        // Fan out to every enabled backend (file/syslog). Cite: openbao
        // `audit/audit.go::broker.LogRequest` which fans out to each
        // registered backend before the request is allowed to proceed.
        if let Ok(backends) = self.backends.lock() {
            for backend in backends.values() {
                let _ = self.dispatch(backend, &json);
            }
        }
        if let Ok(mut buf) = self.log_buffer.lock() {
            if buf.len() < 10_000 {
                buf.push(entry);
            }
        }
    }

    /// Cite: openbao `builtin/audit/file/backend.go::LogRequest` and
    /// `builtin/audit/syslog/backend.go::LogRequest` — the broker
    /// invokes one backend at a time. cave returns the JSON line that
    /// was actually written so callers (tests) can verify shape.
    fn dispatch(&self, backend: &AuditBackend, json: &str) -> std::io::Result<String> {
        match backend.backend_type {
            AuditBackendType::File => {
                if let Some(path) = backend.file_path() {
                    use std::io::Write;
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut f = std::fs::OpenOptions::new()
                        .create(true).append(true).open(&path)?;
                    writeln!(f, "{}", json)?;
                }
                Ok(json.to_string())
            }
            AuditBackendType::Syslog => Ok(backend.syslog_format(json).unwrap_or_default()),
            AuditBackendType::Socket => Ok(json.to_string()),
        }
    }

    /// Cite: openbao `audit/format.go:34` (AuditFormatter) + `:71`
    /// (HashAuth, HashRequest) — produces the same JSON envelope the
    /// formatter would emit, plus a detached HMAC tag derived from the
    /// audit salt. Used by integrators that ship the envelope to a
    /// signed-log service (e.g. transparent log).
    pub fn signed_envelope(&self, entry: &AuditEntry) -> SignedAuditEnvelope {
        let mut redacted = entry.clone();
        if let Some(ref mut auth) = redacted.auth {
            auth.client_token = self.hmac_value(&auth.client_token);
            auth.accessor = self.hmac_value(&auth.accessor);
        }
        let json = serde_json::to_string(&redacted).unwrap_or_default();
        let signature = self.hmac_value(&json);
        SignedAuditEnvelope { json, signature }
    }

    /// Verify a previously-issued [`SignedAuditEnvelope`]. Returns true
    /// iff the HMAC over `json` matches `signature` under the same key.
    pub fn verify_envelope(&self, env: &SignedAuditEnvelope) -> bool {
        env.signature == self.hmac_value(&env.json)
    }

    pub fn enable(&self, path: &str, backend: AuditBackend) {
        if let Ok(mut backends) = self.backends.lock() {
            backends.insert(path.to_string(), backend);
        }
    }

    pub fn disable(&self, path: &str) -> bool {
        if let Ok(mut backends) = self.backends.lock() {
            backends.remove(path).is_some()
        } else {
            false
        }
    }

    pub fn list_backends(&self) -> HashMap<String, AuditBackend> {
        self.backends.lock().map(|b| b.clone()).unwrap_or_default()
    }

    pub fn recent_entries(&self, limit: usize) -> Vec<AuditEntry> {
        self.log_buffer.lock().map(|buf| {
            let start = buf.len().saturating_sub(limit);
            buf[start..].to_vec()
        }).unwrap_or_default()
    }
}
