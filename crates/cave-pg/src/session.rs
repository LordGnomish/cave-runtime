//! Per-connection session state.
//!
//! Each TCP connection gets a `Session` which owns an `Executor` (with its
//! own transaction state, prepared statements, portals and config) plus the
//! protocol-level state machine (startup, auth, query mode).

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use crate::auth::{AuthMethod, Authenticator, UserRecord};
use crate::error::{Error, PgError, Result, SqlState};
use crate::executor::Executor;
use crate::storage::Engine;
use crate::types::{CommandResult, FormatCode, Oid, PgValue};

// ─────────────────────────────────────────────────────────────────────────────
// Notification
// ─────────────────────────────────────────────────────────────────────────────

/// A NOTIFY payload sent over a broadcast channel.
#[derive(Debug, Clone)]
pub struct Notification {
    pub channel: String,
    pub payload: String,
    pub pid: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Cancel key
// ─────────────────────────────────────────────────────────────────────────────

/// A cancel key uniquely identifies a backend process so a client can send
/// a CancelRequest message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CancelKey {
    pub pid: u32,
    pub secret: u32,
}

impl CancelKey {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT_PID: AtomicU32 = AtomicU32::new(1000);
        let pid = NEXT_PID.fetch_add(1, Ordering::SeqCst);
        let secret = rand::random::<u32>();
        Self { pid, secret }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Session
// ─────────────────────────────────────────────────────────────────────────────

/// Session state for a single client connection.
pub struct Session {
    /// The SQL executor.
    pub executor: Executor,
    /// Cancel key for this backend.
    pub cancel_key: CancelKey,
    /// Channels this session is listening on.
    pub listen_channels: Vec<String>,
    /// Sender side of the global notification broadcast.
    pub notify_tx: broadcast::Sender<Notification>,
    /// Receiver side.
    pub notify_rx: broadcast::Receiver<Notification>,
    /// Whether SSL was negotiated.
    pub ssl_enabled: bool,
}

impl Session {
    pub fn new(engine: Arc<Engine>, notify_tx: broadcast::Sender<Notification>) -> Self {
        let notify_rx = notify_tx.subscribe();
        Self {
            executor: Executor::new(engine),
            cancel_key: CancelKey::new(),
            listen_channels: Vec::new(),
            notify_tx,
            notify_rx,
            ssl_enabled: false,
        }
    }

    /// Apply startup parameters to session config.
    pub fn apply_startup_params(&mut self, params: &HashMap<String, String>) {
        for (key, value) in params {
            match key.as_str() {
                "database" => self.executor.config.current_database = value.clone(),
                "user" => self.executor.config.current_user = value.clone(),
                "application_name" => self.executor.config.application_name = value.clone(),
                "client_encoding" => self.executor.config.client_encoding = value.to_uppercase(),
                "options" => {
                    // Parse -c option=value pairs from the options string
                    for part in value.split_whitespace() {
                        if let Some(opt) = part.strip_prefix("-c") {
                            let opt = opt.trim();
                            if let Some((k, v)) = opt.split_once('=') {
                                let _ = self.executor.config.set(k.trim(), v.trim());
                            }
                        }
                    }
                }
                other => {
                    let _ = self.executor.config.set(other, value);
                }
            }
        }
    }

    /// Listen on a channel.
    pub fn listen(&mut self, channel: &str) {
        let ch = channel.to_lowercase();
        if !self.listen_channels.contains(&ch) {
            self.listen_channels.push(ch);
        }
    }

    /// Unlisten from a channel (or all channels).
    pub fn unlisten(&mut self, channel: Option<&str>) {
        if let Some(ch) = channel {
            self.listen_channels.retain(|c| c != ch);
        } else {
            self.listen_channels.clear();
        }
    }

    /// Send a notification to all listeners.
    pub fn notify(&self, channel: &str, payload: &str) {
        let notif = Notification {
            channel: channel.to_string(),
            payload: payload.to_string(),
            pid: self.cancel_key.pid,
        };
        let _ = self.notify_tx.send(notif);
    }

    /// Check if there are pending notifications for this session.
    pub fn pending_notifications(&mut self) -> Vec<Notification> {
        let mut pending = Vec::new();
        loop {
            match self.notify_rx.try_recv() {
                Ok(n) if self.listen_channels.contains(&n.channel.to_lowercase()) => {
                    pending.push(n);
                }
                Ok(_) => {} // Not listening on this channel
                Err(_) => break,
            }
        }
        pending
    }

    /// Initial configuration parameter messages to send after auth.
    pub fn startup_parameter_messages(&self) -> Vec<(&'static str, String)> {
        vec![
            ("server_version", "16.0".to_string()),
            ("server_encoding", "UTF8".to_string()),
            ("client_encoding", self.executor.config.client_encoding.clone()),
            ("application_name", self.executor.config.application_name.clone()),
            ("is_superuser", "on".to_string()),
            ("session_authorization", self.executor.config.current_user.clone()),
            ("DateStyle", self.executor.config.date_style.clone()),
            ("IntervalStyle", "postgres".to_string()),
            ("TimeZone", self.executor.config.timezone.clone()),
            ("integer_datetimes", "on".to_string()),
            ("standard_conforming_strings", if self.executor.config.standard_conforming_strings { "on" } else { "off" }.to_string()),
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Session registry (for cancel key lookup)
// ─────────────────────────────────────────────────────────────────────────────

use parking_lot::Mutex;
use std::collections::HashMap as StdHashMap;

/// Global registry mapping cancel key → session cancellation token.
pub struct SessionRegistry {
    /// pid → (secret, cancel_token)
    sessions: Mutex<StdHashMap<u32, (u32, tokio_util::sync::CancellationToken)>>,
}

impl SessionRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(StdHashMap::new()),
        })
    }

    /// Register a new session. Returns the cancellation token.
    pub fn register(&self, key: CancelKey) -> tokio_util::sync::CancellationToken {
        let token = tokio_util::sync::CancellationToken::new();
        self.sessions.lock().insert(key.pid, (key.secret, token.clone()));
        token
    }

    /// Deregister a session.
    pub fn deregister(&self, key: CancelKey) {
        self.sessions.lock().remove(&key.pid);
    }

    /// Cancel a session by cancel key.
    pub fn cancel(&self, pid: u32, secret: u32) -> bool {
        if let Some((stored_secret, token)) = self.sessions.lock().get(&pid) {
            if *stored_secret == secret {
                token.cancel();
                return true;
            }
        }
        false
    }
}
