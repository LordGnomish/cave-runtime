//! PostgreSQL wire protocol server.

use crate::engine::Engine;
use crate::protocol::messages::{BackendMessage, FieldDescription, FrontendMessage};
use crate::protocol::{ErrorResponse, StartupMessage};
use bytes::{Bytes, BytesMut};
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

async fn handle_client(mut socket: TcpStream, _engine: Arc<Engine>) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let mut len_bytes = [0u8; 4];
        if socket.read_exact(&mut len_bytes).await? == 0 {
            return Ok(());
        }

        let len = u32::from_be_bytes(len_bytes) as usize;
        if len < 4 {
            return Err("invalid message length".into());
        }

        let mut msg = vec![0u8; len - 4];
        socket.read_exact(&mut msg).await?;

        let msg_type = msg[0];
        let body = &msg[1..];

        match msg_type {
            // StartupMessage has special format (no message type byte)
            0 => {
                let _startup = StartupMessage::parse_from_bytes(&len_bytes[..])?;
                send_startup_response(&mut socket).await?;
            }
            // SSLRequest
            b'S' if len == 8 => {
                socket.write_all(b"N").await?; // No TLS
            }
            // Query
            b'Q' => {
                let query = String::from_utf8(body.to_vec())?;
                let query = query.trim_end_matches('\0');
                debug!("executing query: {}", query);

                let rows = vec![];
                send_query_response(&mut socket, &rows).await?;
            }
            // Terminate
            b'X' => {
                debug!("client terminated");
                return Ok(());
            }
            _ => {
                debug!("unknown message type: {}", msg_type as char);
            }
        }
    }
}

async fn send_startup_response(socket: &mut TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    // AuthenticationOk
    let msg = BackendMessage::AuthenticationOk;
    socket.write_all(&msg.serialize()?).await?;

    // BackendKeyData
    let msg = BackendMessage::BackendKeyData {
        pid: 1,
        secret: 0,
    };
    socket.write_all(&msg.serialize()?).await?;

    // ParameterStatus
    let params = vec![
        ("server_version", "14.0"),
        ("client_encoding", "UTF8"),
        ("server_encoding", "UTF8"),
        ("DateStyle", "ISO, MDY"),
        ("TimeZone", "UTC"),
    ];
    for (k, v) in params {
        let msg = BackendMessage::ParameterStatus {
            name: k.to_string(),
            value: v.to_string(),
        };
        socket.write_all(&msg.serialize()?).await?;
    }

    // ReadyForQuery
    let msg = BackendMessage::ReadyForQuery { status: 'I' };
    socket.write_all(&msg.serialize()?).await?;

    Ok(())
}

async fn send_query_response(
    socket: &mut TcpStream,
    _rows: &[Vec<String>],
) -> Result<(), Box<dyn std::error::Error>> {
    // RowDescription with stub columns
    let fields = vec![FieldDescription {
        name: "col".to_string(),
        table_oid: 0,
        column_attr_num: 1,
        type_oid: 25, // text
        type_len: -1,
        type_mod: -1,
        format: 0,
    }];
    let msg = BackendMessage::RowDescription { fields };
    socket.write_all(&msg.serialize()?).await?;

    // CommandComplete
    let msg = BackendMessage::CommandComplete {
        tag: "SELECT 0".to_string(),
    };
    socket.write_all(&msg.serialize()?).await?;

    // ReadyForQuery
    let msg = BackendMessage::ReadyForQuery { status: 'I' };
    socket.write_all(&msg.serialize()?).await?;

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
}
