//! Listener + filter chain — Envoy `Listener` parity (config layer).
//!
//! This module owns the *configuration* model for the data-plane edge:
//! address binding, codec / protocol stack, ALPN negotiation, TLS termination
//! parameters, filter-chain match scoring, and connection-pool sizing.
//!
//! The wire-level transport is mounted in `proxy.rs` (HTTP/1.1 + HTTP/2 today)
//! and gated behind the [`ProtocolStack`] selected here. HTTP/3 + QUIC live in
//! the same shape so the listener config can describe a fully-featured
//! Envoy-equivalent listener even if the transport is not yet linked.
//!
//! Upstream parity: `envoy.config.listener.v3.Listener` +
//! `envoy.config.listener.v3.FilterChain` +
//! `envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use uuid::Uuid;

// ── Address binding ──────────────────────────────────────────────────────────

/// Listener bind address. Either a TCP socket or a Unix domain socket.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BindAddress {
    Socket {
        ip: IpAddr,
        port: u16,
    },
    Unix {
        path: String,
        mode: Option<u32>,
    },
}

impl BindAddress {
    pub fn socket_v4(ip: Ipv4Addr, port: u16) -> Self {
        BindAddress::Socket { ip: IpAddr::V4(ip), port }
    }
    pub fn socket_v6(ip: Ipv6Addr, port: u16) -> Self {
        BindAddress::Socket { ip: IpAddr::V6(ip), port }
    }
    pub fn any_v4(port: u16) -> Self {
        Self::socket_v4(Ipv4Addr::UNSPECIFIED, port)
    }
    pub fn any_v6(port: u16) -> Self {
        Self::socket_v6(Ipv6Addr::UNSPECIFIED, port)
    }
    pub fn unix(path: impl Into<String>) -> Self {
        BindAddress::Unix { path: path.into(), mode: None }
    }

    pub fn as_socket(&self) -> Option<SocketAddr> {
        match self {
            BindAddress::Socket { ip, port } => Some(SocketAddr::new(*ip, *port)),
            BindAddress::Unix { .. } => None,
        }
    }

    pub fn is_ipv4(&self) -> bool {
        matches!(self, BindAddress::Socket { ip: IpAddr::V4(_), .. })
    }

    pub fn is_ipv6(&self) -> bool {
        matches!(self, BindAddress::Socket { ip: IpAddr::V6(_), .. })
    }
}

impl FromStr for BindAddress {
    type Err = ListenerError;

    /// Parse a bind address. Forms:
    /// - `0.0.0.0:8080`, `127.0.0.1:443`, `[::]:8443`
    /// - `unix:/var/run/cave.sock`
    /// - `:8080` (defaults to 0.0.0.0)
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(path) = s.strip_prefix("unix:") {
            if path.is_empty() {
                return Err(ListenerError::InvalidAddress("empty unix path".into()));
            }
            return Ok(BindAddress::unix(path));
        }
        let s = if let Some(port_only) = s.strip_prefix(':') {
            // ":8080" → "0.0.0.0:8080"
            return Ok(BindAddress::any_v4(
                port_only.parse().map_err(|_| ListenerError::InvalidAddress(s.into()))?,
            ));
        } else {
            s
        };
        let socket: SocketAddr = s
            .parse()
            .map_err(|_| ListenerError::InvalidAddress(s.into()))?;
        Ok(BindAddress::Socket { ip: socket.ip(), port: socket.port() })
    }
}

// ── Protocol stack & codec ───────────────────────────────────────────────────

/// Wire protocol selectable on a filter chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WireProtocol {
    /// HTTP/1.0 + HTTP/1.1.
    Http1,
    /// HTTP/2 over TCP (h2 with TLS, h2c without TLS).
    Http2,
    /// HTTP/3 over QUIC.
    Http3,
    /// Raw TCP — no L7 parsing, terminate-and-forward.
    Tcp,
    /// Raw UDP — datagram pass-through.
    Udp,
}

impl WireProtocol {
    /// ALPN identifier as transmitted on the wire (Envoy parity).
    pub fn alpn(&self) -> Option<&'static str> {
        match self {
            WireProtocol::Http1 => Some("http/1.1"),
            WireProtocol::Http2 => Some("h2"),
            WireProtocol::Http3 => Some("h3"),
            WireProtocol::Tcp | WireProtocol::Udp => None,
        }
    }

    /// True if this protocol runs natively over QUIC (UDP).
    pub fn is_quic(&self) -> bool {
        matches!(self, WireProtocol::Http3)
    }

    /// True if this protocol uses TLS by default for HTTP semantics.
    pub fn defaults_to_tls(&self) -> bool {
        matches!(self, WireProtocol::Http2 | WireProtocol::Http3)
    }
}

/// Codec selection mode — Envoy `HttpConnectionManager.codec_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CodecType {
    /// Auto-detect from ALPN / first byte sniffing.
    Auto,
    Http1,
    Http2,
    Http3,
}

impl CodecType {
    pub fn pick(self, alpn: Option<&str>) -> WireProtocol {
        match self {
            CodecType::Http1 => WireProtocol::Http1,
            CodecType::Http2 => WireProtocol::Http2,
            CodecType::Http3 => WireProtocol::Http3,
            CodecType::Auto => match alpn {
                Some("h2") => WireProtocol::Http2,
                Some("h3") => WireProtocol::Http3,
                _ => WireProtocol::Http1,
            },
        }
    }
}

/// Set of protocols offered by the server during ALPN; ordered by preference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlpnList(pub Vec<WireProtocol>);

impl AlpnList {
    pub fn http2_first() -> Self {
        AlpnList(vec![WireProtocol::Http2, WireProtocol::Http1])
    }
    pub fn http1_only() -> Self {
        AlpnList(vec![WireProtocol::Http1])
    }
    pub fn http3_offload() -> Self {
        AlpnList(vec![WireProtocol::Http3, WireProtocol::Http2, WireProtocol::Http1])
    }

    /// Pick the highest-preference protocol both sides support.
    pub fn negotiate(&self, client: &[&str]) -> Option<WireProtocol> {
        for p in &self.0 {
            if let Some(alpn) = p.alpn() {
                if client.iter().any(|c| *c == alpn) {
                    return Some(*p);
                }
            }
        }
        None
    }

    pub fn as_wire(&self) -> Vec<&'static str> {
        self.0.iter().filter_map(|p| p.alpn()).collect()
    }

    pub fn contains(&self, proto: WireProtocol) -> bool {
        self.0.contains(&proto)
    }
}

// ── TLS termination context (config layer) ───────────────────────────────────

/// Client certificate verification policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientAuth {
    None,
    Optional,
    OptionalNoCa,
    Required,
}

/// Minimum TLS version the listener accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TlsVersion {
    Tls12,
    Tls13,
}

impl TlsVersion {
    pub fn ietf(self) -> u16 {
        match self {
            TlsVersion::Tls12 => 0x0303,
            TlsVersion::Tls13 => 0x0304,
        }
    }
}

/// TLS termination parameters for a filter chain — pure config; the actual
/// rustls `ServerConfig` is built in `tls.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsTermination {
    pub min_version: TlsVersion,
    pub max_version: TlsVersion,
    pub alpn: AlpnList,
    pub client_auth: ClientAuth,
    /// SNI hostnames this filter chain serves.
    #[serde(default)]
    pub sni_hostnames: Vec<String>,
    /// Optional CA bundle ID for client cert validation.
    #[serde(default)]
    pub trust_bundle_id: Option<Uuid>,
    /// Cert ID(s) used to satisfy SNI matches.
    #[serde(default)]
    pub certificate_ids: Vec<Uuid>,
    /// Hybrid PQC handshake group preference (see `tls.rs`).
    #[serde(default)]
    pub pqc_hybrid: bool,
}

impl Default for TlsTermination {
    fn default() -> Self {
        Self {
            min_version: TlsVersion::Tls12,
            max_version: TlsVersion::Tls13,
            alpn: AlpnList::http2_first(),
            client_auth: ClientAuth::None,
            sni_hostnames: vec![],
            trust_bundle_id: None,
            certificate_ids: vec![],
            pqc_hybrid: false,
        }
    }
}

impl TlsTermination {
    pub fn validate(&self) -> Result<(), ListenerError> {
        if self.min_version > self.max_version {
            return Err(ListenerError::TlsConfig(
                "min_version > max_version".into(),
            ));
        }
        if matches!(self.client_auth, ClientAuth::Required | ClientAuth::Optional)
            && self.trust_bundle_id.is_none()
        {
            return Err(ListenerError::TlsConfig(
                "client cert verification requires trust_bundle_id".into(),
            ));
        }
        if self.alpn.0.is_empty() {
            return Err(ListenerError::TlsConfig("alpn list is empty".into()));
        }
        Ok(())
    }
}

// ── HTTP connection manager config ───────────────────────────────────────────

/// HTTP-level normalization knobs. Envoy `HttpConnectionManager` parity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpConnectionManager {
    pub codec: CodecType,
    pub stat_prefix: String,
    pub use_remote_address: bool,
    pub xff_num_trusted_hops: u32,
    pub generate_request_id: bool,
    pub preserve_external_request_id: bool,
    pub normalize_path: bool,
    pub merge_slashes: bool,
    pub strip_trailing_host_dot: bool,
    pub stream_idle_timeout: Duration,
    pub request_timeout: Duration,
    pub drain_timeout: Duration,
    pub server_header_transformation: ServerHeader,
    pub max_request_headers_kb: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ServerHeader {
    Overwrite,
    AppendIfAbsent,
    PassThrough,
}

impl Default for HttpConnectionManager {
    fn default() -> Self {
        Self {
            codec: CodecType::Auto,
            stat_prefix: "ingress_http".into(),
            use_remote_address: true,
            xff_num_trusted_hops: 0,
            generate_request_id: true,
            preserve_external_request_id: false,
            normalize_path: true,
            merge_slashes: false,
            strip_trailing_host_dot: false,
            stream_idle_timeout: Duration::from_secs(300),
            request_timeout: Duration::from_secs(0), // disabled
            drain_timeout: Duration::from_secs(5),
            server_header_transformation: ServerHeader::Overwrite,
            max_request_headers_kb: 60,
        }
    }
}

impl HttpConnectionManager {
    pub fn validate(&self) -> Result<(), ListenerError> {
        if self.stat_prefix.is_empty() {
            return Err(ListenerError::HcmConfig("stat_prefix is empty".into()));
        }
        if self.max_request_headers_kb == 0 || self.max_request_headers_kb > 8192 {
            return Err(ListenerError::HcmConfig(
                "max_request_headers_kb out of range (1..=8192)".into(),
            ));
        }
        Ok(())
    }
}

// ── HTTP/3 / QUIC config ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuicConfig {
    pub max_concurrent_streams: u32,
    pub idle_timeout: Duration,
    pub crypto_handshake_timeout: Duration,
    pub initial_stream_window_size: u32,
    pub initial_connection_window_size: u32,
    pub send_disable_active_migration: bool,
    /// Optional preferred-address advertisement (server-preferred-address frame).
    pub preferred_address: Option<SocketAddr>,
}

impl Default for QuicConfig {
    fn default() -> Self {
        Self {
            max_concurrent_streams: 100,
            idle_timeout: Duration::from_secs(60),
            crypto_handshake_timeout: Duration::from_secs(20),
            initial_stream_window_size: 16 * 1024 * 1024,
            initial_connection_window_size: 25 * 1024 * 1024,
            send_disable_active_migration: false,
            preferred_address: None,
        }
    }
}

impl QuicConfig {
    pub fn validate(&self) -> Result<(), ListenerError> {
        if self.max_concurrent_streams == 0 {
            return Err(ListenerError::QuicConfig(
                "max_concurrent_streams must be > 0".into(),
            ));
        }
        if self.idle_timeout.as_secs() < 1 {
            return Err(ListenerError::QuicConfig(
                "idle_timeout must be >= 1s".into(),
            ));
        }
        Ok(())
    }
}

// ── Connection pool sizing ───────────────────────────────────────────────────

/// Per-listener inbound connection limits (Envoy `Listener.connection_balance_config`
/// + `circuit_breakers` for downstream side).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionPool {
    pub max_connections: u32,
    pub max_pending_requests: u32,
    pub max_requests: u32,
    pub max_requests_per_connection: u32,
    pub idle_timeout: Duration,
    pub keepalive: KeepAlive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeepAlive {
    pub probes: u32,
    pub interval: Duration,
    pub idle_time: Duration,
}

impl Default for KeepAlive {
    fn default() -> Self {
        Self {
            probes: 5,
            interval: Duration::from_secs(15),
            idle_time: Duration::from_secs(60),
        }
    }
}

impl Default for ConnectionPool {
    fn default() -> Self {
        Self {
            max_connections: 1024,
            max_pending_requests: 1024,
            max_requests: 1024,
            max_requests_per_connection: 0, // 0 = unlimited
            idle_timeout: Duration::from_secs(3600),
            keepalive: KeepAlive::default(),
        }
    }
}

impl ConnectionPool {
    pub fn validate(&self) -> Result<(), ListenerError> {
        if self.max_connections == 0 {
            return Err(ListenerError::PoolConfig(
                "max_connections must be > 0".into(),
            ));
        }
        if self.max_pending_requests == 0 {
            return Err(ListenerError::PoolConfig(
                "max_pending_requests must be > 0".into(),
            ));
        }
        Ok(())
    }
}

// ── Filter chain ─────────────────────────────────────────────────────────────

/// Envoy `FilterChainMatch` — predicate against an inbound connection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterChainMatch {
    pub server_names: Vec<String>,
    pub transport_protocol: Option<TransportProtocol>,
    pub application_protocols: Vec<String>,
    /// CIDR ranges allowed for the *destination* IP (post-DNAT).
    pub destination_cidrs: Vec<String>,
    /// Source-IP CIDR allow-list. Empty = all.
    pub source_cidrs: Vec<String>,
    /// Optional source-port range, inclusive.
    pub source_port_range: Option<(u16, u16)>,
    /// Optional destination port (after DNAT).
    pub destination_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportProtocol {
    /// Plain TCP (no TLS).
    RawBuffer,
    /// TLS detected on the wire.
    Tls,
    /// QUIC datagrams (UDP framing).
    Quic,
}

/// Inputs the listener observes about the inbound connection, used to score
/// filter-chain matches.
#[derive(Debug, Clone)]
pub struct ConnectionInfo<'a> {
    pub server_name: Option<&'a str>,
    pub transport_protocol: TransportProtocol,
    pub application_protocols: &'a [&'a str],
    pub source_ip: IpAddr,
    pub source_port: u16,
    pub destination_ip: IpAddr,
    pub destination_port: u16,
}

impl FilterChainMatch {
    /// Matches this predicate against the inbound connection. Returns the
    /// specificity score on a hit. None means no match.
    ///
    /// Envoy's match scoring rules (from
    /// `envoy/source/common/listener_manager/listener_impl.cc`):
    ///
    /// 1. Most-specific destination port match.
    /// 2. Most-specific destination CIDR.
    /// 3. server_names (longest suffix match wins).
    /// 4. transport_protocol exact.
    /// 5. application_protocols set membership (more specific = better).
    /// 6. source CIDR / source port.
    pub fn match_score(&self, info: &ConnectionInfo<'_>) -> Option<u64> {
        let mut score: u64 = 0;

        if let Some(p) = self.destination_port {
            if p != info.destination_port {
                return None;
            }
            score += 1 << 32;
        }

        if !self.destination_cidrs.is_empty() {
            let best = self
                .destination_cidrs
                .iter()
                .filter_map(|c| match_cidr(c, info.destination_ip))
                .max();
            match best {
                Some(prefix) => score += (prefix as u64) << 28,
                None => return None,
            }
        }

        if !self.server_names.is_empty() {
            let server = info.server_name.unwrap_or("");
            let best = self
                .server_names
                .iter()
                .filter_map(|n| sni_match_score(n, server))
                .max();
            match best {
                Some(s) => score += (s as u64) << 22,
                None => return None,
            }
        }

        if let Some(t) = self.transport_protocol {
            if t != info.transport_protocol {
                return None;
            }
            score += 1 << 21;
        }

        if !self.application_protocols.is_empty() {
            let hit = self
                .application_protocols
                .iter()
                .any(|ap| info.application_protocols.iter().any(|c| c == ap));
            if !hit {
                return None;
            }
            // More specific = fewer entries → higher score.
            let bonus = (16u64.saturating_sub(self.application_protocols.len() as u64)) << 16;
            score += bonus;
        }

        if !self.source_cidrs.is_empty() {
            let best = self
                .source_cidrs
                .iter()
                .filter_map(|c| match_cidr(c, info.source_ip))
                .max();
            match best {
                Some(prefix) => score += (prefix as u64) << 8,
                None => return None,
            }
        }

        if let Some((lo, hi)) = self.source_port_range {
            if info.source_port < lo || info.source_port > hi {
                return None;
            }
            score += 1;
        }

        Some(score)
    }
}

fn sni_match_score(pattern: &str, server: &str) -> Option<u32> {
    if pattern == server {
        return Some(255);
    }
    // RFC 6125 wildcard: `*.example.com` matches exactly one label
    // (e.g. `api.example.com` but not `a.b.example.com` or `example.com`).
    if let Some(suffix) = pattern.strip_prefix("*.") {
        let dot_pos = server.find('.')?;
        let label = &server[..dot_pos];
        let rest = &server[dot_pos + 1..];
        if !label.is_empty() && rest == suffix {
            // Score by suffix length so longer wildcards beat shorter.
            return Some((suffix.len().min(254)) as u32);
        }
    }
    None
}

fn match_cidr(cidr: &str, ip: IpAddr) -> Option<u32> {
    let parsed: ipnet::IpNet = cidr.parse().ok()?;
    if parsed.contains(&ip) {
        Some(parsed.prefix_len() as u32)
    } else {
        None
    }
}

/// One filter chain — match predicate + the protocols / TLS / HCM that apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterChain {
    pub name: String,
    pub r#match: FilterChainMatch,
    pub tls: Option<TlsTermination>,
    pub hcm: Option<HttpConnectionManager>,
    pub quic: Option<QuicConfig>,
    pub pool: ConnectionPool,
    /// Wire protocols this chain serves. Driven by ALPN if TLS is present.
    pub protocols: Vec<WireProtocol>,
}

impl FilterChain {
    pub fn http_plain(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            r#match: FilterChainMatch {
                transport_protocol: Some(TransportProtocol::RawBuffer),
                ..Default::default()
            },
            tls: None,
            hcm: Some(HttpConnectionManager::default()),
            quic: None,
            pool: ConnectionPool::default(),
            protocols: vec![WireProtocol::Http1, WireProtocol::Http2],
        }
    }

    pub fn https(name: impl Into<String>, sni: Vec<String>, cert_ids: Vec<Uuid>) -> Self {
        let mut tls = TlsTermination::default();
        tls.sni_hostnames = sni.clone();
        tls.certificate_ids = cert_ids;
        Self {
            name: name.into(),
            r#match: FilterChainMatch {
                server_names: sni,
                transport_protocol: Some(TransportProtocol::Tls),
                application_protocols: vec!["h2".into(), "http/1.1".into()],
                ..Default::default()
            },
            tls: Some(tls),
            hcm: Some(HttpConnectionManager::default()),
            quic: None,
            pool: ConnectionPool::default(),
            protocols: vec![WireProtocol::Http1, WireProtocol::Http2],
        }
    }

    pub fn http3(name: impl Into<String>, sni: Vec<String>, cert_ids: Vec<Uuid>) -> Self {
        let mut tls = TlsTermination::default();
        tls.sni_hostnames = sni.clone();
        tls.certificate_ids = cert_ids;
        tls.alpn = AlpnList::http3_offload();
        Self {
            name: name.into(),
            r#match: FilterChainMatch {
                server_names: sni,
                transport_protocol: Some(TransportProtocol::Quic),
                application_protocols: vec!["h3".into()],
                ..Default::default()
            },
            tls: Some(tls),
            hcm: Some(HttpConnectionManager {
                codec: CodecType::Http3,
                ..Default::default()
            }),
            quic: Some(QuicConfig::default()),
            pool: ConnectionPool::default(),
            protocols: vec![WireProtocol::Http3],
        }
    }

    pub fn validate(&self) -> Result<(), ListenerError> {
        if self.name.is_empty() {
            return Err(ListenerError::ChainConfig("empty name".into()));
        }
        if let Some(t) = &self.tls {
            t.validate()?;
        }
        if let Some(h) = &self.hcm {
            h.validate()?;
        }
        if let Some(q) = &self.quic {
            q.validate()?;
            if !self.protocols.iter().any(|p| p.is_quic()) {
                return Err(ListenerError::ChainConfig(
                    "quic config requires HTTP/3 in protocols".into(),
                ));
            }
        }
        if self.protocols.iter().any(|p| p.defaults_to_tls()) && self.tls.is_none() {
            // h2/h3 over plaintext is unusual but legal (h2c). We don't reject;
            // we only reject the inverse (HTTPS without TLS context elsewhere).
        }
        self.pool.validate()?;
        Ok(())
    }
}

// ── Listener ─────────────────────────────────────────────────────────────────

/// A complete listener — bind address, named filter chains, and global knobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listener {
    pub name: String,
    pub address: BindAddress,
    pub filter_chains: Vec<FilterChain>,
    pub default_filter_chain: Option<FilterChain>,
    pub freebind: bool,
    pub transparent: bool,
    pub reuse_port: bool,
    pub per_connection_buffer_limit_kb: u32,
    pub listener_filters_timeout: Duration,
    pub continue_on_listener_filters_timeout: bool,
    pub tcp_fast_open_queue_length: u32,
    pub traffic_direction: TrafficDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TrafficDirection {
    Unspecified,
    Inbound,
    Outbound,
}

impl Listener {
    pub fn http(name: impl Into<String>, port: u16) -> Self {
        Self {
            name: name.into(),
            address: BindAddress::any_v4(port),
            filter_chains: vec![FilterChain::http_plain("default")],
            default_filter_chain: None,
            freebind: false,
            transparent: false,
            reuse_port: true,
            per_connection_buffer_limit_kb: 1024,
            listener_filters_timeout: Duration::from_secs(15),
            continue_on_listener_filters_timeout: false,
            tcp_fast_open_queue_length: 0,
            traffic_direction: TrafficDirection::Inbound,
        }
    }

    pub fn https(
        name: impl Into<String>,
        port: u16,
        sni: Vec<String>,
        cert_ids: Vec<Uuid>,
    ) -> Self {
        let mut l = Self::http(name, port);
        l.filter_chains = vec![FilterChain::https("default", sni, cert_ids)];
        l
    }

    pub fn validate(&self) -> Result<(), ListenerError> {
        if self.name.is_empty() {
            return Err(ListenerError::ListenerConfig("empty name".into()));
        }
        if self.filter_chains.is_empty() && self.default_filter_chain.is_none() {
            return Err(ListenerError::ListenerConfig(
                "no filter chains and no default chain".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for c in &self.filter_chains {
            if !seen.insert(c.name.as_str()) {
                return Err(ListenerError::ListenerConfig(format!(
                    "duplicate filter chain name: {}",
                    c.name
                )));
            }
            c.validate()?;
        }
        if let Some(d) = &self.default_filter_chain {
            d.validate()?;
        }

        // Port range check.
        if let BindAddress::Socket { port, .. } = &self.address {
            if *port == 0 {
                // 0 = ephemeral; allowed for tests but rejected here.
                return Err(ListenerError::ListenerConfig(
                    "port 0 is not allowed for production listeners".into(),
                ));
            }
        }

        // QUIC listener must bind UDP semantics — we model that by saying every
        // chain on the listener has Quic transport_protocol if any does.
        let quic_count = self
            .filter_chains
            .iter()
            .filter(|c| c.r#match.transport_protocol == Some(TransportProtocol::Quic))
            .count();
        if quic_count > 0 && quic_count != self.filter_chains.len() {
            return Err(ListenerError::ListenerConfig(
                "QUIC and TCP filter chains cannot coexist on one listener".into(),
            ));
        }
        Ok(())
    }

    /// Pick the best filter chain for the given inbound connection.
    pub fn select_chain(&self, info: &ConnectionInfo<'_>) -> Option<&FilterChain> {
        let mut best: Option<(u64, &FilterChain)> = None;
        for c in &self.filter_chains {
            if let Some(score) = c.r#match.match_score(info) {
                match best {
                    None => best = Some((score, c)),
                    Some((s, _)) if score > s => best = Some((score, c)),
                    _ => {}
                }
            }
        }
        best.map(|(_, c)| c).or(self.default_filter_chain.as_ref())
    }
}

// ── Listener registry ────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct ListenerRegistry {
    inner: RwLock<BTreeMap<String, Arc<Listener>>>,
}

impl ListenerRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn upsert(&self, l: Listener) -> Result<(), ListenerError> {
        l.validate()?;
        let mut g = self.inner.write().unwrap();
        g.insert(l.name.clone(), Arc::new(l));
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<Listener>> {
        self.inner.read().unwrap().get(name).cloned()
    }

    pub fn list(&self) -> Vec<Arc<Listener>> {
        self.inner.read().unwrap().values().cloned().collect()
    }

    pub fn remove(&self, name: &str) -> bool {
        self.inner.write().unwrap().remove(name).is_some()
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().unwrap().is_empty()
    }
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ListenerError {
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    #[error("listener config: {0}")]
    ListenerConfig(String),
    #[error("filter chain: {0}")]
    ChainConfig(String),
    #[error("tls config: {0}")]
    TlsConfig(String),
    #[error("hcm config: {0}")]
    HcmConfig(String),
    #[error("quic config: {0}")]
    QuicConfig(String),
    #[error("pool config: {0}")]
    PoolConfig(String),
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn info<'a>(
        sni: Option<&'a str>,
        transport: TransportProtocol,
        alpns: &'a [&'a str],
        src: IpAddr,
        src_port: u16,
        dst: IpAddr,
        dst_port: u16,
    ) -> ConnectionInfo<'a> {
        ConnectionInfo {
            server_name: sni,
            transport_protocol: transport,
            application_protocols: alpns,
            source_ip: src,
            source_port: src_port,
            destination_ip: dst,
            destination_port: dst_port,
        }
    }

    // ── BindAddress parsing ──

    #[test]
    fn bind_addr_parse_ipv4() {
        let a: BindAddress = "0.0.0.0:8080".parse().unwrap();
        assert_eq!(a, BindAddress::any_v4(8080));
        assert!(a.is_ipv4());
    }

    #[test]
    fn bind_addr_parse_ipv6() {
        let a: BindAddress = "[::]:8443".parse().unwrap();
        assert!(a.is_ipv6());
    }

    #[test]
    fn bind_addr_parse_loopback() {
        let a: BindAddress = "127.0.0.1:80".parse().unwrap();
        assert_eq!(a.as_socket().unwrap().port(), 80);
    }

    #[test]
    fn bind_addr_parse_port_only() {
        let a: BindAddress = ":8080".parse().unwrap();
        assert_eq!(a, BindAddress::any_v4(8080));
    }

    #[test]
    fn bind_addr_parse_unix() {
        let a: BindAddress = "unix:/var/run/cave.sock".parse().unwrap();
        assert!(matches!(a, BindAddress::Unix { .. }));
        assert!(a.as_socket().is_none());
    }

    #[test]
    fn bind_addr_parse_unix_empty_rejects() {
        let r: Result<BindAddress, _> = "unix:".parse();
        assert!(r.is_err());
    }

    #[test]
    fn bind_addr_parse_invalid() {
        let r: Result<BindAddress, _> = "not-an-address".parse();
        assert!(r.is_err());
    }

    #[test]
    fn bind_addr_unix_helper() {
        let a = BindAddress::unix("/tmp/x.sock");
        assert!(matches!(a, BindAddress::Unix { .. }));
    }

    #[test]
    fn bind_addr_any_v6() {
        let a = BindAddress::any_v6(443);
        assert!(a.is_ipv6());
    }

    // ── WireProtocol & ALPN ──

    #[test]
    fn alpn_identifiers() {
        assert_eq!(WireProtocol::Http1.alpn(), Some("http/1.1"));
        assert_eq!(WireProtocol::Http2.alpn(), Some("h2"));
        assert_eq!(WireProtocol::Http3.alpn(), Some("h3"));
        assert_eq!(WireProtocol::Tcp.alpn(), None);
        assert_eq!(WireProtocol::Udp.alpn(), None);
    }

    #[test]
    fn protocol_quic_flag() {
        assert!(WireProtocol::Http3.is_quic());
        assert!(!WireProtocol::Http2.is_quic());
        assert!(!WireProtocol::Http1.is_quic());
    }

    #[test]
    fn protocol_defaults_to_tls() {
        assert!(WireProtocol::Http2.defaults_to_tls());
        assert!(WireProtocol::Http3.defaults_to_tls());
        assert!(!WireProtocol::Http1.defaults_to_tls());
    }

    #[test]
    fn alpn_list_h2_first_negotiation() {
        let list = AlpnList::http2_first();
        assert_eq!(list.negotiate(&["h2", "http/1.1"]), Some(WireProtocol::Http2));
        assert_eq!(list.negotiate(&["http/1.1"]), Some(WireProtocol::Http1));
        assert_eq!(list.negotiate(&["spdy/3"]), None);
    }

    #[test]
    fn alpn_list_h3_offload() {
        let list = AlpnList::http3_offload();
        assert_eq!(list.negotiate(&["h3"]), Some(WireProtocol::Http3));
    }

    #[test]
    fn alpn_list_as_wire() {
        assert_eq!(AlpnList::http2_first().as_wire(), vec!["h2", "http/1.1"]);
    }

    #[test]
    fn alpn_list_contains() {
        assert!(AlpnList::http2_first().contains(WireProtocol::Http2));
        assert!(!AlpnList::http1_only().contains(WireProtocol::Http2));
    }

    // ── CodecType ──

    #[test]
    fn codec_auto_picks_alpn() {
        assert_eq!(CodecType::Auto.pick(Some("h2")), WireProtocol::Http2);
        assert_eq!(CodecType::Auto.pick(Some("h3")), WireProtocol::Http3);
        assert_eq!(CodecType::Auto.pick(Some("http/1.1")), WireProtocol::Http1);
        assert_eq!(CodecType::Auto.pick(None), WireProtocol::Http1);
    }

    #[test]
    fn codec_explicit_overrides_alpn() {
        assert_eq!(CodecType::Http1.pick(Some("h2")), WireProtocol::Http1);
        assert_eq!(CodecType::Http2.pick(None), WireProtocol::Http2);
    }

    // ── TLS config ──

    #[test]
    fn tls_default_validates() {
        TlsTermination::default().validate().unwrap();
    }

    #[test]
    fn tls_min_max_inverted_rejects() {
        let mut t = TlsTermination::default();
        t.min_version = TlsVersion::Tls13;
        t.max_version = TlsVersion::Tls12;
        assert!(t.validate().is_err());
    }

    #[test]
    fn tls_required_client_auth_needs_trust_bundle() {
        let mut t = TlsTermination::default();
        t.client_auth = ClientAuth::Required;
        assert!(t.validate().is_err());
        t.trust_bundle_id = Some(Uuid::new_v4());
        t.validate().unwrap();
    }

    #[test]
    fn tls_optional_client_auth_needs_trust_bundle() {
        let mut t = TlsTermination::default();
        t.client_auth = ClientAuth::Optional;
        assert!(t.validate().is_err());
    }

    #[test]
    fn tls_optional_no_ca_does_not_need_trust_bundle() {
        let mut t = TlsTermination::default();
        t.client_auth = ClientAuth::OptionalNoCa;
        t.validate().unwrap();
    }

    #[test]
    fn tls_empty_alpn_rejects() {
        let mut t = TlsTermination::default();
        t.alpn = AlpnList(vec![]);
        assert!(t.validate().is_err());
    }

    #[test]
    fn tls_version_ietf_codepoints() {
        assert_eq!(TlsVersion::Tls12.ietf(), 0x0303);
        assert_eq!(TlsVersion::Tls13.ietf(), 0x0304);
    }

    #[test]
    fn tls_version_ordering() {
        assert!(TlsVersion::Tls12 < TlsVersion::Tls13);
    }

    // ── HCM config ──

    #[test]
    fn hcm_default_validates() {
        HttpConnectionManager::default().validate().unwrap();
    }

    #[test]
    fn hcm_empty_stat_prefix_rejects() {
        let mut h = HttpConnectionManager::default();
        h.stat_prefix = "".into();
        assert!(h.validate().is_err());
    }

    #[test]
    fn hcm_max_headers_zero_rejects() {
        let mut h = HttpConnectionManager::default();
        h.max_request_headers_kb = 0;
        assert!(h.validate().is_err());
    }

    #[test]
    fn hcm_max_headers_too_big_rejects() {
        let mut h = HttpConnectionManager::default();
        h.max_request_headers_kb = 9000;
        assert!(h.validate().is_err());
    }

    // ── Connection pool ──

    #[test]
    fn pool_default_validates() {
        ConnectionPool::default().validate().unwrap();
    }

    #[test]
    fn pool_zero_connections_rejects() {
        let mut p = ConnectionPool::default();
        p.max_connections = 0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn pool_zero_pending_rejects() {
        let mut p = ConnectionPool::default();
        p.max_pending_requests = 0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn keepalive_default_sane() {
        let k = KeepAlive::default();
        assert!(k.probes > 0);
        assert!(k.interval > Duration::ZERO);
    }

    // ── QUIC config ──

    #[test]
    fn quic_default_validates() {
        QuicConfig::default().validate().unwrap();
    }

    #[test]
    fn quic_zero_streams_rejects() {
        let mut q = QuicConfig::default();
        q.max_concurrent_streams = 0;
        assert!(q.validate().is_err());
    }

    #[test]
    fn quic_zero_idle_rejects() {
        let mut q = QuicConfig::default();
        q.idle_timeout = Duration::from_millis(500);
        assert!(q.validate().is_err());
    }

    // ── FilterChainMatch ──

    #[test]
    fn match_unconstrained_accepts_anything() {
        let m = FilterChainMatch::default();
        let i = info(
            None,
            TransportProtocol::RawBuffer,
            &[],
            "1.2.3.4".parse().unwrap(),
            10000,
            "10.0.0.1".parse().unwrap(),
            8080,
        );
        assert!(m.match_score(&i).is_some());
    }

    #[test]
    fn match_destination_port_must_match() {
        let m = FilterChainMatch {
            destination_port: Some(443),
            ..Default::default()
        };
        let dst: IpAddr = "10.0.0.1".parse().unwrap();
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        assert!(m.match_score(&info(None, TransportProtocol::RawBuffer, &[], src, 1, dst, 80)).is_none());
        assert!(m.match_score(&info(None, TransportProtocol::RawBuffer, &[], src, 1, dst, 443)).is_some());
    }

    #[test]
    fn match_sni_exact_beats_wildcard() {
        let exact = FilterChainMatch {
            server_names: vec!["api.example.com".into()],
            ..Default::default()
        };
        let wild = FilterChainMatch {
            server_names: vec!["*.example.com".into()],
            ..Default::default()
        };
        let dst: IpAddr = "10.0.0.1".parse().unwrap();
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        let i = info(Some("api.example.com"), TransportProtocol::Tls, &[], src, 1, dst, 443);
        let e = exact.match_score(&i).unwrap();
        let w = wild.match_score(&i).unwrap();
        assert!(e > w);
    }

    #[test]
    fn match_sni_wildcard_does_not_match_apex() {
        let m = FilterChainMatch {
            server_names: vec!["*.example.com".into()],
            ..Default::default()
        };
        let dst: IpAddr = "10.0.0.1".parse().unwrap();
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        let i = info(Some("example.com"), TransportProtocol::Tls, &[], src, 1, dst, 443);
        assert!(m.match_score(&i).is_none());
    }

    #[test]
    fn match_transport_protocol_required() {
        let m = FilterChainMatch {
            transport_protocol: Some(TransportProtocol::Tls),
            ..Default::default()
        };
        let dst: IpAddr = "10.0.0.1".parse().unwrap();
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        assert!(m.match_score(&info(None, TransportProtocol::RawBuffer, &[], src, 1, dst, 443)).is_none());
        assert!(m.match_score(&info(None, TransportProtocol::Tls, &[], src, 1, dst, 443)).is_some());
    }

    #[test]
    fn match_application_protocols_require_intersection() {
        let m = FilterChainMatch {
            application_protocols: vec!["h2".into()],
            ..Default::default()
        };
        let dst: IpAddr = "10.0.0.1".parse().unwrap();
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        assert!(m.match_score(&info(None, TransportProtocol::Tls, &["http/1.1"], src, 1, dst, 443)).is_none());
        assert!(m.match_score(&info(None, TransportProtocol::Tls, &["h2"], src, 1, dst, 443)).is_some());
    }

    #[test]
    fn match_more_specific_app_protocol_set_scores_higher() {
        let one = FilterChainMatch {
            application_protocols: vec!["h2".into()],
            ..Default::default()
        };
        let two = FilterChainMatch {
            application_protocols: vec!["h2".into(), "http/1.1".into()],
            ..Default::default()
        };
        let dst: IpAddr = "10.0.0.1".parse().unwrap();
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        let i = info(None, TransportProtocol::Tls, &["h2"], src, 1, dst, 443);
        assert!(one.match_score(&i).unwrap() > two.match_score(&i).unwrap());
    }

    #[test]
    fn match_destination_cidr_filters() {
        let m = FilterChainMatch {
            destination_cidrs: vec!["10.0.0.0/24".into()],
            ..Default::default()
        };
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        let inside: IpAddr = "10.0.0.5".parse().unwrap();
        let outside: IpAddr = "192.168.0.1".parse().unwrap();
        assert!(m.match_score(&info(None, TransportProtocol::Tls, &[], src, 1, inside, 443)).is_some());
        assert!(m.match_score(&info(None, TransportProtocol::Tls, &[], src, 1, outside, 443)).is_none());
    }

    #[test]
    fn match_destination_cidr_more_specific_wins() {
        let wide = FilterChainMatch {
            destination_cidrs: vec!["10.0.0.0/8".into()],
            ..Default::default()
        };
        let narrow = FilterChainMatch {
            destination_cidrs: vec!["10.0.0.0/24".into()],
            ..Default::default()
        };
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        let dst: IpAddr = "10.0.0.5".parse().unwrap();
        let i = info(None, TransportProtocol::Tls, &[], src, 1, dst, 443);
        assert!(narrow.match_score(&i).unwrap() > wide.match_score(&i).unwrap());
    }

    #[test]
    fn match_source_cidr_required() {
        let m = FilterChainMatch {
            source_cidrs: vec!["10.0.0.0/24".into()],
            ..Default::default()
        };
        let dst: IpAddr = "1.2.3.4".parse().unwrap();
        let inside: IpAddr = "10.0.0.99".parse().unwrap();
        let outside: IpAddr = "8.8.8.8".parse().unwrap();
        assert!(m.match_score(&info(None, TransportProtocol::Tls, &[], inside, 1, dst, 443)).is_some());
        assert!(m.match_score(&info(None, TransportProtocol::Tls, &[], outside, 1, dst, 443)).is_none());
    }

    #[test]
    fn match_source_port_range() {
        let m = FilterChainMatch {
            source_port_range: Some((10000, 20000)),
            ..Default::default()
        };
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        let dst: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(m.match_score(&info(None, TransportProtocol::Tls, &[], src, 5000, dst, 443)).is_none());
        assert!(m.match_score(&info(None, TransportProtocol::Tls, &[], src, 15000, dst, 443)).is_some());
        assert!(m.match_score(&info(None, TransportProtocol::Tls, &[], src, 25000, dst, 443)).is_none());
    }

    #[test]
    fn match_destination_port_dominates() {
        let port = FilterChainMatch {
            destination_port: Some(443),
            ..Default::default()
        };
        let sni = FilterChainMatch {
            server_names: vec!["api.example.com".into()],
            ..Default::default()
        };
        let dst: IpAddr = "10.0.0.1".parse().unwrap();
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        let i = info(Some("api.example.com"), TransportProtocol::Tls, &[], src, 1, dst, 443);
        // Even an exact SNI match must yield to a destination_port match.
        assert!(port.match_score(&i).unwrap() > sni.match_score(&i).unwrap());
    }

    // ── FilterChain helpers ──

    #[test]
    fn filter_chain_http_plain_validates() {
        FilterChain::http_plain("default").validate().unwrap();
    }

    #[test]
    fn filter_chain_https_validates() {
        let c = FilterChain::https(
            "tls",
            vec!["example.com".into()],
            vec![Uuid::new_v4()],
        );
        c.validate().unwrap();
    }

    #[test]
    fn filter_chain_http3_validates() {
        let c = FilterChain::http3(
            "h3",
            vec!["example.com".into()],
            vec![Uuid::new_v4()],
        );
        c.validate().unwrap();
    }

    #[test]
    fn filter_chain_quic_without_h3_proto_rejects() {
        let mut c = FilterChain::http3("h3", vec!["x".into()], vec![Uuid::new_v4()]);
        c.protocols = vec![WireProtocol::Http2];
        assert!(c.validate().is_err());
    }

    #[test]
    fn filter_chain_empty_name_rejects() {
        let c = FilterChain::http_plain("");
        assert!(c.validate().is_err());
    }

    // ── Listener-level ──

    #[test]
    fn listener_http_default_validates() {
        Listener::http("ingress", 8080).validate().unwrap();
    }

    #[test]
    fn listener_https_validates() {
        let l = Listener::https("tls", 8443, vec!["example.com".into()], vec![Uuid::new_v4()]);
        l.validate().unwrap();
    }

    #[test]
    fn listener_port_zero_rejects() {
        let l = Listener::http("x", 0);
        assert!(l.validate().is_err());
    }

    #[test]
    fn listener_no_chains_rejects() {
        let mut l = Listener::http("x", 8080);
        l.filter_chains.clear();
        assert!(l.validate().is_err());
    }

    #[test]
    fn listener_duplicate_chain_names_reject() {
        let mut l = Listener::http("x", 8080);
        l.filter_chains.push(FilterChain::http_plain("default"));
        assert!(l.validate().is_err());
    }

    #[test]
    fn listener_quic_and_tcp_mixed_rejects() {
        let mut l = Listener::http("x", 8080);
        l.filter_chains.push(FilterChain::http3(
            "h3",
            vec!["example.com".into()],
            vec![Uuid::new_v4()],
        ));
        assert!(l.validate().is_err());
    }

    #[test]
    fn listener_select_chain_picks_best() {
        let mut l = Listener::http("x", 443);
        let mut tls = FilterChain::https("api", vec!["api.example.com".into()], vec![Uuid::new_v4()]);
        tls.r#match.application_protocols = vec!["h2".into()];
        l.filter_chains = vec![FilterChain::http_plain("default"), tls];

        let dst: IpAddr = "10.0.0.1".parse().unwrap();
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        let i = info(
            Some("api.example.com"),
            TransportProtocol::Tls,
            &["h2"],
            src,
            1,
            dst,
            443,
        );
        assert_eq!(l.select_chain(&i).unwrap().name, "api");
    }

    #[test]
    fn listener_select_chain_falls_through_to_default() {
        let mut l = Listener::http("x", 443);
        let mut tls = FilterChain::https("api", vec!["only-this.example.com".into()], vec![Uuid::new_v4()]);
        tls.r#match.application_protocols = vec!["h2".into()];
        l.filter_chains = vec![tls];
        l.default_filter_chain = Some(FilterChain::http_plain("fallback"));

        let dst: IpAddr = "10.0.0.1".parse().unwrap();
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        let i = info(Some("other.com"), TransportProtocol::Tls, &["h2"], src, 1, dst, 443);
        assert_eq!(l.select_chain(&i).unwrap().name, "fallback");
    }

    #[test]
    fn listener_select_chain_no_match_returns_none_when_no_default() {
        let mut l = Listener::http("x", 443);
        let mut tls = FilterChain::https("api", vec!["only.example.com".into()], vec![Uuid::new_v4()]);
        tls.r#match.application_protocols = vec!["h2".into()];
        l.filter_chains = vec![tls];

        let dst: IpAddr = "10.0.0.1".parse().unwrap();
        let src: IpAddr = "1.2.3.4".parse().unwrap();
        let i = info(Some("nope.example.com"), TransportProtocol::Tls, &["h2"], src, 1, dst, 443);
        assert!(l.select_chain(&i).is_none());
    }

    // ── Registry ──

    #[test]
    fn registry_upsert_and_get() {
        let r = ListenerRegistry::new();
        r.upsert(Listener::http("ingress", 8080)).unwrap();
        assert!(r.get("ingress").is_some());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn registry_upsert_invalid_rejects() {
        let r = ListenerRegistry::new();
        let l = Listener::http("ingress", 0);
        assert!(r.upsert(l).is_err());
        assert!(r.is_empty());
    }

    #[test]
    fn registry_remove() {
        let r = ListenerRegistry::new();
        r.upsert(Listener::http("ingress", 8080)).unwrap();
        assert!(r.remove("ingress"));
        assert!(!r.remove("ingress"));
    }

    #[test]
    fn registry_list_sorted() {
        let r = ListenerRegistry::new();
        r.upsert(Listener::http("b", 8080)).unwrap();
        r.upsert(Listener::http("a", 8081)).unwrap();
        let names: Vec<_> = r.list().iter().map(|l| l.name.clone()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn registry_upsert_replaces() {
        let r = ListenerRegistry::new();
        r.upsert(Listener::http("ingress", 8080)).unwrap();
        r.upsert(Listener::http("ingress", 8081)).unwrap();
        let l = r.get("ingress").unwrap();
        assert_eq!(l.address.as_socket().unwrap().port(), 8081);
    }

    #[test]
    fn sni_match_score_helper_exact() {
        assert!(super::sni_match_score("api.example.com", "api.example.com").unwrap() > 0);
    }

    #[test]
    fn sni_match_score_helper_wildcard() {
        let s = super::sni_match_score("*.example.com", "api.example.com").unwrap();
        // Wildcard score is lower than exact (255).
        assert!(s < 255);
    }

    #[test]
    fn match_cidr_helper_v4() {
        let prefix = super::match_cidr("10.0.0.0/24", "10.0.0.5".parse().unwrap()).unwrap();
        assert_eq!(prefix, 24);
        assert!(super::match_cidr("10.0.0.0/24", "11.0.0.5".parse().unwrap()).is_none());
    }

    #[test]
    fn match_cidr_helper_v6() {
        let prefix = super::match_cidr("fe80::/10", "fe80::1".parse().unwrap()).unwrap();
        assert_eq!(prefix, 10);
    }

    // ── Serde round-trip ──

    #[test]
    fn listener_json_roundtrip() {
        let l = Listener::https(
            "tls",
            8443,
            vec!["example.com".into()],
            vec![Uuid::new_v4()],
        );
        let s = serde_json::to_string(&l).unwrap();
        let l2: Listener = serde_json::from_str(&s).unwrap();
        assert_eq!(l, l2);
    }

    #[test]
    fn filter_chain_json_roundtrip() {
        let c = FilterChain::https("api", vec!["api.example.com".into()], vec![Uuid::new_v4()]);
        let s = serde_json::to_string(&c).unwrap();
        let c2: FilterChain = serde_json::from_str(&s).unwrap();
        assert_eq!(c, c2);
    }

    #[test]
    fn alpn_list_json_roundtrip() {
        let l = AlpnList::http3_offload();
        let s = serde_json::to_string(&l).unwrap();
        let l2: AlpnList = serde_json::from_str(&s).unwrap();
        assert_eq!(l, l2);
    }

    #[test]
    fn quic_config_json_roundtrip() {
        let q = QuicConfig::default();
        let s = serde_json::to_string(&q).unwrap();
        let q2: QuicConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(q, q2);
    }
}
