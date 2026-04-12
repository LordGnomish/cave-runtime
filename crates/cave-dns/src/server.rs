use crate::error::DnsResult;
use crate::message::{decode, encode};
use crate::resolver::Resolver;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};

pub struct DnsServer {
    pub resolver: Arc<Resolver>,
    pub bind_addr: String,
    pub port: u16,
}

impl DnsServer {
    pub fn new(resolver: Arc<Resolver>, bind_addr: &str, port: u16) -> Self {
        DnsServer {
            resolver,
            bind_addr: bind_addr.to_string(),
            port,
        }
    }

    /// Handle a raw DNS message (bytes) and return the response bytes.
    pub fn handle_message(&self, data: &[u8]) -> Vec<u8> {
        match decode(data) {
            Ok(msg) => {
                let response = self.resolver.resolve(&msg);
                encode(&response).unwrap_or_default()
            }
            Err(_) => {
                // Return a SERVFAIL if we can't parse the message
                // Build minimal error response
                if data.len() >= 2 {
                    let id = u16::from_be_bytes([data[0], data[1]]);
                    // flags: QR=1, RCODE=2 (SERVFAIL)
                    let resp = vec![
                        (id >> 8) as u8,
                        (id & 0xFF) as u8,
                        0x80,
                        0x02,
                        0x00,
                        0x00,
                        0x00,
                        0x00,
                        0x00,
                        0x00,
                        0x00,
                        0x00,
                    ];
                    resp
                } else {
                    vec![]
                }
            }
        }
    }

    /// Start UDP DNS server.
    pub async fn serve_udp(&self) -> DnsResult<()> {
        let addr = format!("{}:{}", self.bind_addr, self.port);
        let socket = UdpSocket::bind(&addr)
            .await
            .map_err(|e| crate::error::DnsError::Io(e.to_string()))?;
        tracing::info!("DNS UDP listening on {}", addr);

        let mut buf = vec![0u8; 512];
        loop {
            match socket.recv_from(&mut buf).await {
                Ok((len, peer)) => {
                    let data = buf[..len].to_vec();
                    let response = self.handle_message(&data);
                    if !response.is_empty() {
                        let _ = socket.send_to(&response, peer).await;
                    }
                }
                Err(e) => {
                    tracing::error!("UDP recv error: {}", e);
                }
            }
        }
    }

    /// Start TCP DNS server.
    pub async fn serve_tcp(&self) -> DnsResult<()> {
        let addr = format!("{}:{}", self.bind_addr, self.port);
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| crate::error::DnsError::Io(e.to_string()))?;
        tracing::info!("DNS TCP listening on {}", addr);

        loop {
            match listener.accept().await {
                Ok((mut stream, _peer)) => {
                    let resolver = self.resolver.clone();
                    tokio::spawn(async move {
                        // TCP DNS: 2-byte length prefix
                        let mut len_buf = [0u8; 2];
                        if stream.read_exact(&mut len_buf).await.is_err() {
                            return;
                        }
                        let msg_len = u16::from_be_bytes(len_buf) as usize;
                        let mut msg_buf = vec![0u8; msg_len];
                        if stream.read_exact(&mut msg_buf).await.is_err() {
                            return;
                        }

                        let response = match decode(&msg_buf) {
                            Ok(msg) => {
                                let resp = resolver.resolve(&msg);
                                encode(&resp).unwrap_or_default()
                            }
                            Err(_) => vec![],
                        };

                        if !response.is_empty() {
                            let resp_len = (response.len() as u16).to_be_bytes();
                            let _ = stream.write_all(&resp_len).await;
                            let _ = stream.write_all(&response).await;
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("TCP accept error: {}", e);
                }
            }
        }
    }

    /// Run both UDP and TCP servers concurrently.
    pub async fn run(&self) -> DnsResult<()> {
        let addr = format!("{}:{}", self.bind_addr, self.port);
        let udp_socket = UdpSocket::bind(&addr)
            .await
            .map_err(|e| crate::error::DnsError::Io(e.to_string()))?;
        let tcp_listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| crate::error::DnsError::Io(e.to_string()))?;

        tracing::info!("DNS server running on {} (UDP+TCP)", addr);

        let resolver_udp = self.resolver.clone();
        let resolver_tcp = self.resolver.clone();

        let udp_task = tokio::spawn(async move {
            let mut buf = vec![0u8; 512];
            loop {
                match udp_socket.recv_from(&mut buf).await {
                    Ok((len, peer)) => {
                        let data = buf[..len].to_vec();
                        let response = match decode(&data) {
                            Ok(msg) => {
                                let resp = resolver_udp.resolve(&msg);
                                encode(&resp).unwrap_or_default()
                            }
                            Err(_) => vec![],
                        };
                        if !response.is_empty() {
                            let _ = udp_socket.send_to(&response, peer).await;
                        }
                    }
                    Err(e) => {
                        tracing::error!("UDP error: {}", e);
                    }
                }
            }
        });

        let tcp_task = tokio::spawn(async move {
            loop {
                match tcp_listener.accept().await {
                    Ok((mut stream, _)) => {
                        let resolver = resolver_tcp.clone();
                        tokio::spawn(async move {
                            let mut len_buf = [0u8; 2];
                            if stream.read_exact(&mut len_buf).await.is_err() {
                                return;
                            }
                            let msg_len = u16::from_be_bytes(len_buf) as usize;
                            let mut msg_buf = vec![0u8; msg_len];
                            if stream.read_exact(&mut msg_buf).await.is_err() {
                                return;
                            }
                            let response = match decode(&msg_buf) {
                                Ok(msg) => {
                                    let resp = resolver.resolve(&msg);
                                    encode(&resp).unwrap_or_default()
                                }
                                Err(_) => vec![],
                            };
                            if !response.is_empty() {
                                let resp_len = (response.len() as u16).to_be_bytes();
                                let _ = stream.write_all(&resp_len).await;
                                let _ = stream.write_all(&response).await;
                            }
                        });
                    }
                    Err(e) => {
                        tracing::error!("TCP error: {}", e);
                    }
                }
            }
        });

        let _ = tokio::join!(udp_task, tcp_task);
        Ok(())
    }
}
