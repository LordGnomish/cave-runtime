// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PostgreSQL wire protocol server.
//!
//! Framing — both startup and regular phase — runs through
//! [`crate::protocol::codec::PgWireCodec`], which delegates the
//! length-prefix mechanics to `cave_kernel::codec::length_prefix`.
//! That gives us partial-read backpressure (decoder returns `Ok(None)`
//! when the buffer is short, the read loop loops back to read more)
//! and a max-frame ceiling for free.

use crate::engine::Engine;
use crate::executor::delete::execute_delete;
use crate::executor::insert::execute_insert;
use crate::executor::select::execute_select;
use crate::executor::update::execute_update;
use crate::protocol::StartupMessage;
use crate::protocol::codec::{PgFrame, PgWireCodec, StartupKind, classify_startup};
use crate::protocol::messages::{BackendMessage, FieldDescription};
use crate::sql::ast::Statement;
use crate::sql::parser::Parser;
use crate::types::{SqlValue, oid};
use bytes::{Bytes, BytesMut};
use cave_kernel::codec::FrameCodec;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info};

/// Initial read buffer size. Grows on demand via `read_buf`.
const READ_BUFFER_INITIAL: usize = 8 * 1024;

pub struct Server {
    engine: Arc<Engine>,
    port: u16,
}

impl Server {
    pub fn new(engine: Arc<Engine>, port: u16) -> Self {
        Server { engine, port }
    }

    pub async fn listen(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

/// Read one full frame off the socket using `codec`. Drives `codec.decode`
/// in a loop, calling `read_buf` whenever the codec reports `Ok(None)`.
async fn read_frame(
    socket: &mut TcpStream,
    buf: &mut BytesMut,
    codec: &mut PgWireCodec,
) -> Result<Option<PgFrame>, Box<dyn std::error::Error + Send + Sync>> {
    loop {
        if let Some(frame) = codec.decode(buf)? {
            return Ok(Some(frame));
        }
        let n = socket.read_buf(buf).await?;
        if n == 0 {
            // Peer closed cleanly between frames if the buffer is empty.
            // Mid-frame EOF is a protocol error but we report it the same
            // way for now.
            return Ok(None);
        }
    }
}

/// PostgreSQL startup loop: an SSLRequest may precede the real
/// StartupMessage; loop until we've handled either a real startup or a
/// CancelRequest.
async fn handle_startup(
    socket: &mut TcpStream,
    buf: &mut BytesMut,
    codec: &mut PgWireCodec,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    loop {
        let frame = match read_frame(socket, buf, codec).await? {
            Some(f) => f,
            None => return Err("EOF during startup".into()),
        };
        let body = &frame.body[..];
        match classify_startup(body) {
            Some(StartupKind::SslRequest) => {
                socket.write_all(b"N").await?;
                continue;
            }
            Some(StartupKind::CancelRequest) => {
                debug!("client sent CancelRequest; closing");
                return Ok(false);
            }
            Some(StartupKind::Startup) => {
                let startup = StartupMessage::parse_from_bytes(body)?;
                debug!(
                    "startup: user={:?} db={:?}",
                    startup.user(),
                    startup.database()
                );
                send_startup_response(socket).await?;
                codec.advance_to_regular();
                return Ok(true);
            }
            None => return Err("startup body shorter than 4 bytes".into()),
        }
    }
}

async fn handle_client(
    mut socket: TcpStream,
    engine: Arc<Engine>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut buf = BytesMut::with_capacity(READ_BUFFER_INITIAL);
    let mut codec = PgWireCodec::new();

    if !handle_startup(&mut socket, &mut buf, &mut codec).await? {
        return Ok(());
    }

    while let Some(frame) = read_frame(&mut socket, &mut buf, &mut codec).await? {
        let type_byte = match frame.type_byte {
            Some(t) => t,
            None => {
                error!("regular-phase frame missing type byte");
                return Ok(());
            }
        };
        match type_byte {
            b'Q' => {
                let sql = String::from_utf8_lossy(&frame.body);
                let sql = sql.trim_end_matches('\0').trim();
                debug!("query: {}", sql);
                execute_and_respond(&mut socket, &engine, sql).await?;
            }
            b'P' => {
                socket
                    .write_all(&BackendMessage::ParseComplete.serialize()?)
                    .await?;
            }
            b'B' => {
                socket
                    .write_all(&BackendMessage::BindComplete.serialize()?)
                    .await?;
            }
            b'D' => {
                socket
                    .write_all(&BackendMessage::RowDescription { fields: vec![] }.serialize()?)
                    .await?;
            }
            b'E' => {
                socket
                    .write_all(
                        &BackendMessage::CommandComplete {
                            tag: "SELECT 0".to_string(),
                        }
                        .serialize()?,
                    )
                    .await?;
            }
            b'S' => {
                socket
                    .write_all(&BackendMessage::ReadyForQuery { status: 'I' }.serialize()?)
                    .await?;
            }
            b'X' => {
                debug!("client terminated");
                return Ok(());
            }
            t => {
                debug!("unhandled message type: 0x{:02x} ({})", t, t as char);
            }
        }
    }
    Ok(())
}

async fn execute_and_respond(
    socket: &mut TcpStream,
    engine: &Arc<Engine>,
    sql: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if sql.is_empty() {
        socket
            .write_all(&BackendMessage::EmptyQueryResponse.serialize()?)
            .await?;
        socket
            .write_all(&BackendMessage::ReadyForQuery { status: 'I' }.serialize()?)
            .await?;
        return Ok(());
    }

    let mut parser = Parser::new(sql);
    match parser.parse() {
        Err(e) => {
            send_error(socket, &e).await?;
        }
        Ok(ast) => match ast.statement {
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
                                    other => Some(Bytes::from(other.to_string().into_bytes())),
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
                    .write_all(
                        &BackendMessage::CommandComplete {
                            tag: "BEGIN".to_string(),
                        }
                        .serialize()?,
                    )
                    .await?
            }
            Statement::Commit => {
                socket
                    .write_all(
                        &BackendMessage::CommandComplete {
                            tag: "COMMIT".to_string(),
                        }
                        .serialize()?,
                    )
                    .await?
            }
            Statement::Rollback => {
                socket
                    .write_all(
                        &BackendMessage::CommandComplete {
                            tag: "ROLLBACK".to_string(),
                        }
                        .serialize()?,
                    )
                    .await?
            }
            _ => {
                socket
                    .write_all(
                        &BackendMessage::CommandComplete {
                            tag: "OK".to_string(),
                        }
                        .serialize()?,
                    )
                    .await?
            }
        },
    }

    socket
        .write_all(&BackendMessage::ReadyForQuery { status: 'I' }.serialize()?)
        .await?;
    Ok(())
}

async fn send_error(
    socket: &mut TcpStream,
    msg: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut fields = HashMap::new();
    fields.insert('S', "ERROR".to_string());
    fields.insert('C', "42601".to_string());
    fields.insert('M', msg.to_string());
    socket
        .write_all(&BackendMessage::ErrorResponse { fields }.serialize()?)
        .await?;
    Ok(())
}

async fn send_startup_response(
    socket: &mut TcpStream,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    socket
        .write_all(&BackendMessage::AuthenticationOk.serialize()?)
        .await?;
    socket
        .write_all(&BackendMessage::BackendKeyData { pid: 1, secret: 0 }.serialize()?)
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
    use crate::protocol::codec::{PgFrame, PgPhase, PgWireCodec};

    #[test]
    fn test_server_creation() {
        let engine = Arc::new(Engine::new());
        let server = Server::new(engine, 5432);
        assert_eq!(server.port, 5432);
    }

    #[test]
    fn test_startup_response_serializes() {
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

    /// Parity with the legacy hand-rolled handle_client read loop:
    /// confirm the codec yields the same `Q` frame given the same wire
    /// bytes.
    #[test]
    fn test_codec_matches_legacy_query_framing() {
        // Bytes that the old `handle_client` would have read with two
        // `read_exact` calls (1 type byte + 4 length + body).
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[b'Q']);
        buf.extend_from_slice(&13u32.to_be_bytes()); // 4 + 9
        buf.extend_from_slice(b"SELECT 1\0");

        let mut codec = PgWireCodec::new();
        codec.advance_to_regular();
        assert_eq!(codec.phase(), PgPhase::Regular);

        let frame = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(frame.type_byte, Some(b'Q'));
        assert_eq!(&frame.body[..], b"SELECT 1\0");
    }

    #[test]
    fn test_codec_streams_two_messages_in_one_buffer() {
        // Real-world: pipelined Q + Sync arriving in a single TCP read.
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&[b'Q']);
        buf.extend_from_slice(&5u32.to_be_bytes());
        buf.extend_from_slice(&[0u8]); // empty query body (just \0)
        buf.extend_from_slice(&[b'S']);
        buf.extend_from_slice(&4u32.to_be_bytes()); // Sync, no body

        let mut codec = PgWireCodec::new();
        codec.advance_to_regular();

        let f1 = codec.decode(&mut buf).unwrap().unwrap();
        let f2 = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(f1.type_byte, Some(b'Q'));
        assert_eq!(f2.type_byte, Some(b'S'));
        assert!(buf.is_empty());
    }

    #[test]
    fn test_pg_frame_constructable_from_static_bytes() {
        // Ensures the PgFrame type is plumbed through the public API.
        let frame = PgFrame {
            type_byte: Some(b'Z'),
            body: bytes::Bytes::from_static(&[b'I']),
        };
        assert_eq!(frame.type_byte, Some(b'Z'));
        assert_eq!(frame.body[0], b'I');
    }
}
