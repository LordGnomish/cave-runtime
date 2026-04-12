//! PostgreSQL wire protocol v3 TCP server.
//!
//! Listens for connections, negotiates SSL, authenticates clients, then drives
//! the simple and extended query protocols. Handles COPY, LISTEN/NOTIFY, and
//! cancellation.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use bytes::BytesMut;
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_util::codec::Framed;
use tracing::{debug, error, info, warn};

use crate::auth::{AuthMethod, AuthState, Authenticator, UserRecord};
use crate::error::{Error, PgError, Result, SqlState};
use crate::executor::{Executor, PreparedStatement, Portal};
use crate::protocol::codec::PgCodec;
use crate::protocol::message::{
    AuthRequest as AuthReq, BackendMessage, BindMessage, CloseMessage, DescribeKind,
    DescribeMessage, ExecuteMessage, FrontendMessage, ParseMessage, StartupMessage,
    TransactionStatus,
};
use crate::session::{CancelKey, Notification, Session, SessionRegistry};
use crate::storage::Engine;
use crate::types::{ColumnDesc, CommandResult, FormatCode, Oid, PgValue, ResultSet, oid};

// ─────────────────────────────────────────────────────────────────────────────
// Server
// ─────────────────────────────────────────────────────────────────────────────

/// The cave-pg TCP server.
pub struct Server {
    engine: Arc<Engine>,
    listener: TcpListener,
    notify_tx: broadcast::Sender<Notification>,
    registry: Arc<SessionRegistry>,
    auth_method: AuthMethod,
    users: HashMap<String, UserRecord>,
}

impl Server {
    /// Create a new server bound to the given address.
    pub async fn new(engine: Arc<Engine>, addr: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let (notify_tx, _) = broadcast::channel(1024);
        let registry = SessionRegistry::new();

        let mut users = HashMap::new();
        users.insert("postgres".to_string(), UserRecord::superuser("postgres", 10));

        info!("cave-pg listening on {}", addr);
        Ok(Self { engine, listener, notify_tx, registry, auth_method: AuthMethod::Trust, users })
    }

    /// Add a user with a password credential.
    pub fn add_user(&mut self, name: impl Into<String>, password: impl Into<String>) {
        let name = name.into();
        let password = password.into();
        let record = UserRecord::superuser(&name, crate::storage::alloc_oid())
            .with_password(password);
        self.users.insert(name, record);
    }

    /// Set the authentication method (default: Trust).
    pub fn set_auth_method(&mut self, method: AuthMethod) {
        self.auth_method = method;
    }

    /// Run the server loop indefinitely.
    pub async fn run(self) -> std::io::Result<()> {
        let state = Arc::new(ServerState {
            engine: self.engine,
            notify_tx: self.notify_tx,
            registry: self.registry,
            auth_method: self.auth_method,
            users: self.users,
        });

        loop {
            match self.listener.accept().await {
                Ok((stream, addr)) => {
                    debug!("new connection from {}", addr);
                    let state = state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, addr, state).await {
                            warn!("connection from {} ended: {}", addr, e);
                        }
                    });
                }
                Err(e) => error!("accept error: {}", e),
            }
        }
    }
}

struct ServerState {
    engine: Arc<Engine>,
    notify_tx: broadcast::Sender<Notification>,
    registry: Arc<SessionRegistry>,
    auth_method: AuthMethod,
    users: HashMap<String, UserRecord>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Connection handler
// ─────────────────────────────────────────────────────────────────────────────

async fn handle_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    state: Arc<ServerState>,
) -> Result<()> {
    let startup = read_startup_message(&mut stream).await?;

    if startup.is_ssl_request() {
        use tokio::io::AsyncWriteExt;
        stream.write_all(b"N").await?;
        let startup = read_startup_message(&mut stream).await?;
        return drive_connection(stream, addr, startup, state).await;
    }

    if startup.is_cancel_request() {
        return Ok(());
    }

    if startup.is_gssenc_request() {
        use tokio::io::AsyncWriteExt;
        stream.write_all(b"N").await?;
        let startup = read_startup_message(&mut stream).await?;
        return drive_connection(stream, addr, startup, state).await;
    }

    drive_connection(stream, addr, startup, state).await
}

async fn drive_connection(
    stream: TcpStream,
    _addr: SocketAddr,
    startup: StartupMessage,
    state: Arc<ServerState>,
) -> Result<()> {
    let mut framed = Framed::new(stream, PgCodec);
    let mut session = Session::new(state.engine.clone(), state.notify_tx.clone());
    session.apply_startup_params(&startup.parameters);

    // ── Authentication ──────────────────────────────────────────────────────
    let username = session.executor.config.current_user.clone();
    let authenticator = Authenticator::new(state.auth_method.clone());
    // The users are stored in the global state; add them to the authenticator
    let mut auth = authenticator;
    for user in state.users.values() {
        auth.add_user(user.clone());
    }

    let (auth_req, mut auth_state) = auth.begin(&username);
    framed.send(BackendMessage::Authentication(auth_req)).await?;

    // Auth exchange
    loop {
        match &auth_state {
            AuthState::Authenticated { .. } => break,
            AuthState::Failed => {
                framed.send(BackendMessage::ErrorResponse(PgError::invalid_password(&username))).await?;
                return Ok(());
            }
            _ => {
                match framed.next().await {
                    None => return Ok(()),
                    Some(Err(e)) => return Err(e),
                    Some(Ok(FrontendMessage::Password(data))) => {
                        match auth.process_password(auth_state, &data) {
                            Ok((req, next_state)) => {
                                framed.send(BackendMessage::Authentication(req)).await?;
                                auth_state = next_state;
                            }
                            Err(e) => {
                                framed.send(BackendMessage::ErrorResponse(e.to_pg_error())).await?;
                                return Ok(());
                            }
                        }
                    }
                    Some(Ok(_)) => {
                        framed.send(BackendMessage::ErrorResponse(
                            PgError::error(SqlState::PROTOCOL_VIOLATION, "unexpected message during auth")
                        )).await?;
                        return Ok(());
                    }
                }
            }
        }
    }

    // ── Startup parameter messages ──────────────────────────────────────────
    for (name, value) in session.startup_parameter_messages() {
        framed.send(BackendMessage::ParameterStatus { name: name.to_string(), value }).await?;
    }

    framed.send(BackendMessage::BackendKeyData {
        pid: session.cancel_key.pid,
        secret_key: session.cancel_key.secret,
    }).await?;

    let _cancel_token = state.registry.register(session.cancel_key);

    framed.send(BackendMessage::ReadyForQuery(TransactionStatus::Idle)).await?;

    // ── Main loop ───────────────────────────────────────────────────────────
    let result = session_loop(&mut framed, &mut session).await;
    state.registry.deregister(session.cancel_key);
    result
}

async fn session_loop(
    framed: &mut Framed<TcpStream, PgCodec>,
    session: &mut Session,
) -> Result<()> {
    loop {
        // Drain pending notifications
        for notif in session.pending_notifications() {
            framed.send(BackendMessage::NotificationResponse {
                pid: notif.pid,
                channel: notif.channel,
                payload: notif.payload,
            }).await?;
        }

        let msg = match framed.next().await {
            None => return Ok(()),
            Some(Err(e)) => return Err(e),
            Some(Ok(m)) => m,
        };

        match msg {
            FrontendMessage::Terminate => return Ok(()),
            FrontendMessage::Query(q) => simple_query(framed, session, &q.query).await?,
            FrontendMessage::Parse(p) => parse_msg(framed, session, p).await?,
            FrontendMessage::Bind(b) => bind_msg(framed, session, b).await?,
            FrontendMessage::Describe(d) => describe_msg(framed, session, d).await?,
            FrontendMessage::Execute(e) => execute_msg(framed, session, e).await?,
            FrontendMessage::Close(c) => close_msg(framed, session, c).await?,
            FrontendMessage::Sync => {
                framed.send(BackendMessage::ReadyForQuery(tx_status(session))).await?;
            }
            FrontendMessage::Flush => {} // No-op; responses already written
            _ => {}
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Simple query protocol
// ─────────────────────────────────────────────────────────────────────────────

async fn simple_query(
    framed: &mut Framed<TcpStream, PgCodec>,
    session: &mut Session,
    sql: &str,
) -> Result<()> {
    let results = match session.executor.execute_sql(sql) {
        Ok(r) => r,
        Err(e) => {
            framed.send(BackendMessage::ErrorResponse(e.to_pg_error())).await?;
            framed.send(BackendMessage::ReadyForQuery(tx_status(session))).await?;
            return Ok(());
        }
    };

    for result in results {
        send_result(framed, result).await?;
    }

    framed.send(BackendMessage::ReadyForQuery(tx_status(session))).await?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Extended query protocol
// ─────────────────────────────────────────────────────────────────────────────

async fn parse_msg(
    framed: &mut Framed<TcpStream, PgCodec>,
    session: &mut Session,
    msg: ParseMessage,
) -> Result<()> {
    match crate::executor::parse_sql(&msg.query) {
        Err(e) => {
            framed.send(BackendMessage::ErrorResponse(e.to_pg_error())).await?;
        }
        Ok(stmts) => {
            let ps = PreparedStatement {
                name: msg.statement_name.clone(),
                query: msg.query,
                param_types: msg.param_types,
                statements: stmts,
            };
            session.executor.prepared.insert(msg.statement_name, ps);
            framed.send(BackendMessage::ParseComplete).await?;
        }
    }
    Ok(())
}

async fn bind_msg(
    framed: &mut Framed<TcpStream, PgCodec>,
    session: &mut Session,
    msg: BindMessage,
) -> Result<()> {
    let ps = match session.executor.prepared.get(&msg.statement_name).cloned() {
        None => {
            framed.send(BackendMessage::ErrorResponse(PgError::error(
                SqlState::UNDEFINED_PSTATEMENT,
                format!("prepared statement \"{}\" does not exist", msg.statement_name),
            ))).await?;
            return Ok(());
        }
        Some(ps) => ps,
    };

    let portal = Portal {
        name: msg.portal_name.clone(),
        statement: ps,
        params: msg.params,
        param_formats: msg.param_formats,
        result_formats: msg.result_formats,
        cached_rows: None,
        row_pos: 0,
    };
    session.executor.portals.insert(msg.portal_name, portal);
    framed.send(BackendMessage::BindComplete).await?;
    Ok(())
}

async fn describe_msg(
    framed: &mut Framed<TcpStream, PgCodec>,
    session: &mut Session,
    msg: DescribeMessage,
) -> Result<()> {
    match msg.kind {
        DescribeKind::Statement => {
            if let Some(ps) = session.executor.prepared.get(&msg.name) {
                let param_oids = ps.param_types.clone();
                framed.send(BackendMessage::ParameterDescription(param_oids)).await?;
                framed.send(BackendMessage::NoData).await?;
            } else {
                framed.send(BackendMessage::ErrorResponse(PgError::error(
                    SqlState::UNDEFINED_PSTATEMENT,
                    format!("prepared statement \"{}\" does not exist", msg.name),
                ))).await?;
            }
        }
        DescribeKind::Portal => {
            if session.executor.portals.contains_key(&msg.name) {
                framed.send(BackendMessage::NoData).await?;
            } else {
                framed.send(BackendMessage::ErrorResponse(PgError::error(
                    SqlState::UNDEFINED_CURSOR,
                    format!("portal \"{}\" does not exist", msg.name),
                ))).await?;
            }
        }
    }
    Ok(())
}

async fn execute_msg(
    framed: &mut Framed<TcpStream, PgCodec>,
    session: &mut Session,
    msg: ExecuteMessage,
) -> Result<()> {
    let portal = match session.executor.portals.get(&msg.portal_name).cloned() {
        None => {
            framed.send(BackendMessage::ErrorResponse(PgError::error(
                SqlState::UNDEFINED_CURSOR,
                format!("portal \"{}\" does not exist", msg.portal_name),
            ))).await?;
            return Ok(());
        }
        Some(p) => p,
    };

    for stmt in portal.statement.statements.clone() {
        match session.executor.execute_statement(stmt) {
            Ok(r) => send_result(framed, r).await?,
            Err(e) => {
                framed.send(BackendMessage::ErrorResponse(e.to_pg_error())).await?;
                return Ok(());
            }
        }
    }
    Ok(())
}

async fn close_msg(
    framed: &mut Framed<TcpStream, PgCodec>,
    session: &mut Session,
    msg: CloseMessage,
) -> Result<()> {
    match msg.kind {
        DescribeKind::Statement => { session.executor.prepared.remove(&msg.name); }
        DescribeKind::Portal => { session.executor.portals.remove(&msg.name); }
    }
    framed.send(BackendMessage::CloseComplete).await?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Result serialization
// ─────────────────────────────────────────────────────────────────────────────

async fn send_result(
    framed: &mut Framed<TcpStream, PgCodec>,
    result: CommandResult,
) -> Result<()> {
    match result {
        CommandResult::Rows(rs) => {
            framed.send(BackendMessage::RowDescription(rs.columns.clone())).await?;
            for row in &rs.rows {
                let data: Vec<Option<Vec<u8>>> = row.iter()
                    .map(|v| if *v == PgValue::Null { None } else { Some(v.to_text().into_bytes()) })
                    .collect();
                framed.send(BackendMessage::DataRow(data)).await?;
            }
            framed.send(BackendMessage::CommandComplete(format!("SELECT {}", rs.rows.len()))).await?;
        }
        CommandResult::Modified { tag, .. } => {
            framed.send(BackendMessage::CommandComplete(tag)).await?;
        }
        CommandResult::Created(tag) | CommandResult::Dropped(tag) |
        CommandResult::Truncated(tag) | CommandResult::Altered(tag) |
        CommandResult::Transaction(tag) | CommandResult::Notify(tag) => {
            framed.send(BackendMessage::CommandComplete(tag)).await?;
        }
        CommandResult::Set => {
            framed.send(BackendMessage::CommandComplete("SET".to_string())).await?;
        }
        CommandResult::Show(value) => {
            let col = ColumnDesc::new("?column?", oid::TEXT);
            framed.send(BackendMessage::RowDescription(vec![col])).await?;
            framed.send(BackendMessage::DataRow(vec![Some(value.into_bytes())])).await?;
            framed.send(BackendMessage::CommandComplete("SHOW".to_string())).await?;
        }
        CommandResult::Explain(rs) => {
            framed.send(BackendMessage::RowDescription(rs.columns.clone())).await?;
            for row in &rs.rows {
                let data: Vec<Option<Vec<u8>>> = row.iter()
                    .map(|v| if *v == PgValue::Null { None } else { Some(v.to_text().into_bytes()) })
                    .collect();
                framed.send(BackendMessage::DataRow(data)).await?;
            }
            framed.send(BackendMessage::CommandComplete("EXPLAIN".to_string())).await?;
        }
        CommandResult::Copy { rows, .. } => {
            framed.send(BackendMessage::CopyOutResponse {
                overall_format: FormatCode::Text,
                column_formats: vec![],
            }).await?;
            framed.send(BackendMessage::CopyDone).await?;
            framed.send(BackendMessage::CommandComplete(format!("COPY {rows}"))).await?;
        }
        CommandResult::Do | CommandResult::Empty => {
            framed.send(BackendMessage::EmptyQueryResponse).await?;
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Startup message reader (raw, before codec takes over)
// ─────────────────────────────────────────────────────────────────────────────

async fn read_startup_message(stream: &mut TcpStream) -> Result<StartupMessage> {
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let total_len = u32::from_be_bytes(len_buf) as usize;
    if total_len < 8 || total_len > 10_000 {
        return Err(Error::Protocol(format!("invalid startup message length: {total_len}")));
    }
    let mut body = vec![0u8; total_len - 4];
    stream.read_exact(&mut body).await?;
    StartupMessage::parse(&body)
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn tx_status(session: &Session) -> TransactionStatus {
    use crate::executor::TxnState;
    match &session.executor.txn_state {
        TxnState::Idle => TransactionStatus::Idle,
        TxnState::InTransaction { .. } => TransactionStatus::InTransaction,
        TxnState::Failed { .. } => TransactionStatus::Failed,
    }
}
