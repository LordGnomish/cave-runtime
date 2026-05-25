// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// DNS-over-TLS server (port 853, RFC 7858).
use std::net::SocketAddr;
use std::sync::Arc;

use rustls::ServerConfig;
use rustls_pemfile::{certs, pkcs8_private_keys};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, warn};

use crate::{
    error::{DnsError, DnsResult},
    plugins::{PluginChain, Protocol, QueryContext},
    protocol::message::{encode_message, make_error_response, parse_message},
};
use hickory_proto::op::ResponseCode;

pub async fn serve(
    addr: String,
    plugins: Arc<PluginChain>,
    cert_path: String,
    key_path: String,
) -> DnsResult<()> {
    let tls_config = load_tls_config(&cert_path, &key_path)?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let listener = TcpListener::bind(&addr).await.map_err(DnsError::Io)?;
    tracing::info!(addr = %addr, "DoT server listening");

    loop {
        match listener.accept().await {
            Ok((stream, client_addr)) => {
                let acceptor = acceptor.clone();
                let chain = Arc::clone(&plugins);
                tokio::spawn(async move {
                    match acceptor.accept(stream).await {
                        Ok(tls_stream) => {
                            if let Err(e) =
                                handle_dot_connection(tls_stream, client_addr, chain).await
                            {
                                debug!(error = %e, client = %client_addr, "DoT connection error");
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, client = %client_addr, "DoT TLS handshake failed");
                        }
                    }
                });
            }
            Err(e) => {
                warn!(error = %e, "DoT accept error");
            }
        }
    }
}

async fn handle_dot_connection(
    mut stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    client_addr: SocketAddr,
    plugins: Arc<PluginChain>,
) -> DnsResult<()> {
    loop {
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
                debug!(error = %e, "DoT: malformed DNS message");
                break;
            }
        };

        let mut ctx = QueryContext::new(request, client_addr, Protocol::Dot);
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

fn load_tls_config(cert_path: &str, key_path: &str) -> DnsResult<ServerConfig> {
    let cert_file = std::fs::File::open(cert_path)
        .map_err(|e| DnsError::Tls(format!("open cert {cert_path}: {e}")))?;
    let key_file = std::fs::File::open(key_path)
        .map_err(|e| DnsError::Tls(format!("open key {key_path}: {e}")))?;

    let certs_der: Vec<_> = certs(&mut std::io::BufReader::new(cert_file))
        .filter_map(|r| r.ok())
        .collect();

    let keys: Vec<_> = pkcs8_private_keys(&mut std::io::BufReader::new(key_file))
        .filter_map(|r| r.ok())
        .collect();

    let key = keys
        .into_iter()
        .next()
        .ok_or_else(|| DnsError::Tls("no private key found".into()))?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            certs_der
                .into_iter()
                .map(rustls::pki_types::CertificateDer::from)
                .collect(),
            rustls::pki_types::PrivateKeyDer::Pkcs8(key.into()),
        )
        .map_err(|e| DnsError::Tls(e.to_string()))?;

    Ok(config)
}
