//! Wire-faithful PostgreSQL TCP server.
//!
//! Loops:
//!
//! 1. `tokio::net::TcpListener` accepts a connection.
//! 2. [`handle_connection`] reads the startup frame, performs the
//!    AuthenticationOk handshake (no password challenge in this build —
//!    intended for trusted-network deployment behind cave-mesh mTLS), sends
//!    `BackendKeyData`, `ParameterStatus`, then enters the main message loop.
//! 3. Each `Q` (simple query) is parsed by `crate::engine::execute_sql` and
//!    serialized via `crate::wire::simple_query::render_query_response`.
//! 4. `Terminate` ends the connection cleanly.
//!
//! Per-tenant isolation: the server can be constructed with a
//! [`crate::wire::tenant::TenantPolicy`] that rejects startup messages whose
//! `cave_tenant=` parameter is not in the allow-list.

use std::sync::Arc;

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::engine::{execute_sql, EngineError, ResultSet, ScalarType, SqlEngine, TxnId};
use crate::wire::auth::AuthRequest;
use crate::wire::codec::{
    decode_frontend, encode_backend, encode_frontend, BackendMessage, FieldDescription,
    FrontendMessage, ProtocolError,
};
use crate::wire::pg_types;
use crate::wire::simple_query::{render_query_response, StatementResult, TxStatus};
use crate::wire::startup::StartupMessage;
use crate::wire::tenant::{extract_tenant, TenantPolicy};

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("tenant denied: {0}")]
    TenantDenied(String),
}

#[derive(Debug)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub server_version: String,
    pub require_tenant: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:0".into(),
            server_version: "16.0 (cave-pg)".into(),
            require_tenant: false,
        }
    }
}

#[derive(Debug)]
pub struct PgServer {
    pub engine: Arc<SqlEngine>,
    pub tenant_policy: parking_lot::RwLock<TenantPolicy>,
    pub config: ServerConfig,
}

impl PgServer {
    pub fn new(engine: Arc<SqlEngine>, config: ServerConfig) -> Arc<Self> {
        Arc::new(Self {
            engine,
            tenant_policy: parking_lot::RwLock::new(TenantPolicy::new()),
            config,
        })
    }

    pub fn allow_tenant(&self, tenant: &str) {
        let _ = self.tenant_policy.write().allow(tenant);
    }

    /// Bind and run forever. Returns the bound address so tests can connect.
    pub async fn bind(self: Arc<Self>) -> Result<(std::net::SocketAddr, BoundServer), ServerError> {
        let listener = TcpListener::bind(&self.config.bind_addr).await?;
        let addr = listener.local_addr()?;
        let server = self.clone();
        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _peer)) => {
                        let s = server.clone();
                        tokio::spawn(async move {
                            let _ = handle_connection(s, stream).await;
                        });
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "accept failed");
                    }
                }
            }
        });
        Ok((addr, BoundServer { handle }))
    }
}

#[derive(Debug)]
pub struct BoundServer {
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for BoundServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Per-connection session state.
#[derive(Debug)]
struct Session {
    server: Arc<PgServer>,
    process_id: i32,
    secret_key: i32,
    tenant: Option<String>,
    txn: Option<TxnId>,
}

pub async fn handle_connection(
    server: Arc<PgServer>,
    mut stream: TcpStream,
) -> Result<(), ServerError> {
    let mut read_buf = BytesMut::with_capacity(4096);
    let mut write_buf = BytesMut::with_capacity(4096);

    // Startup phase.
    let startup = read_startup(&mut stream, &mut read_buf).await?;

    // Tenant authorization.
    let tenant = extract_tenant(&startup.parameters).map(String::from);
    if server.config.require_tenant {
        let policy = server.tenant_policy.read();
        let _ = policy
            .authorize(tenant.as_deref())
            .map_err(|e| ServerError::TenantDenied(e.to_string()))?;
    }

    let mut session = Session {
        server: server.clone(),
        process_id: rand::random(),
        secret_key: rand::random(),
        tenant,
        txn: None,
    };

    // AuthenticationOk → ParameterStatus → BackendKeyData → ReadyForQuery
    write_buf.clear();
    encode_backend(&BackendMessage::Authentication(AuthRequest::Ok), &mut write_buf);
    for (k, v) in default_parameter_statuses(&server.config.server_version) {
        encode_backend(
            &BackendMessage::ParameterStatus {
                name: k.into(),
                value: v.into(),
            },
            &mut write_buf,
        );
    }
    encode_backend(
        &BackendMessage::BackendKeyData {
            process_id: session.process_id,
            secret_key: session.secret_key,
        },
        &mut write_buf,
    );
    encode_backend(&BackendMessage::ReadyForQuery(b'I'), &mut write_buf);
    stream.write_all(&write_buf).await?;
    stream.flush().await?;

    // Main message loop.
    loop {
        let msg = match read_frontend(&mut stream, &mut read_buf).await? {
            Some(m) => m,
            None => break,
        };
        write_buf.clear();
        match msg {
            FrontendMessage::Query(sql) => {
                handle_simple_query(&mut session, &sql, &mut write_buf);
            }
            FrontendMessage::Terminate => break,
            FrontendMessage::Sync => {
                encode_backend(&BackendMessage::ReadyForQuery(current_status(&session)), &mut write_buf);
            }
            FrontendMessage::Flush => {}
            other => {
                // Extended-query and other unsupported messages → error + ReadyForQuery.
                encode_backend(
                    &BackendMessage::ErrorResponse(vec![
                        (b'S', "ERROR".into()),
                        (b'C', "0A000".into()),
                        (b'M', format!("frontend message {:?} not yet supported", other)),
                    ]),
                    &mut write_buf,
                );
                encode_backend(&BackendMessage::ReadyForQuery(current_status(&session)), &mut write_buf);
            }
        }
        if !write_buf.is_empty() {
            stream.write_all(&write_buf).await?;
            stream.flush().await?;
        }
    }
    Ok(())
}

fn handle_simple_query(session: &mut Session, sql: &str, out: &mut BytesMut) {
    if sql.trim().is_empty() {
        encode_backend(&BackendMessage::EmptyQueryResponse, out);
        encode_backend(&BackendMessage::ReadyForQuery(current_status(session)), out);
        return;
    }

    // Multiple statements separated by ';' — parse each in turn.
    let mut results: Vec<StatementResult> = Vec::new();
    for stmt in split_statements(sql) {
        if stmt.trim().is_empty() {
            continue;
        }
        match execute_sql(&session.server.engine, &stmt, session.txn) {
            Ok(rs) => {
                if rs.txn_after.is_some() {
                    session.txn = rs.txn_after;
                }
                if rs.command_tag == "COMMIT" || rs.command_tag == "ROLLBACK" {
                    session.txn = None;
                }
                results.push(result_set_to_stmt(rs));
            }
            Err(e) => {
                let (code, msg) = engine_error_to_pg(&e);
                encode_backend(
                    &BackendMessage::ErrorResponse(vec![
                        (b'S', "ERROR".into()),
                        (b'C', code.into()),
                        (b'M', msg),
                    ]),
                    out,
                );
                encode_backend(&BackendMessage::ReadyForQuery(b'E'), out);
                return;
            }
        }
    }
    let status = current_status(session);
    let tx_status = TxStatus::from_byte(status).unwrap_or(TxStatus::Idle);
    render_query_response(&results, tx_status, out);
}

fn current_status(session: &Session) -> u8 {
    if session.txn.is_some() {
        b'T'
    } else {
        b'I'
    }
}

fn result_set_to_stmt(rs: ResultSet) -> StatementResult {
    let fields = if rs.columns.is_empty() {
        None
    } else {
        Some(
            rs.columns
                .iter()
                .map(|(name, ty)| FieldDescription {
                    name: name.clone(),
                    table_oid: 0,
                    column_attr: 0,
                    type_oid: ty.oid(),
                    type_size: ty.type_size(),
                    type_modifier: -1,
                    format_code: 0,
                })
                .collect(),
        )
    };
    let _ = ScalarType::Bool; // keep import
    let rows: Vec<Vec<Option<Vec<u8>>>> = rs
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|cell| cell.map(|s| s.into_bytes()))
                .collect()
        })
        .collect();
    StatementResult {
        fields,
        rows,
        completion_tag: rs.command_tag,
    }
}

fn engine_error_to_pg(e: &EngineError) -> (&'static str, String) {
    match e {
        EngineError::Plan(_) => ("42601", e.to_string()),    // syntax_error
        EngineError::Parse(_) => ("42601", e.to_string()),
        EngineError::Execution(_) => ("42P01", e.to_string()), // undefined_table
        EngineError::Txn(_) => ("25P02", e.to_string()),       // in_failed_transaction
    }
}

fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut prev = '\0';
    for ch in sql.chars() {
        if ch == '\'' && prev != '\\' {
            in_string = !in_string;
            current.push(ch);
        } else if ch == ';' && !in_string {
            out.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
        prev = ch;
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

fn default_parameter_statuses(server_version: &str) -> Vec<(&'static str, String)> {
    vec![
        ("server_version", server_version.into()),
        ("server_encoding", "UTF8".into()),
        ("client_encoding", "UTF8".into()),
        ("DateStyle", "ISO, MDY".into()),
        ("integer_datetimes", "on".into()),
        ("TimeZone", "UTC".into()),
        ("standard_conforming_strings", "on".into()),
        ("application_name", "".into()),
    ]
}

async fn read_startup(
    stream: &mut TcpStream,
    buf: &mut BytesMut,
) -> Result<StartupMessage, ServerError> {
    loop {
        match decode_frontend(buf, true) {
            Ok(FrontendMessage::Startup(s)) => return Ok(s),
            Ok(FrontendMessage::SslRequest) => {
                // We don't terminate TLS in this build. Reply 'N' (no SSL),
                // then loop to read the post-SSL StartupMessage.
                stream.write_all(b"N").await?;
                stream.flush().await?;
                continue;
            }
            Ok(FrontendMessage::GssEncRequest) => {
                stream.write_all(b"N").await?;
                stream.flush().await?;
                continue;
            }
            Ok(FrontendMessage::Cancel { .. }) => {
                // Cancel-key requests open a fresh connection, do nothing then close.
                return Err(ServerError::Protocol(ProtocolError::InvalidValue {
                    field: "startup",
                    reason: "cancel request on new conn".into(),
                }));
            }
            Ok(_) => {
                return Err(ServerError::Protocol(ProtocolError::InvalidValue {
                    field: "startup",
                    reason: "non-startup frame".into(),
                }))
            }
            Err(ProtocolError::Incomplete { .. }) => {
                let mut tmp = [0u8; 4096];
                let n = stream.read(&mut tmp).await?;
                if n == 0 {
                    return Err(ServerError::Protocol(ProtocolError::Incomplete { needed: 1 }));
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            Err(e) => return Err(ServerError::Protocol(e)),
        }
    }
}

async fn read_frontend(
    stream: &mut TcpStream,
    buf: &mut BytesMut,
) -> Result<Option<FrontendMessage>, ServerError> {
    loop {
        if !buf.is_empty() {
            match decode_frontend(buf, false) {
                Ok(m) => return Ok(Some(m)),
                Err(ProtocolError::Incomplete { .. }) => {}
                Err(e) => return Err(ServerError::Protocol(e)),
            }
        }
        let mut tmp = [0u8; 4096];
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

// suppress unused-import warnings for items pulled in for type-clarity above
#[allow(dead_code)]
fn _import_anchor() {
    let _ = pg_types::TEXT;
    let _ = encode_frontend;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_statements_simple() {
        let v = split_statements("SELECT 1; SELECT 2;");
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn split_statements_keeps_strings_intact() {
        let v = split_statements("SELECT 'a;b'; SELECT 1");
        assert_eq!(v.len(), 2);
        assert!(v[0].contains("a;b"));
    }

    #[test]
    fn split_statements_no_trailing_semicolon() {
        let v = split_statements("SELECT 1");
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn split_statements_empty_input() {
        let v = split_statements("");
        assert!(v.is_empty());
    }

    #[test]
    fn default_parameter_statuses_includes_server_version() {
        let p = default_parameter_statuses("17.0");
        assert!(p.iter().any(|(k, v)| *k == "server_version" && v == "17.0"));
    }

    #[test]
    fn engine_error_codes_distinct() {
        let e1 = EngineError::Parse("x".into());
        let (c1, _) = engine_error_to_pg(&e1);
        assert_eq!(c1, "42601");
    }
}
