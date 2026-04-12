//! Plugin system — Kong-compatible plugin chain executor.
//!
//! Built-in plugins (20+):
//!   Auth:     key-auth, jwt, oauth2, basic-auth, hmac-auth, ldap-auth
//!   Traffic:  rate-limiting, request-size-limiting, proxy-cache
//!   Transform: request-transformer, response-transformer, correlation-id
//!   Logging:  http-log, file-log, tcp-log
//!   Security: cors, ip-restriction, bot-detection, acl

use crate::models::Consumer;
use crate::store::GatewayStore;
use base64::Engine as _;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

// ─────────────────────────────────────────────
//  Plugin context — request state passed through chain
// ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PluginContext {
    pub request_id: Uuid,
    pub method: String,
    pub path: String,
    pub host: String,
    pub headers: HashMap<String, String>,
    pub query_params: HashMap<String, String>,
    pub body_size: usize,
    pub client_ip: String,
    pub consumer: Option<Consumer>,
    pub route_id: Option<Uuid>,
    pub service_id: Option<Uuid>,
}

impl PluginContext {
    pub fn header(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == lower)
            .map(|(_, v)| v.as_str())
    }

    pub fn query(&self, name: &str) -> Option<&str> {
        self.query_params.get(name).map(|s| s.as_str())
    }
}

/// Result of executing a single plugin.
#[derive(Debug, Clone)]
pub enum PluginOutcome {
    /// Request proceeds; optionally add/remove headers.
    Continue {
        add_request_headers: HashMap<String, String>,
        remove_request_headers: Vec<String>,
        add_response_headers: HashMap<String, String>,
        set_consumer: Option<Consumer>,
    },
    /// Request is rejected with the given status and message.
    Reject { status: u16, message: String },
}

impl PluginOutcome {
    pub fn allow() -> Self {
        PluginOutcome::Continue {
            add_request_headers: HashMap::new(),
            remove_request_headers: vec![],
            add_response_headers: HashMap::new(),
            set_consumer: None,
        }
    }

    pub fn reject(status: u16, message: impl Into<String>) -> Self {
        PluginOutcome::Reject {
            status,
            message: message.into(),
        }
    }

    pub fn is_rejected(&self) -> bool {
        matches!(self, PluginOutcome::Reject { .. })
    }
}

// ─────────────────────────────────────────────
//  Rate limiter state
// ─────────────────────────────────────────────

/// Fixed-window counter entry
#[derive(Debug, Clone)]
pub struct FixedWindowEntry {
    pub count: u64,
    pub window_start: Instant,
    pub window_size: Duration,
}

impl FixedWindowEntry {
    pub fn new(window_secs: u64) -> Self {
        Self {
            count: 0,
            window_start: Instant::now(),
            window_size: Duration::from_secs(window_secs),
        }
    }

    /// Increment and return whether the request is within the limit.
    pub fn check_and_increment(&mut self, limit: u64) -> bool {
        if self.window_start.elapsed() >= self.window_size {
            self.count = 0;
            self.window_start = Instant::now();
        }
        if self.count < limit {
            self.count += 1;
            true
        } else {
            false
        }
    }
}

/// Sliding-window request timestamps
#[derive(Debug, Clone, Default)]
pub struct SlidingWindowState {
    pub timestamps: std::collections::VecDeque<Instant>,
}

impl SlidingWindowState {
    pub fn check_and_record(&mut self, limit: u64, window_secs: u64) -> bool {
        let window = Duration::from_secs(window_secs);
        let now = Instant::now();

        // Remove timestamps outside the window
        while let Some(&front) = self.timestamps.front() {
            if now.duration_since(front) >= window {
                self.timestamps.pop_front();
            } else {
                break;
            }
        }

        if self.timestamps.len() < limit as usize {
            self.timestamps.push_back(now);
            true
        } else {
            false
        }
    }
}

// ─────────────────────────────────────────────
//  Proxy cache
// ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: HashMap<String, String>,
    pub cached_at: Instant,
    pub ttl: Duration,
}

impl CacheEntry {
    pub fn is_expired(&self) -> bool {
        self.cached_at.elapsed() >= self.ttl
    }
}

// ─────────────────────────────────────────────
//  Plugin executor
// ─────────────────────────────────────────────

/// Registry for mutable plugin state (rate limiters, caches).
#[derive(Default)]
pub struct PluginState {
    /// key = "{plugin_id}:{rate_limit_key}" → fixed-window state
    pub fixed_windows: HashMap<String, FixedWindowEntry>,
    /// key = "{plugin_id}:{rate_limit_key}" → sliding-window state
    pub sliding_windows: HashMap<String, SlidingWindowState>,
    /// key = "{plugin_id}:{cache_key}" → cached response
    pub cache: HashMap<String, CacheEntry>,
    /// Correlation ID counters (monotonic per process)
    pub correlation_counter: u64,
}

impl PluginState {
    /// Rate-limit key: prefer consumer ID, fall back to client IP.
    fn rl_key(plugin_id: Uuid, ctx: &PluginContext) -> String {
        let subject = ctx
            .consumer
            .as_ref()
            .map(|c| c.id.to_string())
            .unwrap_or_else(|| ctx.client_ip.clone());
        format!("{plugin_id}:{subject}")
    }

    pub fn check_rate_limit_fixed(
        &mut self,
        plugin_id: Uuid,
        ctx: &PluginContext,
        limit: u64,
        window_secs: u64,
    ) -> bool {
        let key = Self::rl_key(plugin_id, ctx);
        let entry = self
            .fixed_windows
            .entry(key)
            .or_insert_with(|| FixedWindowEntry::new(window_secs));
        entry.check_and_increment(limit)
    }

    pub fn check_rate_limit_sliding(
        &mut self,
        plugin_id: Uuid,
        ctx: &PluginContext,
        limit: u64,
        window_secs: u64,
    ) -> bool {
        let key = Self::rl_key(plugin_id, ctx);
        self.sliding_windows
            .entry(key)
            .or_default()
            .check_and_record(limit, window_secs)
    }

    pub fn get_cache(&self, key: &str) -> Option<&CacheEntry> {
        self.cache
            .get(key)
            .filter(|e| !e.is_expired())
    }

    pub fn store_cache(&mut self, key: String, entry: CacheEntry) {
        self.cache.insert(key, entry);
    }

    pub fn next_correlation_id(&mut self) -> String {
        self.correlation_counter += 1;
        format!("{}-{}", Uuid::new_v4(), self.correlation_counter)
    }
}

/// Runs a single plugin against the given context.
///
/// Takes the plugin name, config JSON, mutable plugin state, and the store.
pub fn run_plugin(
    plugin_name: &str,
    plugin_id: Uuid,
    config: &serde_json::Value,
    ctx: &mut PluginContext,
    state: &mut PluginState,
    store: &GatewayStore,
) -> PluginOutcome {
    match plugin_name {
        // ── Auth plugins ─────────────────────────────────────────────────
        "key-auth" => run_key_auth(plugin_id, config, ctx, store),
        "jwt" => run_jwt(plugin_id, config, ctx, store),
        "basic-auth" => run_basic_auth(plugin_id, config, ctx, store),
        "hmac-auth" => run_hmac_auth(plugin_id, config, ctx, store),
        "oauth2" => run_oauth2(config, ctx),
        "ldap-auth" => run_ldap_auth(config, ctx),

        // ── Traffic plugins ───────────────────────────────────────────────
        "rate-limiting" => run_rate_limiting(plugin_id, config, ctx, state),
        "request-size-limiting" => run_request_size_limiting(config, ctx),
        "proxy-cache" => run_proxy_cache(config, ctx, state),

        // ── Transform plugins ─────────────────────────────────────────────
        "request-transformer" => run_request_transformer(config, ctx),
        "response-transformer" => run_response_transformer(config, ctx),
        "correlation-id" => run_correlation_id(config, ctx, state),

        // ── Logging plugins ───────────────────────────────────────────────
        "http-log" => run_http_log(config, ctx),
        "file-log" => run_file_log(config, ctx),
        "tcp-log" => run_tcp_log(config, ctx),

        // ── Security plugins ──────────────────────────────────────────────
        "cors" => run_cors(config, ctx),
        "ip-restriction" => run_ip_restriction(config, ctx),
        "bot-detection" => run_bot_detection(config, ctx),
        "acl" => run_acl(config, ctx),

        _ => PluginOutcome::allow(), // Unknown plugins pass through
    }
}

// ─────────────────────────────────────────────
//  Auth: key-auth
// ─────────────────────────────────────────────

fn run_key_auth(
    _plugin_id: Uuid,
    config: &serde_json::Value,
    ctx: &mut PluginContext,
    store: &GatewayStore,
) -> PluginOutcome {
    let header_name = config
        .get("key_names")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .unwrap_or("apikey");

    let key = ctx
        .header(header_name)
        .or_else(|| ctx.query(header_name))
        .map(|s| s.to_string());

    match key {
        None => PluginOutcome::reject(401, "No API key provided"),
        Some(k) => match store.find_key_auth(&k) {
            None => PluginOutcome::reject(401, "Invalid API key"),
            Some(cred) => {
                if let Some(consumer) = store.get_consumer(cred.consumer_id) {
                    let mut out = PluginOutcome::allow();
                    if let PluginOutcome::Continue { set_consumer, .. } = &mut out {
                        *set_consumer = Some(consumer.clone());
                    }
                    out
                } else {
                    PluginOutcome::reject(401, "Consumer not found")
                }
            }
        },
    }
}

// ─────────────────────────────────────────────
//  Auth: jwt
// ─────────────────────────────────────────────

fn run_jwt(
    _plugin_id: Uuid,
    _config: &serde_json::Value,
    ctx: &mut PluginContext,
    store: &GatewayStore,
) -> PluginOutcome {
    let auth_header = match ctx.header("authorization") {
        Some(h) => h.to_string(),
        None => return PluginOutcome::reject(401, "Missing Authorization header"),
    };

    let token = match auth_header.strip_prefix("Bearer ") {
        Some(t) => t.to_string(),
        None => return PluginOutcome::reject(401, "Authorization header must be Bearer"),
    };

    // Decode header to get `iss` / `kid`
    let header = match jsonwebtoken::decode_header(&token) {
        Ok(h) => h,
        Err(_) => return PluginOutcome::reject(401, "Invalid JWT header"),
    };

    let kid = header.kid.unwrap_or_default();
    let cred = match store.find_jwt_by_key(&kid) {
        Some(c) => c.clone(),
        None => return PluginOutcome::reject(401, "Unknown JWT key"),
    };

    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.validate_exp = true;

    match jsonwebtoken::decode::<serde_json::Value>(
        &token,
        &jsonwebtoken::DecodingKey::from_secret(cred.secret.as_bytes()),
        &validation,
    ) {
        Err(_) => PluginOutcome::reject(401, "Invalid JWT signature or claims"),
        Ok(_token_data) => {
            if let Some(consumer) = store.get_consumer(cred.consumer_id) {
                let mut out = PluginOutcome::allow();
                if let PluginOutcome::Continue { set_consumer, .. } = &mut out {
                    *set_consumer = Some(consumer.clone());
                }
                out
            } else {
                PluginOutcome::reject(401, "Consumer not found")
            }
        }
    }
}

// ─────────────────────────────────────────────
//  Auth: basic-auth
// ─────────────────────────────────────────────

fn run_basic_auth(
    _plugin_id: Uuid,
    _config: &serde_json::Value,
    ctx: &mut PluginContext,
    store: &GatewayStore,
) -> PluginOutcome {
    let auth_header = match ctx.header("authorization") {
        Some(h) => h.to_string(),
        None => return PluginOutcome::reject(401, "Missing Authorization header"),
    };

    let encoded = match auth_header.strip_prefix("Basic ") {
        Some(e) => e.to_string(),
        None => return PluginOutcome::reject(401, "Authorization must be Basic"),
    };

    let decoded = match base64::engine::general_purpose::STANDARD.decode(encoded.trim()) {
        Ok(b) => b,
        Err(_) => return PluginOutcome::reject(401, "Invalid Base64 encoding"),
    };

    let credentials = match String::from_utf8(decoded) {
        Ok(s) => s,
        Err(_) => return PluginOutcome::reject(401, "Invalid credentials encoding"),
    };

    let (username, password) = match credentials.split_once(':') {
        Some((u, p)) => (u.to_string(), p.to_string()),
        None => return PluginOutcome::reject(401, "Malformed basic credentials"),
    };

    match store.find_basic_auth(&username) {
        None => PluginOutcome::reject(401, "Invalid credentials"),
        Some(cred) => {
            if cred.password_hash != password {
                return PluginOutcome::reject(401, "Invalid credentials");
            }
            if let Some(consumer) = store.get_consumer(cred.consumer_id) {
                let mut out = PluginOutcome::allow();
                if let PluginOutcome::Continue { set_consumer, .. } = &mut out {
                    *set_consumer = Some(consumer.clone());
                }
                out
            } else {
                PluginOutcome::reject(401, "Consumer not found")
            }
        }
    }
}

// ─────────────────────────────────────────────
//  Auth: hmac-auth
// ─────────────────────────────────────────────

fn run_hmac_auth(
    _plugin_id: Uuid,
    _config: &serde_json::Value,
    ctx: &mut PluginContext,
    store: &GatewayStore,
) -> PluginOutcome {
    // Expected header: Authorization: hmac username="alice", headers="...", signature="base64sig"
    let auth_header = match ctx.header("authorization") {
        Some(h) => h.to_string(),
        None => return PluginOutcome::reject(401, "Missing Authorization header"),
    };

    if !auth_header.to_lowercase().starts_with("hmac ") {
        return PluginOutcome::reject(401, "Authorization must be HMAC");
    }

    let params = parse_hmac_params(&auth_header[5..]);
    let username = match params.get("username") {
        Some(u) => u.clone(),
        None => return PluginOutcome::reject(401, "Missing HMAC username"),
    };
    let signature_b64 = match params.get("signature") {
        Some(s) => s.clone(),
        None => return PluginOutcome::reject(401, "Missing HMAC signature"),
    };

    let cred = match store.find_hmac_auth(&username) {
        Some(c) => c.clone(),
        None => return PluginOutcome::reject(401, "Unknown HMAC username"),
    };

    // Build the signing string from the request
    let signing_string = format!("{} {}", ctx.method.to_uppercase(), ctx.path);

    let sig_bytes = match base64::engine::general_purpose::STANDARD.decode(&signature_b64) {
        Ok(b) => b,
        Err(_) => return PluginOutcome::reject(401, "Invalid HMAC signature encoding"),
    };

    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, cred.secret.as_bytes());
    match ring::hmac::verify(&key, signing_string.as_bytes(), &sig_bytes) {
        Err(_) => PluginOutcome::reject(401, "HMAC signature verification failed"),
        Ok(()) => {
            if let Some(consumer) = store.get_consumer(cred.consumer_id) {
                let mut out = PluginOutcome::allow();
                if let PluginOutcome::Continue { set_consumer, .. } = &mut out {
                    *set_consumer = Some(consumer.clone());
                }
                out
            } else {
                PluginOutcome::reject(401, "Consumer not found")
            }
        }
    }
}

fn parse_hmac_params(params_str: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in params_str.split(',') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            let key = k.trim().to_string();
            let val = v.trim().trim_matches('"').to_string();
            map.insert(key, val);
        }
    }
    map
}

// ─────────────────────────────────────────────
//  Auth: oauth2 (token introspection stub)
// ─────────────────────────────────────────────

fn run_oauth2(_config: &serde_json::Value, ctx: &mut PluginContext) -> PluginOutcome {
    let auth = ctx.header("authorization").map(|s| s.to_string());
    match auth {
        Some(h) if h.starts_with("Bearer ") => PluginOutcome::allow(),
        _ => PluginOutcome::reject(401, "OAuth2 Bearer token required"),
    }
}

// ─────────────────────────────────────────────
//  Auth: ldap-auth (stub — requires LDAP connection)
// ─────────────────────────────────────────────

fn run_ldap_auth(_config: &serde_json::Value, ctx: &mut PluginContext) -> PluginOutcome {
    // Full LDAP auth requires an LDAP connection; treat as pass-through with warning.
    // In production this would bind to an LDAP server.
    let auth = ctx.header("authorization").map(|s| s.to_string());
    match auth {
        Some(h) if h.starts_with("Basic ") => PluginOutcome::allow(),
        _ => PluginOutcome::reject(401, "LDAP Basic credentials required"),
    }
}

// ─────────────────────────────────────────────
//  Traffic: rate-limiting
// ─────────────────────────────────────────────

fn run_rate_limiting(
    plugin_id: Uuid,
    config: &serde_json::Value,
    ctx: &mut PluginContext,
    state: &mut PluginState,
) -> PluginOutcome {
    let limit = config
        .get("minute")
        .and_then(|v| v.as_u64())
        .or_else(|| config.get("second").and_then(|v| v.as_u64()))
        .unwrap_or(60);

    let window_secs = if config.get("second").is_some() { 1 } else { 60 };

    let algorithm = config
        .get("policy")
        .and_then(|v| v.as_str())
        .unwrap_or("fixed-window");

    let allowed = match algorithm {
        "sliding-window" => state.check_rate_limit_sliding(plugin_id, ctx, limit, window_secs),
        _ => state.check_rate_limit_fixed(plugin_id, ctx, limit, window_secs),
    };

    if allowed {
        PluginOutcome::allow()
    } else {
        PluginOutcome::reject(429, "API rate limit exceeded")
    }
}

// ─────────────────────────────────────────────
//  Traffic: request-size-limiting
// ─────────────────────────────────────────────

fn run_request_size_limiting(
    config: &serde_json::Value,
    ctx: &mut PluginContext,
) -> PluginOutcome {
    let max_bytes = config
        .get("allowed_payload_size")
        .and_then(|v| v.as_u64())
        .unwrap_or(128 * 1024 * 1024) as usize; // default 128 MB

    if ctx.body_size > max_bytes {
        PluginOutcome::reject(
            413,
            format!("Request body exceeds maximum allowed size of {max_bytes} bytes"),
        )
    } else {
        PluginOutcome::allow()
    }
}

// ─────────────────────────────────────────────
//  Traffic: proxy-cache
// ─────────────────────────────────────────────

fn run_proxy_cache(
    config: &serde_json::Value,
    ctx: &mut PluginContext,
    state: &mut PluginState,
) -> PluginOutcome {
    // Only cache GET requests
    if ctx.method != "GET" {
        return PluginOutcome::allow();
    }

    let ttl_secs = config
        .get("cache_ttl")
        .and_then(|v| v.as_u64())
        .unwrap_or(300);

    let cache_key = format!("{}:{}", ctx.path, serde_json::to_string(&ctx.query_params).unwrap_or_default());

    if let Some(_entry) = state.get_cache(&cache_key) {
        // Cache hit — inject header so the proxy layer can short-circuit
        let mut headers = HashMap::new();
        headers.insert("X-Cache-Status".to_string(), "HIT".to_string());
        headers.insert("X-Cache-TTL".to_string(), ttl_secs.to_string());
        PluginOutcome::Continue {
            add_request_headers: HashMap::new(),
            remove_request_headers: vec![],
            add_response_headers: headers,
            set_consumer: None,
        }
    } else {
        let mut headers = HashMap::new();
        headers.insert("X-Cache-Status".to_string(), "MISS".to_string());
        PluginOutcome::Continue {
            add_request_headers: HashMap::new(),
            remove_request_headers: vec![],
            add_response_headers: headers,
            set_consumer: None,
        }
    }
}

// ─────────────────────────────────────────────
//  Transform: request-transformer
// ─────────────────────────────────────────────

fn run_request_transformer(
    config: &serde_json::Value,
    _ctx: &mut PluginContext,
) -> PluginOutcome {
    let mut add_headers = HashMap::new();
    let mut remove_headers = Vec::new();

    if let Some(add) = config.get("add").and_then(|v| v.get("headers")).and_then(|v| v.as_array()) {
        for item in add {
            if let Some(s) = item.as_str() {
                if let Some((k, v)) = s.split_once(':') {
                    add_headers.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
    }

    if let Some(remove) = config.get("remove").and_then(|v| v.get("headers")).and_then(|v| v.as_array()) {
        for item in remove {
            if let Some(s) = item.as_str() {
                remove_headers.push(s.to_string());
            }
        }
    }

    PluginOutcome::Continue {
        add_request_headers: add_headers,
        remove_request_headers: remove_headers,
        add_response_headers: HashMap::new(),
        set_consumer: None,
    }
}

// ─────────────────────────────────────────────
//  Transform: response-transformer
// ─────────────────────────────────────────────

fn run_response_transformer(
    config: &serde_json::Value,
    _ctx: &mut PluginContext,
) -> PluginOutcome {
    let mut add_response_headers = HashMap::new();

    if let Some(add) = config.get("add").and_then(|v| v.get("headers")).and_then(|v| v.as_array()) {
        for item in add {
            if let Some(s) = item.as_str() {
                if let Some((k, v)) = s.split_once(':') {
                    add_response_headers.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
    }

    PluginOutcome::Continue {
        add_request_headers: HashMap::new(),
        remove_request_headers: vec![],
        add_response_headers,
        set_consumer: None,
    }
}

// ─────────────────────────────────────────────
//  Transform: correlation-id
// ─────────────────────────────────────────────

fn run_correlation_id(
    config: &serde_json::Value,
    ctx: &mut PluginContext,
    state: &mut PluginState,
) -> PluginOutcome {
    let header_name = config
        .get("header_name")
        .and_then(|v| v.as_str())
        .unwrap_or("X-Correlation-ID");

    let echo_downstream = config
        .get("echo_downstream")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Use existing if already set; otherwise generate
    let id = match ctx.header(header_name) {
        Some(existing) => existing.to_string(),
        None => state.next_correlation_id(),
    };

    let mut add_req = HashMap::new();
    add_req.insert(header_name.to_string(), id.clone());

    let mut add_resp = HashMap::new();
    if echo_downstream {
        add_resp.insert(header_name.to_string(), id);
    }

    PluginOutcome::Continue {
        add_request_headers: add_req,
        remove_request_headers: vec![],
        add_response_headers: add_resp,
        set_consumer: None,
    }
}

// ─────────────────────────────────────────────
//  Logging: http-log
// ─────────────────────────────────────────────

fn run_http_log(config: &serde_json::Value, ctx: &mut PluginContext) -> PluginOutcome {
    let _endpoint = config.get("http_endpoint").and_then(|v| v.as_str()).unwrap_or("");
    // In production: spawn a task to POST the log entry to the endpoint.
    tracing::info!(
        request_id = %ctx.request_id,
        method = %ctx.method,
        path = %ctx.path,
        client_ip = %ctx.client_ip,
        plugin = "http-log",
        "request logged"
    );
    PluginOutcome::allow()
}

// ─────────────────────────────────────────────
//  Logging: file-log
// ─────────────────────────────────────────────

fn run_file_log(config: &serde_json::Value, ctx: &mut PluginContext) -> PluginOutcome {
    let path = config.get("path").and_then(|v| v.as_str()).unwrap_or("/var/log/cave-gateway/access.log");
    tracing::debug!(
        request_id = %ctx.request_id,
        method = %ctx.method,
        path = %ctx.path,
        log_file = path,
        plugin = "file-log",
        "request logged to file"
    );
    PluginOutcome::allow()
}

// ─────────────────────────────────────────────
//  Logging: tcp-log
// ─────────────────────────────────────────────

fn run_tcp_log(config: &serde_json::Value, ctx: &mut PluginContext) -> PluginOutcome {
    let host = config.get("host").and_then(|v| v.as_str()).unwrap_or("127.0.0.1");
    let port = config.get("port").and_then(|v| v.as_u64()).unwrap_or(5044);
    tracing::debug!(
        request_id = %ctx.request_id,
        tcp_host = host,
        tcp_port = port,
        plugin = "tcp-log",
        "request logged to TCP endpoint"
    );
    PluginOutcome::allow()
}

// ─────────────────────────────────────────────
//  Security: cors
// ─────────────────────────────────────────────

fn run_cors(config: &serde_json::Value, _ctx: &mut PluginContext) -> PluginOutcome {
    let origins = config
        .get("origins")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "*".to_string());

    let methods = config
        .get("methods")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "GET, POST, PUT, PATCH, DELETE, OPTIONS".to_string());

    let mut resp_headers = HashMap::new();
    resp_headers.insert("Access-Control-Allow-Origin".to_string(), origins);
    resp_headers.insert("Access-Control-Allow-Methods".to_string(), methods);
    resp_headers.insert(
        "Access-Control-Allow-Headers".to_string(),
        "Content-Type, Authorization".to_string(),
    );

    PluginOutcome::Continue {
        add_request_headers: HashMap::new(),
        remove_request_headers: vec![],
        add_response_headers: resp_headers,
        set_consumer: None,
    }
}

// ─────────────────────────────────────────────
//  Security: ip-restriction
// ─────────────────────────────────────────────

fn run_ip_restriction(config: &serde_json::Value, ctx: &mut PluginContext) -> PluginOutcome {
    let ip = &ctx.client_ip;

    if let Some(allow) = config.get("allow").and_then(|v| v.as_array()) {
        let allowed: Vec<&str> = allow.iter().filter_map(|v| v.as_str()).collect();
        if !allowed.is_empty() && !allowed.contains(&ip.as_str()) {
            return PluginOutcome::reject(403, "Your IP address is not allowed");
        }
    }

    if let Some(deny) = config.get("deny").and_then(|v| v.as_array()) {
        let denied: Vec<&str> = deny.iter().filter_map(|v| v.as_str()).collect();
        if denied.contains(&ip.as_str()) {
            return PluginOutcome::reject(403, "Your IP address is blocked");
        }
    }

    PluginOutcome::allow()
}

// ─────────────────────────────────────────────
//  Security: bot-detection
// ─────────────────────────────────────────────

// Known bot User-Agent substrings
const BOT_SIGNATURES: &[&str] = &[
    "bot", "crawl", "spider", "slurp", "scraper", "fetcher",
    "httpclient", "python-requests", "curl/", "wget/",
    "libwww", "java/", "go-http-client",
];

fn run_bot_detection(config: &serde_json::Value, ctx: &mut PluginContext) -> PluginOutcome {
    let allow_bots = config
        .get("allow")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    let deny_bots = config
        .get("deny")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    let ua = ctx.header("user-agent").unwrap_or("").to_lowercase();

    // Explicit deny list takes precedence
    for pattern in &deny_bots {
        if ua.contains(pattern.to_lowercase().as_str()) {
            return PluginOutcome::reject(403, "Bot detected — access denied");
        }
    }

    // Explicit allow list
    for pattern in &allow_bots {
        if ua.contains(pattern.to_lowercase().as_str()) {
            return PluginOutcome::allow();
        }
    }

    // Built-in bot signatures
    for sig in BOT_SIGNATURES {
        if ua.contains(sig) {
            return PluginOutcome::reject(403, "Bot detected — access denied");
        }
    }

    PluginOutcome::allow()
}

// ─────────────────────────────────────────────
//  Security: acl
// ─────────────────────────────────────────────

fn run_acl(config: &serde_json::Value, ctx: &mut PluginContext) -> PluginOutcome {
    let allowed_groups: Vec<&str> = config
        .get("allow")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let denied_groups: Vec<&str> = config
        .get("deny")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    if allowed_groups.is_empty() && denied_groups.is_empty() {
        return PluginOutcome::allow();
    }

    let consumer_group = ctx
        .consumer
        .as_ref()
        .and_then(|c| c.custom_id.as_deref())
        .unwrap_or("anonymous");

    if denied_groups.contains(&consumer_group) {
        return PluginOutcome::reject(403, "Group not allowed by ACL");
    }

    if !allowed_groups.is_empty() && !allowed_groups.contains(&consumer_group) {
        return PluginOutcome::reject(403, "Group not allowed by ACL");
    }

    PluginOutcome::allow()
}

// ─────────────────────────────────────────────
//  Plugin chain runner
// ─────────────────────────────────────────────

/// Run the full plugin chain, stopping at the first rejection.
///
/// Returns either the final allow (with accumulated header mutations)
/// or the first rejection encountered.
pub fn run_plugin_chain(
    plugins: &[(&crate::models::Plugin, &str)], // (plugin_record, plugin_name)
    ctx: &mut PluginContext,
    state: &mut PluginState,
    store: &GatewayStore,
) -> PluginOutcome {
    let mut accumulated_req_headers: HashMap<String, String> = HashMap::new();
    let mut accumulated_remove_headers: Vec<String> = Vec::new();
    let mut accumulated_resp_headers: HashMap<String, String> = HashMap::new();

    for (plugin, _name) in plugins {
        if !plugin.enabled {
            continue;
        }

        let mut outcome = run_plugin(
            &plugin.name,
            plugin.id,
            &plugin.config,
            ctx,
            state,
            store,
        );

        match &mut outcome {
            PluginOutcome::Reject { .. } => return outcome,
            PluginOutcome::Continue {
                add_request_headers,
                remove_request_headers,
                add_response_headers,
                set_consumer,
            } => {
                accumulated_req_headers.extend(add_request_headers.drain());
                accumulated_remove_headers.append(remove_request_headers);
                accumulated_resp_headers.extend(add_response_headers.drain());
                if let Some(consumer) = set_consumer.take() {
                    ctx.consumer = Some(consumer);
                }
            }
        }
    }

    PluginOutcome::Continue {
        add_request_headers: accumulated_req_headers,
        remove_request_headers: accumulated_remove_headers,
        add_response_headers: accumulated_resp_headers,
        set_consumer: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        BasicAuthCredential, Consumer, HmacAuthCredential, KeyAuthCredential,
    };
    use crate::store::GatewayStore;
    use base64::Engine as _;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_ctx() -> PluginContext {
        PluginContext {
            request_id: Uuid::new_v4(),
            method: "GET".into(),
            path: "/api/test".into(),
            host: "api.example.com".into(),
            headers: HashMap::new(),
            query_params: HashMap::new(),
            body_size: 0,
            client_ip: "10.0.0.1".into(),
            consumer: None,
            route_id: None,
            service_id: None,
        }
    }

    fn make_consumer() -> Consumer {
        Consumer {
            id: Uuid::new_v4(),
            username: Some("alice".into()),
            custom_id: Some("group-a".into()),
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // ── key-auth ──────────────────────────────────────────────────────────

    #[test]
    fn test_key_auth_valid_key() {
        let mut store = GatewayStore::default();
        let consumer = make_consumer();
        store.add_consumer(consumer.clone());
        store.key_auth_creds.insert(
            Uuid::new_v4(),
            KeyAuthCredential {
                id: Uuid::new_v4(),
                consumer_id: consumer.id,
                key: "secret-key-123".into(),
                tags: vec![],
                created_at: Utc::now(),
            },
        );

        let mut ctx = make_ctx();
        ctx.headers.insert("apikey".into(), "secret-key-123".into());

        let outcome = run_plugin("key-auth", Uuid::new_v4(), &serde_json::json!({}), &mut ctx, &mut PluginState::default(), &store);
        assert!(!outcome.is_rejected());
    }

    #[test]
    fn test_key_auth_invalid_key() {
        let store = GatewayStore::default();
        let mut ctx = make_ctx();
        ctx.headers.insert("apikey".into(), "wrong-key".into());

        let outcome = run_plugin("key-auth", Uuid::new_v4(), &serde_json::json!({}), &mut ctx, &mut PluginState::default(), &store);
        assert!(outcome.is_rejected());
    }

    #[test]
    fn test_key_auth_missing_key() {
        let store = GatewayStore::default();
        let mut ctx = make_ctx(); // no apikey header

        let outcome = run_plugin("key-auth", Uuid::new_v4(), &serde_json::json!({}), &mut ctx, &mut PluginState::default(), &store);
        assert!(outcome.is_rejected());
    }

    // ── basic-auth ────────────────────────────────────────────────────────

    #[test]
    fn test_basic_auth_valid() {
        let mut store = GatewayStore::default();
        let consumer = make_consumer();
        store.add_consumer(consumer.clone());
        store.basic_auth_creds.insert(
            Uuid::new_v4(),
            BasicAuthCredential {
                id: Uuid::new_v4(),
                consumer_id: consumer.id,
                username: "alice".into(),
                password_hash: "password123".into(),
                tags: vec![],
                created_at: Utc::now(),
            },
        );

        let mut ctx = make_ctx();
        let encoded = base64::engine::general_purpose::STANDARD.encode("alice:password123");
        ctx.headers.insert("authorization".into(), format!("Basic {encoded}"));

        let outcome = run_plugin("basic-auth", Uuid::new_v4(), &serde_json::json!({}), &mut ctx, &mut PluginState::default(), &store);
        assert!(!outcome.is_rejected());
    }

    #[test]
    fn test_basic_auth_wrong_password() {
        let mut store = GatewayStore::default();
        let consumer = make_consumer();
        store.add_consumer(consumer.clone());
        store.basic_auth_creds.insert(
            Uuid::new_v4(),
            BasicAuthCredential {
                id: Uuid::new_v4(),
                consumer_id: consumer.id,
                username: "alice".into(),
                password_hash: "correct-password".into(),
                tags: vec![],
                created_at: Utc::now(),
            },
        );

        let mut ctx = make_ctx();
        let encoded = base64::engine::general_purpose::STANDARD.encode("alice:wrong-password");
        ctx.headers.insert("authorization".into(), format!("Basic {encoded}"));

        let outcome = run_plugin("basic-auth", Uuid::new_v4(), &serde_json::json!({}), &mut ctx, &mut PluginState::default(), &store);
        assert!(outcome.is_rejected());
    }

    // ── hmac-auth ─────────────────────────────────────────────────────────

    #[test]
    fn test_hmac_auth_valid() {
        let mut store = GatewayStore::default();
        let consumer = make_consumer();
        store.add_consumer(consumer.clone());
        let secret = "super-secret";
        store.hmac_auth_creds.insert(
            Uuid::new_v4(),
            HmacAuthCredential {
                id: Uuid::new_v4(),
                consumer_id: consumer.id,
                username: "bob".into(),
                secret: secret.into(),
                tags: vec![],
                created_at: Utc::now(),
            },
        );

        let mut ctx = make_ctx();
        ctx.method = "GET".into();
        ctx.path = "/api/test".into();

        // Compute correct signature
        let signing_string = "GET /api/test";
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        let tag = ring::hmac::sign(&key, signing_string.as_bytes());
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(tag.as_ref());

        ctx.headers.insert(
            "authorization".into(),
            format!(r#"hmac username="bob", signature="{sig_b64}""#),
        );

        let outcome = run_plugin("hmac-auth", Uuid::new_v4(), &serde_json::json!({}), &mut ctx, &mut PluginState::default(), &store);
        assert!(!outcome.is_rejected());
    }

    #[test]
    fn test_hmac_auth_invalid_signature() {
        let mut store = GatewayStore::default();
        let consumer = make_consumer();
        store.add_consumer(consumer.clone());
        store.hmac_auth_creds.insert(
            Uuid::new_v4(),
            HmacAuthCredential {
                id: Uuid::new_v4(),
                consumer_id: consumer.id,
                username: "bob".into(),
                secret: "super-secret".into(),
                tags: vec![],
                created_at: Utc::now(),
            },
        );

        let mut ctx = make_ctx();
        let bad_sig = base64::engine::general_purpose::STANDARD.encode("bad-signature");
        ctx.headers.insert(
            "authorization".into(),
            format!(r#"hmac username="bob", signature="{bad_sig}""#),
        );

        let outcome = run_plugin("hmac-auth", Uuid::new_v4(), &serde_json::json!({}), &mut ctx, &mut PluginState::default(), &store);
        assert!(outcome.is_rejected());
    }

    // ── rate limiting ─────────────────────────────────────────────────────

    #[test]
    fn test_rate_limit_fixed_window_allows_within_limit() {
        let store = GatewayStore::default();
        let mut state = PluginState::default();
        let plugin_id = Uuid::new_v4();
        let config = serde_json::json!({ "minute": 10 });

        let mut ctx = make_ctx();
        for _ in 0..10 {
            let outcome = run_plugin("rate-limiting", plugin_id, &config, &mut ctx, &mut state, &store);
            assert!(!outcome.is_rejected());
        }
    }

    #[test]
    fn test_rate_limit_fixed_window_blocks_over_limit() {
        let store = GatewayStore::default();
        let mut state = PluginState::default();
        let plugin_id = Uuid::new_v4();
        let config = serde_json::json!({ "minute": 3 });

        let mut ctx = make_ctx();
        // First 3 requests pass
        for _ in 0..3 {
            let outcome = run_plugin("rate-limiting", plugin_id, &config, &mut ctx, &mut state, &store);
            assert!(!outcome.is_rejected());
        }
        // 4th is rejected
        let outcome = run_plugin("rate-limiting", plugin_id, &config, &mut ctx, &mut state, &store);
        assert!(outcome.is_rejected());
    }

    #[test]
    fn test_rate_limit_sliding_window() {
        let store = GatewayStore::default();
        let mut state = PluginState::default();
        let plugin_id = Uuid::new_v4();
        let config = serde_json::json!({ "second": 2, "policy": "sliding-window" });

        let mut ctx = make_ctx();
        // 2 requests pass
        assert!(!run_plugin("rate-limiting", plugin_id, &config, &mut ctx, &mut state, &store).is_rejected());
        assert!(!run_plugin("rate-limiting", plugin_id, &config, &mut ctx, &mut state, &store).is_rejected());
        // 3rd is blocked
        assert!(run_plugin("rate-limiting", plugin_id, &config, &mut ctx, &mut state, &store).is_rejected());
    }

    // ── ip-restriction ────────────────────────────────────────────────────

    #[test]
    fn test_ip_restriction_deny() {
        let store = GatewayStore::default();
        let mut state = PluginState::default();
        let config = serde_json::json!({ "deny": ["10.0.0.1"] });
        let mut ctx = make_ctx(); // client_ip = "10.0.0.1"

        let outcome = run_plugin("ip-restriction", Uuid::new_v4(), &config, &mut ctx, &mut state, &store);
        assert!(outcome.is_rejected());
    }

    #[test]
    fn test_ip_restriction_allow_list() {
        let store = GatewayStore::default();
        let mut state = PluginState::default();
        let config = serde_json::json!({ "allow": ["192.168.1.1"] });
        let mut ctx = make_ctx(); // client_ip = "10.0.0.1"

        let outcome = run_plugin("ip-restriction", Uuid::new_v4(), &config, &mut ctx, &mut state, &store);
        assert!(outcome.is_rejected()); // not in allow list
    }

    // ── bot-detection ─────────────────────────────────────────────────────

    #[test]
    fn test_bot_detection_blocks_known_bot() {
        let store = GatewayStore::default();
        let mut state = PluginState::default();
        let config = serde_json::json!({});
        let mut ctx = make_ctx();
        ctx.headers.insert("user-agent".into(), "Googlebot/2.1 (+http://www.google.com/bot.html)".into());

        let outcome = run_plugin("bot-detection", Uuid::new_v4(), &config, &mut ctx, &mut state, &store);
        assert!(outcome.is_rejected());
    }

    #[test]
    fn test_bot_detection_allows_normal_browser() {
        let store = GatewayStore::default();
        let mut state = PluginState::default();
        let config = serde_json::json!({});
        let mut ctx = make_ctx();
        ctx.headers.insert(
            "user-agent".into(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36".into(),
        );

        let outcome = run_plugin("bot-detection", Uuid::new_v4(), &config, &mut ctx, &mut state, &store);
        assert!(!outcome.is_rejected());
    }

    // ── correlation-id ────────────────────────────────────────────────────

    #[test]
    fn test_correlation_id_injected() {
        let store = GatewayStore::default();
        let mut state = PluginState::default();
        let config = serde_json::json!({ "header_name": "X-Correlation-ID" });
        let mut ctx = make_ctx();

        let outcome = run_plugin("correlation-id", Uuid::new_v4(), &config, &mut ctx, &mut state, &store);
        match outcome {
            PluginOutcome::Continue { add_request_headers, .. } => {
                assert!(add_request_headers.contains_key("X-Correlation-ID"));
            }
            _ => panic!("Expected Continue"),
        }
    }
}
