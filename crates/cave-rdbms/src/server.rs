//! PostgreSQL wire protocol server.

use crate::engine::Engine;
use crate::executor::delete::execute_delete;
use crate::executor::insert::execute_insert;
use crate::executor::select::execute_select;
use crate::executor::update::execute_update;
use crate::protocol::messages::{BackendMessage, FieldDescription};
use crate::protocol::StartupMessage;
use crate::sql::ast::Statement;
use crate::sql::parser::Parser;
use crate::types::{SqlValue, oid};
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info};

pub struct Server {
    engine: Arc<Engine>,
    port: u16,
}

impl Server {
    pub fn new(engine: Arc<Engine>, port: u16) -> Self {
        Server { engine, port }
    }

    pub async fn listen(&self) -> Result<(), Box<dyn std::error::Error>> {
        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr).await?;
        info!("RDBMS server listening on {}", addr);

        loop {
            let (socket, peer_addr) = listener.accept().await?;
            debug!("accepted connection from {}", peer_addr);
            let engine = Arc::clone(&self.engine);
            tokio::spawn(async move {
                if let Err(e) = handle_client(socket, engine).await {
                    error!("client error: {}", e);
                }
            });
        }
    }
}

/// PostgreSQL startup messages lack a type byte; they start with 4-byte total length.
async fn handle_startup(socket: &mut TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let mut len_buf = [0u8; 4];
        socket.read_exact(&mut len_buf).await?;
        let total_len = u32::from_be_bytes(len_buf) as usize;
        if total_len < 4 {
            return Err("startup message too short".into());
        }
        let mut body = vec![0u8; total_len - 4];
        socket.read_exact(&mut body).await?;

        if body.len() >= 4 {
            let code = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
            if code == 80877103 {
                // SSLRequest — decline gracefully
                socket.write_all(b"N").await?;
                continue;
            }
        }

        // StartupMessage
        let startup = StartupMessage::parse_from_bytes(&body)?;
        debug!("startup: user={:?} db={:?}", startup.user(), startup.database());
        send_startup_response(socket).await?;
        return Ok(());
    }
}

async fn handle_client(mut socket: TcpStream, engine: Arc<Engine>) -> Result<(), Box<dyn std::error::Error>> {
    handle_startup(&mut socket).await?;

    // Regular message loop: [1-byte type][4-byte length incl. self][payload]
    loop {
        let mut type_buf = [0u8; 1];
        match socket.read_exact(&mut type_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        }

        let mut len_buf = [0u8; 4];
        socket.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        let body_len = len.saturating_sub(4);
        let mut body = vec![0u8; body_len];
        if body_len > 0 {
            socket.read_exact(&mut body).await?;
        }

        match type_buf[0] {
            b'Q' => {
                let sql = String::from_utf8_lossy(&body);
                let sql = sql.trim_end_matches('\0').trim();
                debug!("query: {}", sql);
                execute_and_respond(&mut socket, &engine, sql).await?;
            }
            // Extended query: Parse
            b'P' => {
                let msg = BackendMessage::ParseComplete;
                socket.write_all(&msg.serialize()?).await?;
            }
            // Extended query: Bind
            b'B' => {
                let msg = BackendMessage::BindComplete;
                socket.write_all(&msg.serialize()?).await?;
            }
            // Extended query: Describe — send empty RowDescription
            b'D' => {
                let msg = BackendMessage::RowDescription { fields: vec![] };
                socket.write_all(&msg.serialize()?).await?;
            }
            // Extended query: Execute — send empty result
            b'E' => {
                let msg = BackendMessage::CommandComplete { tag: "SELECT 0".to_string() };
                socket.write_all(&msg.serialize()?).await?;
            }
            // Sync — send ReadyForQuery
            b'S' => {
                let msg = BackendMessage::ReadyForQuery { status: 'I' };
                socket.write_all(&msg.serialize()?).await?;
            }
            // Terminate
            b'X' => {
                debug!("client terminated");
                return Ok(());
            }
            t => {
                debug!("unhandled message type: 0x{:02x} ({})", t, t as char);
            }
        }
    }
}

async fn execute_and_respond(
    socket: &mut TcpStream,
    engine: &Arc<Engine>,
    sql: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if sql.is_empty() {
        socket.write_all(&BackendMessage::EmptyQueryResponse.serialize()?).await?;
        socket.write_all(&BackendMessage::ReadyForQuery { status: 'I' }.serialize()?).await?;
        return Ok(());
    }

    let mut parser = Parser::new(sql);
    match parser.parse() {
        Err(e) => {
            send_error(socket, &e).await?;
        }
        Ok(ast) => {
            match ast.statement {
                Statement::Select(ref select) => {
                    let db = engine.get_database().await;
                    match execute_select(select, &db) {
                        Err(e) => send_error(socket, &e).await?,
                        Ok(result) => {
                            let fields: Vec<FieldDescription> = result
                                .columns
                                .iter()
                                .map(|c| FieldDescription {
                                    name: c.clone(),
                                    table_oid: 0,
                                    column_attr_num: 0,
                                    type_oid: oid::TEXT,
                                    type_len: -1,
                                    type_mod: -1,
                                    format: 0,
                                })
                                .collect();
                            socket
                                .write_all(&BackendMessage::RowDescription { fields }.serialize()?)
                                .await?;
                            let row_count = result.rows.len();
                            for row in &result.rows {
                                let values: Vec<Option<Bytes>> = row
                                    .iter()
                                    .map(|v| match v {
                                        SqlValue::Null => None,
                                        other => {
                                            Some(Bytes::from(other.to_string().into_bytes()))
                                        }
                                    })
                                    .collect();
                                socket
                                    .write_all(&BackendMessage::DataRow { values }.serialize()?)
                                    .await?;
                            }
                            socket
                                .write_all(
                                    &BackendMessage::CommandComplete {
                                        tag: format!("SELECT {}", row_count),
                                    }
                                    .serialize()?,
                                )
                                .await?;
                        }
                    }
                }
                Statement::Insert(ref insert) => {
                    let mut db = engine.storage.write().await;
                    match execute_insert(insert, &mut db) {
                        Err(e) => send_error(socket, &e).await?,
                        Ok(n) => {
                            socket
                                .write_all(
                                    &BackendMessage::CommandComplete {
                                        tag: format!("INSERT 0 {}", n),
                                    }
                                    .serialize()?,
                                )
                                .await?
                        }
                    }
                }
                Statement::Update(ref update) => {
                    let mut db = engine.storage.write().await;
                    match execute_update(update, &mut db) {
                        Err(e) => send_error(socket, &e).await?,
                        Ok(n) => {
                            socket
                                .write_all(
                                    &BackendMessage::CommandComplete {
                                        tag: format!("UPDATE {}", n),
                                    }
                                    .serialize()?,
                                )
                                .await?
                        }
                    }
                }
                Statement::Delete(ref delete) => {
                    let mut db = engine.storage.write().await;
                    match execute_delete(delete, &mut db) {
                        Err(e) => send_error(socket, &e).await?,
                        Ok(n) => {
                            socket
                                .write_all(
                                    &BackendMessage::CommandComplete {
                                        tag: format!("DELETE {}", n),
                                    }
                                    .serialize()?,
                                )
                                .await?
                        }
                    }
                }
                Statement::Begin => {
                    socket
                        .write_all(&BackendMessage::CommandComplete { tag: "BEGIN".to_string() }.serialize()?)
                        .await?
                }
                Statement::Commit => {
                    socket
                        .write_all(&BackendMessage::CommandComplete { tag: "COMMIT".to_string() }.serialize()?)
                        .await?
                }
                Statement::Rollback => {
                    socket
                        .write_all(
                            &BackendMessage::CommandComplete { tag: "ROLLBACK".to_string() }.serialize()?,
                        )
                        .await?
                }
                _ => {
                    socket
                        .write_all(&BackendMessage::CommandComplete { tag: "OK".to_string() }.serialize()?)
                        .await?
                }
            }
        }
    }

    socket
        .write_all(&BackendMessage::ReadyForQuery { status: 'I' }.serialize()?)
        .await?;
    Ok(())
}

async fn send_error(socket: &mut TcpStream, msg: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut fields = HashMap::new();
    fields.insert('S', "ERROR".to_string());
    fields.insert('C', "42601".to_string()); // syntax_error
    fields.insert('M', msg.to_string());
    socket
        .write_all(&BackendMessage::ErrorResponse { fields }.serialize()?)
        .await?;
    Ok(())
}

async fn send_startup_response(socket: &mut TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    socket
        .write_all(&BackendMessage::AuthenticationOk.serialize()?)
        .await?;
    socket
        .write_all(
            &BackendMessage::BackendKeyData { pid: 1, secret: 0 }.serialize()?,
        )
        .await?;

    for (k, v) in &[
        ("server_version", "14.0"),
        ("client_encoding", "UTF8"),
        ("server_encoding", "UTF8"),
        ("DateStyle", "ISO, MDY"),
        ("TimeZone", "UTC"),
    ] {
        socket
            .write_all(
                &BackendMessage::ParameterStatus {
                    name: k.to_string(),
                    value: v.to_string(),
                }
                .serialize()?,
            )
            .await?;
    }

    socket
        .write_all(&BackendMessage::ReadyForQuery { status: 'I' }.serialize()?)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_creation() {
        let engine = Arc::new(Engine::new());
        let server = Server::new(engine, 5432);
        assert_eq!(server.port, 5432);
    }

    #[test]
    fn test_startup_response_serializes() {
        // Verify all startup backend messages serialize without error
        let msgs: Vec<BackendMessage> = vec![
            BackendMessage::AuthenticationOk,
            BackendMessage::BackendKeyData { pid: 1, secret: 0 },
            BackendMessage::ParameterStatus {
                name: "server_version".to_string(),
                value: "14.0".to_string(),
            },
            BackendMessage::ReadyForQuery { status: 'I' },
        ];
        for msg in msgs {
            msg.serialize().expect("startup message must serialize");
        }
    }

    #[test]
    fn test_error_response_fields() {
        let mut fields = HashMap::new();
        fields.insert('S', "ERROR".to_string());
        fields.insert('C', "42601".to_string());
        fields.insert('M', "syntax error".to_string());
        let msg = BackendMessage::ErrorResponse { fields };
        let bytes = msg.serialize().unwrap();
        assert_eq!(bytes[0], b'E');
    }
}
