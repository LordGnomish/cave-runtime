use ring::hmac;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditBackendType {
    File,
    Syslog,
    Socket,
}

pub struct AuditLogger {
    hmac_key: Vec<u8>,
    backends: Mutex<HashMap<String, AuditBackend>>,
    log_buffer: Mutex<Vec<AuditEntry>>,
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
        if let Ok(mut buf) = self.log_buffer.lock() {
            if buf.len() < 10_000 {
                buf.push(entry);
            }
        }
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
