// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::message::{decode, encode};
use crate::types::DnsMessage;
use std::time::Duration;
use tokio::net::UdpSocket;

pub struct Forwarder {
    pub upstream_servers: Vec<String>,
    pub timeout_ms: u64,
}

impl Forwarder {
    pub fn new(servers: Vec<String>) -> Self {
        Forwarder {
            upstream_servers: servers,
            timeout_ms: 2000,
        }
    }

    /// Forward a DNS query to an upstream server, trying each in order.
    pub async fn forward(&self, msg: &DnsMessage) -> Option<DnsMessage> {
        for server in &self.upstream_servers {
            if let Some(response) = self.query_server(server, msg).await {
                return Some(response);
            }
        }
        None
    }

    async fn query_server(&self, server: &str, msg: &DnsMessage) -> Option<DnsMessage> {
        let wire = encode(msg).ok()?;

        let socket = UdpSocket::bind("0.0.0.0:0").await.ok()?;
        socket.connect(server).await.ok()?;

        let send_result = tokio::time::timeout(
            Duration::from_millis(self.timeout_ms),
            socket.send(&wire),
        )
        .await;

        if send_result.is_err() {
            return None;
        }
        if send_result.unwrap().is_err() {
            return None;
        }

        let mut buf = vec![0u8; 4096];
        let recv_result = tokio::time::timeout(
            Duration::from_millis(self.timeout_ms),
            socket.recv(&mut buf),
        )
        .await;

        match recv_result {
            Ok(Ok(len)) => decode(&buf[..len]).ok(),
            _ => None,
        }
    }
}
