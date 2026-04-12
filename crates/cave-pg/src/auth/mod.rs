//! PostgreSQL authentication — trust, cleartext password, MD5, and SCRAM-SHA-256.

pub mod md5;
pub mod scram;

use crate::error::{Error, PgError, Result, SqlState};
use crate::protocol::message::AuthRequest;
use rand::Rng;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Authentication method configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Which authentication method to use for a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    /// No password required — accept all connections.
    Trust,
    /// Plaintext password (insecure, but useful for internal connections).
    Password,
    /// MD5-hashed password challenge.
    Md5,
    /// SCRAM-SHA-256 (RFC 7677) — the recommended modern method.
    ScramSha256,
}

impl Default for AuthMethod {
    fn default() -> Self {
        Self::Trust
    }
}

/// A user record.
#[derive(Debug, Clone)]
pub struct UserRecord {
    pub username: String,
    /// Stored password (plaintext or SCRAM verifier string).
    pub password: Option<String>,
    pub superuser: bool,
    pub create_db: bool,
    pub create_role: bool,
    pub replication: bool,
    pub oid: u32,
}

impl UserRecord {
    pub fn superuser(username: impl Into<String>, oid: u32) -> Self {
        Self {
            username: username.into(),
            password: None,
            superuser: true,
            create_db: true,
            create_role: true,
            replication: true,
            oid,
        }
    }

    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Authentication state machine
// ─────────────────────────────────────────────────────────────────────────────

/// State of the authentication handshake.
#[derive(Debug)]
pub enum AuthState {
    /// Authentication not yet started.
    Pending,
    /// Waiting for client password response (MD5 challenge sent).
    WaitingMd5 { salt: [u8; 4], username: String },
    /// SCRAM first message sent, waiting for client-first.
    WaitingScramClientFirst { server_nonce: String, username: String },
    /// SCRAM server-first sent, waiting for client-final.
    WaitingScramClientFinal { scram_ctx: scram::ServerScramContext },
    /// Authentication succeeded.
    Authenticated { username: String },
    /// Authentication failed.
    Failed,
}

/// Drives the authentication handshake.
pub struct Authenticator {
    pub method: AuthMethod,
    pub users: HashMap<String, UserRecord>,
}

impl Authenticator {
    pub fn new(method: AuthMethod) -> Self {
        Self { method, users: HashMap::new() }
    }

    pub fn add_user(&mut self, record: UserRecord) {
        self.users.insert(record.username.clone(), record);
    }

    /// Produce the first backend message(s) for a new connection.
    /// Returns the AuthRequest to send and the new AuthState.
    pub fn begin(&self, username: &str) -> (AuthRequest, AuthState) {
        match &self.method {
            AuthMethod::Trust => (
                AuthRequest::Ok,
                AuthState::Authenticated { username: username.to_string() },
            ),
            AuthMethod::Password => (
                AuthRequest::CleartextPassword,
                AuthState::Pending,
            ),
            AuthMethod::Md5 => {
                let mut salt = [0u8; 4];
                rand::thread_rng().fill(&mut salt);
                (
                    AuthRequest::MD5Password { salt },
                    AuthState::WaitingMd5 { salt, username: username.to_string() },
                )
            }
            AuthMethod::ScramSha256 => (
                AuthRequest::SASL(vec!["SCRAM-SHA-256".to_string()]),
                AuthState::Pending,
            ),
        }
    }

    /// Process a password response message from the client.
    /// Returns the next AuthRequest to send and the new AuthState.
    pub fn process_password(
        &self,
        state: AuthState,
        data: &[u8],
    ) -> Result<(AuthRequest, AuthState)> {
        match state {
            AuthState::Pending => {
                // Cleartext password or SCRAM client-first
                match &self.method {
                    AuthMethod::Password => {
                        // data is the cleartext password — username must be provided
                        // (In practice the session has the username)
                        Ok((AuthRequest::Ok, AuthState::Authenticated {
                            username: String::new(),
                        }))
                    }
                    AuthMethod::ScramSha256 => {
                        // Client sends: "SCRAM-SHA-256\x00" + 4-byte int32 length + client-first-message
                        let msg = std::str::from_utf8(data)
                            .map_err(|e| Error::Protocol(format!("SCRAM: invalid UTF-8: {e}")))?;

                        // Parse "SCRAM-SHA-256\x00<len><client-first-message>"
                        let null_pos = msg.find('\x00').ok_or_else(|| {
                            Error::Protocol("SCRAM: missing null separator".into())
                        })?;
                        let mech = &msg[..null_pos];
                        if mech != "SCRAM-SHA-256" {
                            return Err(Error::Protocol(format!("unsupported SASL mechanism: {mech}")));
                        }
                        // Next 4 bytes are the length (big-endian), then the message
                        let rest = &data[null_pos + 1..];
                        if rest.len() < 4 {
                            return Err(Error::Protocol("SCRAM: truncated client-first".into()));
                        }
                        let msg_len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
                        let client_first = std::str::from_utf8(&rest[4..4 + msg_len])
                            .map_err(|e| Error::Protocol(format!("SCRAM: invalid client-first: {e}")))?;

                        let (ctx, server_first) =
                            scram::server_first(client_first).map_err(|e| {
                                Error::Protocol(format!("SCRAM: {e}"))
                            })?;

                        Ok((
                            AuthRequest::SASLContinue(server_first.into_bytes()),
                            AuthState::WaitingScramClientFinal { scram_ctx: ctx },
                        ))
                    }
                    _ => Err(Error::Protocol("unexpected password message".into())),
                }
            }
            AuthState::WaitingMd5 { salt, username } => {
                let response = std::str::from_utf8(data)
                    .map_err(|e| Error::Protocol(format!("MD5 response invalid UTF-8: {e}")))?
                    .trim_end_matches('\0');

                let user = self.users.get(&username).ok_or_else(|| {
                    Error::Pg(PgError::invalid_password(&username))
                })?;

                let stored_password = user.password.as_deref().unwrap_or("");
                let expected = md5::compute_md5_response(&username, stored_password, &salt);

                if response == expected {
                    Ok((
                        AuthRequest::Ok,
                        AuthState::Authenticated { username },
                    ))
                } else {
                    Err(Error::Pg(PgError::invalid_password(&username)))
                }
            }
            AuthState::WaitingScramClientFinal { scram_ctx } => {
                let client_final = std::str::from_utf8(data)
                    .map_err(|e| Error::Protocol(format!("SCRAM: invalid client-final: {e}")))?;

                let (username, server_final) =
                    scram::server_final(&scram_ctx, client_final, &self.users)
                        .map_err(|e| Error::Protocol(format!("SCRAM: {e}")))?;

                Ok((
                    AuthRequest::SASLFinal(server_final.into_bytes()),
                    AuthState::Authenticated { username },
                ))
            }
            AuthState::WaitingScramClientFirst { .. } => {
                Err(Error::Protocol("unexpected SCRAM state".into()))
            }
            _ => Err(Error::Protocol("unexpected password message".into())),
        }
    }

    /// Verify cleartext password for a user.
    pub fn verify_password(&self, username: &str, password: &str) -> bool {
        match self.users.get(username) {
            None => false,
            Some(user) => {
                user.password.as_deref().map(|p| p == password).unwrap_or(false)
            }
        }
    }
}

/// Default authenticator — trust with a single superuser.
pub fn default_authenticator() -> Authenticator {
    let mut auth = Authenticator::new(AuthMethod::Trust);
    auth.add_user(UserRecord::superuser("postgres", 10));
    auth.add_user(UserRecord::superuser("cave", 16384));
    auth
}
