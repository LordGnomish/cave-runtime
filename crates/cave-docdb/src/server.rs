//! MongoDB OP_MSG wire protocol server.

use crate::bson::Document;
use crate::commands;
use crate::cursor::CursorStore;
use crate::engine::Engine;
use crate::wire::{decode_op_msg, encode_op_msg};
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub async fn spawn_wire_server(
    port: u16,
    engine: Arc<Engine>,
    cursors: Arc<CursorStore>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let addr = format!("0.0.0.0:{}", port);
        match TcpListener::bind(&addr).await {
            Ok(listener) => {
                tracing::info!(target: "cave_docdb::wire", "wire server listening on {}", addr);
                loop {
                    match listener.accept().await {
                        Ok((socket, peer_addr)) => {
                            let engine = engine.clone();
                            let cursors = cursors.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(socket, engine, cursors).await {
                                    tracing::error!(target: "cave_docdb::wire", "connection error from {}: {}", peer_addr, e);
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!(target: "cave_docdb::wire", "accept error: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::error!(target: "cave_docdb::wire", "bind error on port {}: {}", port, e);
            }
        }
    })
}

async fn handle_connection(
    mut socket: TcpStream,
    engine: Arc<Engine>,
    cursors: Arc<CursorStore>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut buf = [0; 16384];

    loop {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            break; // Connection closed
        }

        match decode_op_msg(&buf[..n]) {
            Ok((request_id, op_msg)) => {
                tracing::info!(target: "cave_docdb::wire", "received OP_MSG request_id={}", request_id);

                let body = match op_msg.body() {
                    Some(b) => b.clone(),
                    None => {
                        let mut resp = Document::new();
                        resp.insert("ok".to_string(), Value::Number(0.into()));
                        resp.insert(
                            "errmsg".to_string(),
                            Value::String("no command in OP_MSG body".to_string()),
                        );
                        encode_and_send(&mut socket, resp, request_id, request_id).await?;
                        continue;
                    }
                };

                let cmd_name = body.keys().next().cloned().unwrap_or_default();
                tracing::info!(target: "cave_docdb::wire", "executing command: {}", cmd_name);

                let response = commands::dispatch(&cmd_name, &body, engine.clone(), cursors.clone())
                    .await
                    .unwrap_or_else(|e| {
                        let mut resp = Document::new();
                        resp.insert("ok".to_string(), Value::Number(0.into()));
                        resp.insert("errmsg".to_string(), Value::String(e));
                        resp
                    });

                encode_and_send(&mut socket, response, request_id, request_id).await?;
            }
            Err(e) => {
                tracing::error!(target: "cave_docdb::wire", "decode error: {}", e);
                let mut resp = Document::new();
                resp.insert("ok".to_string(), Value::Number(0.into()));
                resp.insert("errmsg".to_string(), Value::String(format!("decode error: {}", e)));
                encode_and_send(&mut socket, resp, 0, 0).await?;
            }
        }
    }

    Ok(())
}

async fn encode_and_send(
    socket: &mut TcpStream,
    response: Document,
    request_id: i32,
    response_to: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let op_msg = crate::wire::OpMsg::new(response);
    let encoded = encode_op_msg(&op_msg, request_id, response_to)?;
    socket.write_all(&encoded).await?;
    socket.flush().await?;
    Ok(())
}
