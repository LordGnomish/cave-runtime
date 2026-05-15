// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// UDP DNS server.
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::UdpSocket;
use tracing::{debug, warn};

use crate::{
    error::{DnsError, DnsResult},
    plugins::{Plugin, PluginChain, Protocol, QueryContext},
    protocol::message::{edns_payload_size, encode_message, make_error_response, parse_message, truncate_to_udp},
};
use hickory_proto::op::ResponseCode;

pub async fn serve(addr: String, plugins: Arc<PluginChain>) -> DnsResult<()> {
    let socket = Arc::new(
        UdpSocket::bind(&addr)
            .await
            .map_err(|e| DnsError::Io(e))?,
    );
    tracing::info!(addr = %addr, "UDP DNS server listening");

    let mut buf = vec![0u8; 4096];

    loop {
        let (n, client_addr) = match socket.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, "UDP recv error");
                continue;
            }
        };

        let pkt = buf[..n].to_vec();
        let sock = Arc::clone(&socket);
        let chain = Arc::clone(&plugins);

        tokio::spawn(async move {
            let response_bytes = handle_udp_packet(&pkt, client_addr, &chain).await;
            match response_bytes {
                Ok(bytes) => {
                    if let Err(e) = sock.send_to(&bytes, client_addr).await {
                        warn!(error = %e, client = %client_addr, "UDP send error");
                    }
                }
                Err(e) => {
                    warn!(error = %e, client = %client_addr, "UDP handler error");
                }
            }
        });
    }
}

async fn handle_udp_packet(
    buf: &[u8],
    client_addr: SocketAddr,
    plugins: &PluginChain,
) -> DnsResult<Vec<u8>> {
    let request = match parse_message(buf) {
        Ok(m) => m,
        Err(e) => {
            debug!(error = %e, "UDP: malformed DNS message");
            return Err(e);
        }
    };

    let udp_size = edns_payload_size(&request) as usize;
    let mut ctx = QueryContext::new(request, client_addr, Protocol::Udp);

    if let Err(e) = plugins.execute(&mut ctx).await {
        ctx.response = make_error_response(&ctx.request, ResponseCode::ServFail);
    }

    truncate_to_udp(&mut ctx.response, udp_size);
    encode_message(&ctx.response)
}
