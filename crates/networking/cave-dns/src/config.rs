// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use serde::{Deserialize, Serialize};

// ─── Top-level server config ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsConfig {
    /// UDP listen addresses (default ["0.0.0.0:53"])
    pub listen_udp: Vec<String>,
    /// TCP listen addresses
    pub listen_tcp: Vec<String>,
    /// DNS-over-TLS listen addresses (port 853)
    pub dot_listen: Vec<String>,
    /// DNS-over-HTTPS listen addresses
    pub doh_listen: Vec<String>,

    /// Path to TLS certificate (PEM)
    pub tls_cert_path: Option<String>,
    /// Path to TLS private key (PEM)
    pub tls_key_path: Option<String>,

    /// Maximum UDP payload without EDNS
    pub max_udp_size: u16,
    /// EDNS advertised buffer size
    pub edns_buf_size: u16,

    /// Ordered plugin chain
    pub plugins: Vec<PluginConfig>,
    /// Zones to load at startup
    pub zones: Vec<ZoneConfig>,

    /// HTTP API / health / metrics listen address
    pub api_listen: String,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            listen_udp: vec!["0.0.0.0:53".into()],
            listen_tcp: vec!["0.0.0.0:53".into()],
            dot_listen: vec![],
            doh_listen: vec![],
            tls_cert_path: None,
            tls_key_path: None,
            max_udp_size: 512,
            edns_buf_size: 4096,
            plugins: vec![],
            zones: vec![],
            api_listen: "0.0.0.0:8053".into(),
        }
    }
}

impl DnsConfig {
    /// Build a [`DnsConfig`] from a native Corefile, bridging the
    /// [`crate::corefile`] parser into the live config type.
    ///
    /// The first server block's address key supplies the listen port (the
    /// `:port` suffix, defaulting to 53), and its directives are mapped onto
    /// the [`PluginConfig`] chain. `{$VAR}` references in address keys are
    /// resolved via the process environment by [`crate::corefile::parse`].
    /// Directives without a config mapping yet are ignored (Phase 2).
    pub fn from_corefile(input: &str) -> Result<Self, crate::corefile::ParseError> {
        let blocks = crate::corefile::parse(input)?;
        let mut cfg = DnsConfig::default();

        if let Some(block) = blocks.first() {
            // Port from the first key's ":port" suffix (default 53).
            let port = block
                .keys
                .first()
                .and_then(|k| k.rsplit_once(':'))
                .and_then(|(_, p)| p.parse::<u16>().ok())
                .unwrap_or(53);
            let listen = format!("0.0.0.0:{port}");
            cfg.listen_udp = vec![listen.clone()];
            cfg.listen_tcp = vec![listen];

            // Map directives onto the plugin chain.
            cfg.plugins = block
                .tokens
                .values()
                .filter_map(|toks| plugin_from_tokens(toks))
                .collect();
        }
        Ok(cfg)
    }
}

/// Translate one directive's token slice (directive name first) into a
/// [`PluginConfig`]. Returns `None` for directives without a mapping yet.
fn plugin_from_tokens(toks: &[crate::corefile::Token]) -> Option<PluginConfig> {
    let name = toks.first()?.text.as_str();
    let args: Vec<&str> = toks[1..].iter().map(|t| t.text.as_str()).collect();
    Some(match name {
        "whoami" => PluginConfig::Whoami,
        "forward" => {
            // forward FROM TO...  — drop the FROM zone, keep upstreams.
            let upstreams = args.iter().skip(1).map(|s| s.to_string()).collect();
            PluginConfig::Forward(ForwardConfig { upstreams, ..Default::default() })
        }
        "cache" => PluginConfig::Cache(CacheConfig::default()),
        "errors" => PluginConfig::Errors(ErrorsConfig::default()),
        "log" => PluginConfig::Log(LogConfig::default()),
        "health" => PluginConfig::Health(HealthConfig::default()),
        "ready" => PluginConfig::Ready(ReadyConfig::default()),
        "metrics" | "prometheus" => PluginConfig::Metrics(MetricsConfig::default()),
        "loadbalance" => PluginConfig::Loadbalance(LoadbalanceConfig::default()),
        "loop" => PluginConfig::Loop(LoopConfig::default()),
        "reload" => PluginConfig::Reload(ReloadConfig::default()),
        "chaos" => PluginConfig::Chaos(ChaosConfig::default()),
        "any" => PluginConfig::Any(AnyConfig::default()),
        _ => return None,
    })
}

// ─── Zone config ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneConfig {
    pub name: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub zone_type: ZoneType,
    /// Master addresses for secondary zones
    #[serde(default)]
    pub masters: Vec<String>,
    /// TSIG key name for zone transfers
    #[serde(default)]
    pub tsig_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ZoneType {
    #[default]
    Primary,
    Secondary,
    Hint,
}

// ─── Plugin configs ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "name", rename_all = "snake_case")]
pub enum PluginConfig {
    Forward(ForwardConfig),
    Cache(CacheConfig),
    File(FilePluginConfig),
    Auto(AutoConfig),
    Hosts(HostsConfig),
    Kubernetes(KubernetesConfig),
    Rewrite(RewriteConfig),
    Template(TemplateConfig),
    Errors(ErrorsConfig),
    Log(LogConfig),
    Health(HealthConfig),
    Ready(ReadyConfig),
    Metrics(MetricsConfig),
    Loadbalance(LoadbalanceConfig),
    Loop(LoopConfig),
    Reload(ReloadConfig),
    Whoami,
    Chaos(ChaosConfig),
    Any(AnyConfig),
    Acl(AclConfig),
    Secondary(SecondaryConfig),
    Etcd(EtcdConfig),
    Route53(Route53Config),
}

// ─── Per-plugin configs ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ForwardConfig {
    pub upstreams: Vec<String>,
    pub policy: ForwardPolicy,
    /// Seconds between health checks
    pub health_check_interval: u64,
    /// Failures before marking upstream unhealthy
    pub max_fails: u32,
    /// Seconds an upstream stays out of rotation after failure
    pub expire: u64,
    /// Forward query timeout in milliseconds
    pub timeout_ms: u64,
    /// Maximum concurrent queries per upstream
    pub max_concurrent: usize,
}

impl Default for ForwardConfig {
    fn default() -> Self {
        Self {
            upstreams: vec!["8.8.8.8:53".into(), "8.8.4.4:53".into()],
            policy: ForwardPolicy::Random,
            health_check_interval: 0,
            max_fails: 2,
            expire: 10,
            timeout_ms: 5000,
            max_concurrent: 1000,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardPolicy {
    #[default]
    Random,
    RoundRobin,
    Sequential,
}

// ─── Cache ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    pub max_ttl: u32,
    pub min_ttl: u32,
    /// Negative (NXDOMAIN) cache TTL
    pub neg_ttl: u32,
    /// Maximum entries
    pub capacity: usize,
    /// Background prefetch when TTL < 10 %
    pub prefetch: bool,
    /// Serve stale while refreshing
    pub serve_stale: bool,
    /// How long past expiry to serve stale (seconds)
    pub stale_ttl: u32,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_ttl: 3600,
            min_ttl: 0,
            neg_ttl: 900,
            capacity: 10_000,
            prefetch: false,
            serve_stale: false,
            stale_ttl: 3600,
        }
    }
}

// ─── File plugin ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FilePluginConfig {
    pub zones: Vec<String>,
    pub reload_interval: Option<u64>,
}

// ─── Auto plugin ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoConfig {
    pub directory: String,
    pub template: String,
}

impl Default for AutoConfig {
    fn default() -> Self {
        Self {
            directory: "/etc/coredns/zones".into(),
            template: "*.zone".into(),
        }
    }
}

// ─── Hosts plugin ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HostsConfig {
    pub path: Option<String>,
    pub reload_period: u64,
    /// Extra inline entries
    pub inline: Vec<String>,
    pub ttl: u32,
    pub fallthrough: bool,
}

impl Default for HostsConfig {
    fn default() -> Self {
        Self {
            path: None,
            reload_period: 5,
            inline: vec![],
            ttl: 3600,
            fallthrough: false,
        }
    }
}

// ─── Kubernetes plugin ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KubernetesConfig {
    pub zones: Vec<String>,
    pub endpoint: Option<String>,
    pub kubeconfig: Option<String>,
    pub namespaces: Vec<String>,
    pub pods: PodMode,
    pub external_names: bool,
    pub ttl: u32,
    pub cluster_domain: String,
}

impl Default for KubernetesConfig {
    fn default() -> Self {
        Self {
            zones: vec!["cluster.local.".into()],
            endpoint: None,
            kubeconfig: None,
            namespaces: vec![],
            pods: PodMode::Disabled,
            external_names: false,
            ttl: 5,
            cluster_domain: "cluster.local".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodMode {
    #[default]
    Disabled,
    Insecure,
    Verified,
}

// ─── Rewrite plugin ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RewriteConfig {
    pub rules: Vec<RewriteRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewriteRule {
    pub match_type: MatchType,
    pub from: String,
    pub to: String,
    pub action: RewriteAction,
    #[serde(default)]
    pub continue_on_match: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    #[default]
    Exact,
    Prefix,
    Suffix,
    Regex,
    Substring,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RewriteAction {
    #[default]
    Name,
    Type,
    Class,
    Ttl,
}

// ─── Template plugin ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateConfig {
    pub templates: Vec<TemplateRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateRule {
    pub match_regex: String,
    pub qtype: String,
    #[serde(default)]
    pub answer: Vec<String>,
    #[serde(default)]
    pub authority: Vec<String>,
    #[serde(default)]
    pub additional: Vec<String>,
    #[serde(default = "default_rcode")]
    pub rcode: String,
    #[serde(default)]
    pub fall_through: bool,
}

fn default_rcode() -> String {
    "NOERROR".into()
}

// ─── Errors plugin ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ErrorsConfig {
    pub log_format: String,
    pub consolidate: bool,
}

impl Default for ErrorsConfig {
    fn default() -> Self {
        Self {
            log_format: "{time} {type} {class} {name} {rcode}".into(),
            consolidate: false,
        }
    }
}

// ─── Log plugin ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub format: String,
    pub class_filter: Vec<String>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            format: "{remote} - {>id} \"{type} {class} {name} {proto} {size} {>do} {>bufsize}\" {rcode} {>ttl} {latency}".into(),
            class_filter: vec![],
        }
    }
}

// ─── Health/Ready/Metrics ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthConfig {
    pub addr: String,
    pub path: String,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            addr: "0.0.0.0:8080".into(),
            path: "/health".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadyConfig {
    pub addr: String,
    pub path: String,
}

impl Default for ReadyConfig {
    fn default() -> Self {
        Self {
            addr: "0.0.0.0:8181".into(),
            path: "/ready".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    pub addr: String,
    pub path: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            addr: "0.0.0.0:9153".into(),
            path: "/metrics".into(),
        }
    }
}

// ─── Loadbalance ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoadbalanceConfig {
    pub policy: LbPolicy,
}

impl Default for LoadbalanceConfig {
    fn default() -> Self {
        Self {
            policy: LbPolicy::RoundRobin,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LbPolicy {
    #[default]
    RoundRobin,
    Random,
    Weighted,
}

// ─── Loop detection ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoopConfig {
    pub timeout_ms: u64,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self { timeout_ms: 2000 }
    }
}

// ─── Reload plugin ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReloadConfig {
    pub interval_secs: u64,
}

impl Default for ReloadConfig {
    fn default() -> Self {
        Self { interval_secs: 30 }
    }
}

// ─── Chaos plugin ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChaosConfig {
    pub version: String,
    pub hostname: String,
}

impl Default for ChaosConfig {
    fn default() -> Self {
        Self {
            version: "cave-dns 0.1.0".into(),
            hostname: hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "cave-dns".into()),
        }
    }
}

mod hostname {
    pub fn get() -> std::io::Result<std::ffi::OsString> {
        std::env::var_os("HOSTNAME").map_or_else(
            || {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no HOSTNAME",
                ))
            },
            Ok,
        )
    }
}

// ─── Any plugin ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnyConfig {
    pub response: AnyResponse,
}

impl Default for AnyConfig {
    fn default() -> Self {
        Self {
            response: AnyResponse::Minimal,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnyResponse {
    #[default]
    Minimal,
    Refuse,
    All,
}

// ─── ACL ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AclConfig {
    pub rules: Vec<AclRule>,
    pub default_action: AclAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclRule {
    pub action: AclAction,
    #[serde(default)]
    pub source: Vec<String>,
    #[serde(default)]
    pub zones: Vec<String>,
    #[serde(default)]
    pub types: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AclAction {
    #[default]
    Allow,
    Deny,
}

// ─── Secondary ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SecondaryConfig {
    pub zones: Vec<SecondaryZoneConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecondaryZoneConfig {
    pub name: String,
    pub masters: Vec<String>,
    pub tsig_key: Option<String>,
    #[serde(default = "default_refresh")]
    pub refresh_interval: u64,
}

fn default_refresh() -> u64 {
    300
}

// ─── Etcd ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EtcdConfig {
    pub endpoints: Vec<String>,
    pub prefix: String,
    pub timeout_ms: u64,
    pub credentials: Option<EtcdCredentials>,
}

impl Default for EtcdConfig {
    fn default() -> Self {
        Self {
            endpoints: vec!["http://localhost:2379".into()],
            prefix: "/skydns".into(),
            timeout_ms: 5000,
            credentials: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtcdCredentials {
    pub username: String,
    pub password: String,
}

// ─── Route53 ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Route53Config {
    pub zones: Vec<String>,
    pub region: String,
    pub aws_access_key: Option<String>,
    pub aws_secret_key: Option<String>,
    pub refresh_secs: u64,
}

impl Default for Route53Config {
    fn default() -> Self {
        Self {
            zones: vec![],
            region: "us-east-1".into(),
            aws_access_key: None,
            aws_secret_key: None,
            refresh_secs: 300,
        }
    }
}

#[cfg(test)]
mod corefile_config_tests {
    use super::*;

    #[test]
    fn from_corefile_sets_port_from_server_key() {
        let cfg = DnsConfig::from_corefile(".:1053 {\n    whoami\n}\n").expect("ok");
        assert_eq!(cfg.listen_udp, vec!["0.0.0.0:1053".to_string()]);
        assert_eq!(cfg.listen_tcp, vec!["0.0.0.0:1053".to_string()]);
    }

    #[test]
    fn from_corefile_keyless_port_defaults_to_53() {
        let cfg = DnsConfig::from_corefile("example.com {\n    whoami\n}\n").expect("ok");
        assert_eq!(cfg.listen_udp, vec!["0.0.0.0:53".to_string()]);
    }

    #[test]
    fn from_corefile_builds_whoami_and_forward_plugins() {
        let cfg = DnsConfig::from_corefile(
            ".:53 {\n    whoami\n    forward . 1.1.1.1 8.8.8.8\n}\n",
        )
        .expect("ok");
        // Plugin chain order is registration-defined upstream, not Corefile
        // order, so assert by presence rather than index.
        assert!(cfg.plugins.iter().any(|p| matches!(p, PluginConfig::Whoami)));
        let fwd = cfg
            .plugins
            .iter()
            .find_map(|p| match p {
                PluginConfig::Forward(f) => Some(f),
                _ => None,
            })
            .expect("forward plugin present");
        // The FROM zone (".") is dropped; the rest are upstreams.
        assert_eq!(fwd.upstreams, vec!["1.1.1.1".to_string(), "8.8.8.8".to_string()]);
    }

    #[test]
    fn from_corefile_resolves_env_in_port() {
        // {$VAR} substitution flows through parse() into the listen port.
        // SAFETY: single-threaded test process for this var; restored after.
        unsafe { std::env::set_var("CAVE_DNS_CFG_PORT", "5300") };
        let cfg = DnsConfig::from_corefile(".:{$CAVE_DNS_CFG_PORT} {\n    whoami\n}\n")
            .expect("ok");
        unsafe { std::env::remove_var("CAVE_DNS_CFG_PORT") };
        assert_eq!(cfg.listen_udp, vec!["0.0.0.0:5300".to_string()]);
    }
}
