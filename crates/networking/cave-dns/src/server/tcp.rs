// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// TCP DNS server with 2-byte length-prefix framing.
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, warn};

use crate::{
    error::{DnsError, DnsResult},
    plugins::{PluginChain, Protocol, QueryContext},
    protocol::message::{encode_message, make_error_response, parse_message},
};
use hickory_proto::op::ResponseCode;

pub async fn serve(addr: String, plugins: Arc<PluginChain>) -> DnsResult<()> {
    let listener = TcpListener::bind(&addr).await.map_err(DnsError::Io)?;
    tracing::info!(addr = %addr, "TCP DNS server listening");

    loop {
        match listener.accept().await {
            Ok((stream, client_addr)) => {
                let chain = Arc::clone(&plugins);
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, client_addr, chain).await {
                        debug!(error = %e, client = %client_addr, "TCP connection error");
                    }
                });
            }
            Err(e) => {
                warn!(error = %e, "TCP accept error");
            }
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    client_addr: SocketAddr,
    plugins: Arc<PluginChain>,
) -> DnsResult<()> {
    loop {
        // Read 2-byte length prefix
        let mut len_buf = [0u8; 2];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(DnsError::Io(e)),
        }
        let msg_len = u16::from_be_bytes(len_buf) as usize;
        if msg_len == 0 {
            break;
        }

        let mut buf = vec![0u8; msg_len];
        stream.read_exact(&mut buf).await.map_err(DnsError::Io)?;

        let request = match parse_message(&buf) {
            Ok(m) => m,
            Err(e) => {
                debug!(error = %e, "TCP: malformed DNS message");
                break;
            }
        };

        let mut ctx = QueryContext::new(request, client_addr, Protocol::Tcp);
        if let Err(e) = plugins.execute(&mut ctx).await {
            ctx.response = make_error_response(&ctx.request, ResponseCode::ServFail);
        }

        let response_bytes = encode_message(&ctx.response)?;
        let len = response_bytes.len() as u16;
        stream
            .write_all(&len.to_be_bytes())
            .await
            .map_err(DnsError::Io)?;
        stream
            .write_all(&response_bytes)
            .await
            .map_err(DnsError::Io)?;
    }
    Ok(())
}
