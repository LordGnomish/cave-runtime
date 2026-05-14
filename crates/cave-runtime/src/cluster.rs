// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-runtime cluster` subcommands: `init`, `join`, `status`, `destroy`.
//!
//! Cluster lifecycle plumbing — provisions data dir, PKI hierarchy
//! (root CA + per-component leaves), kubeconfig, RBAC, and default
//! namespaces. The actual control-plane processes are started by
//! `cave-runtime serve` against the same data dir.

use anyhow::{anyhow, Context, Result};
use clap::Subcommand;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, SanType,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration as StdDuration;
use time::{Duration, OffsetDateTime};
use tracing::{info, warn};

/// Cluster lifecycle subcommands.
#[derive(Subcommand, Debug, Clone)]
pub enum ClusterCmd {
    /// Initialize a new Cave cluster. Defaults to single-node;
    /// `--bootstrap-strategy=multi` plus `--peers=...` declares the
    /// initial multi-node membership.
    Init {
        #[arg(long, default_value = "/var/lib/cave")]
        data_dir: PathBuf,
        #[arg(long, default_value = "cave-local")]
        cluster_name: String,
        #[arg(long, default_value = "127.0.0.1:6443")]
        advertise_address: String,
        /// Bootstrap strategy: `single` (default, one node) or `multi`.
        #[arg(long, default_value = "single")]
        bootstrap_strategy: String,
        /// Comma-separated peer list, each shaped as `<node_id>:<advertise>`.
        /// Example: `2:10.0.0.2:6443,3:10.0.0.3:6443`. Empty for single-node.
        #[arg(long, default_value = "")]
        peers: String,
        /// Local node id within the cluster. Defaults to 1.
        #[arg(long, default_value = "1")]
        node_id: u64,
        /// When set, reuse `pki/ca.{crt,key}` already present in the
        /// data dir (signed by an earlier init or copied from a peer)
        /// instead of generating a fresh CA. Used by the 3-node bash
        /// smoke + by operators bootstrapping co-resident nodes; the
        /// production join flow uses `cluster join` to fetch the
        /// leader's CA via TOFU.
        #[arg(long, default_value_t = false)]
        reuse_existing_ca: bool,
    },
    /// Worker-node join: POST the bootstrap token to the master apiserver and
    /// persist the returned join config + issued kubelet certificate.
    Join {
        #[arg(long)]
        bootstrap_token: String,
        #[arg(long, help = "Master apiserver, e.g. https://10.0.0.1:6443")]
        master_address: String,
        #[arg(long, default_value = "/var/lib/cave")]
        data_dir: PathBuf,
        #[arg(long, default_value = "")]
        node_name: String,
        /// Path to the master's CA certificate (PEM). If omitted, the master's
        /// CA is fetched once over an unverified TLS connection (TOFU) and
        /// cached at `<data_dir>/master-ca.crt` — subsequent joins to the same
        /// data dir then pin against the cached copy.
        #[arg(long)]
        ca_bundle: Option<PathBuf>,
    },
    /// Cluster health: parse kubeconfig and probe the control-plane.
    Status {
        #[arg(long)]
        kubeconfig: PathBuf,
    },
    /// Tear down a local cluster data dir. Requires `--force`.
    Destroy {
        #[arg(long, default_value = "/var/lib/cave")]
        data_dir: PathBuf,
        #[arg(long)]
        force: bool,
    },
}

const COMPONENTS: &[&str] = &[
    "apiserver",
    "etcd",
    "kubelet",
    "scheduler",
    "controller-manager",
    "admin",
];

/// Manifest written to `<data_dir>/cluster.json` summarizing the init.
///
/// `bootstrap_strategy` and `peers` are forward-looking fields so the file
/// shape doesn't need to change when multi-node Raft lands. Today every
/// init writes `bootstrap_strategy: "single"` and an empty peer list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterManifest {
    pub cluster_name: String,
    pub advertise_address: String,
    pub created_at: String,
    pub pki_root_cn: String,
    pub components: Vec<String>,
    pub kubeconfig_path: String,
    pub data_dir: String,
    #[serde(default = "default_bootstrap_strategy")]
    pub bootstrap_strategy: String,
    /// Peer entries shaped as `"<node_id>:<advertise_url>"`, e.g.
    /// `"2:https://10.0.0.2:6443"`. The peer transport parses these on
    /// startup. Empty by default; populated by `cluster init` when
    /// `--peers ...` is supplied or appended by `cluster join` on the
    /// master.
    #[serde(default)]
    pub peers: Vec<String>,
    /// Stable identifier for the local node within the Raft cluster.
    /// `None` (the default) implies single-node mode and the registry
    /// uses `1`.
    #[serde(default)]
    pub local_node_id: Option<u64>,
}

fn default_bootstrap_strategy() -> String {
    "single".into()
}

pub async fn dispatch(cmd: ClusterCmd) -> Result<()> {
    match cmd {
        ClusterCmd::Init {
            data_dir,
            cluster_name,
            advertise_address,
            bootstrap_strategy,
            peers,
            node_id,
            reuse_existing_ca,
        } => init(
            &data_dir,
            &cluster_name,
            &advertise_address,
            &bootstrap_strategy,
            &peers,
            node_id,
            reuse_existing_ca,
        ),
        ClusterCmd::Join {
            bootstrap_token,
            master_address,
            data_dir,
            node_name,
            ca_bundle,
        } => {
            join(
                &data_dir,
                &bootstrap_token,
                &master_address,
                &node_name,
                ca_bundle.as_deref(),
            )
            .await
        }
        ClusterCmd::Status { kubeconfig } => status(&kubeconfig).await,
        ClusterCmd::Destroy { data_dir, force } => destroy(&data_dir, force),
    }
}

/// `cluster init` — provision a fresh single- or multi-node cluster on disk.
pub fn init(
    data_dir: &Path,
    cluster_name: &str,
    advertise_address: &str,
    bootstrap_strategy: &str,
    peers_csv: &str,
    node_id: u64,
    reuse_existing_ca: bool,
) -> Result<()> {
    info!(?data_dir, %cluster_name, %advertise_address, reuse_existing_ca, "cluster init");

    if data_dir.join("cluster.json").exists() {
        return Err(anyhow!(
            "{} is already initialized (cluster.json exists). Use `destroy --force` first.",
            data_dir.display()
        ));
    }

    let pki_dir = data_dir.join("pki");
    let etcd_dir = data_dir.join("etcd");
    let kubeconfig_dir = data_dir.join("kubeconfig");
    let manifests_dir = data_dir.join("manifests");
    for d in [&pki_dir, &etcd_dir, &kubeconfig_dir, &manifests_dir] {
        fs::create_dir_all(d).with_context(|| format!("create {}", d.display()))?;
    }

    // CA path: when `reuse_existing_ca` is set and `pki/ca.{crt,key}`
    // are already on disk, parse them into rcgen primitives and skip
    // root-CA generation. The new leaf certs are then signed by the
    // *existing* CA, which is the precondition for cross-node TLS to
    // validate in a multi-node setup (each peer must trust a common
    // root). When the flag is unset, behaviour is unchanged.
    let root_cn = format!("cave-runtime root CA ({})", cluster_name);
    let (ca_cert_pem, ca_key_pem, ca_cert, ca_key) = if reuse_existing_ca
        && pki_dir.join("ca.crt").exists()
        && pki_dir.join("ca.key").exists()
    {
        let ca_cert_pem = fs::read_to_string(pki_dir.join("ca.crt"))
            .with_context(|| format!("read {}", pki_dir.join("ca.crt").display()))?;
        let ca_key_pem = fs::read_to_string(pki_dir.join("ca.key"))
            .with_context(|| format!("read {}", pki_dir.join("ca.key").display()))?;
        let ca_key = KeyPair::from_pem(&ca_key_pem)
            .map_err(|e| anyhow!("parse existing ca.key: {e}"))?;
        let params = CertificateParams::from_ca_cert_pem(&ca_cert_pem)
            .map_err(|e| anyhow!("parse existing ca.crt: {e}"))?;
        let ca_cert = params
            .self_signed(&ca_key)
            .map_err(|e| anyhow!("re-derive existing CA: {e}"))?;
        info!("cluster init: reusing existing CA from pki/ca.crt");
        (ca_cert_pem, ca_key_pem, ca_cert, ca_key)
    } else {
        let g = generate_root_ca(&root_cn)?;
        fs::write(pki_dir.join("ca.crt"), &g.0)?;
        fs::write(pki_dir.join("ca.key"), &g.1)?;
        g
    };

    let advertise_host = advertise_address
        .split(':')
        .next()
        .unwrap_or("127.0.0.1")
        .to_string();
    for component in COMPONENTS {
        let (leaf_cert, leaf_key) =
            generate_leaf(&ca_cert, &ca_key, component, cluster_name, &advertise_host)?;
        fs::write(pki_dir.join(format!("{}.crt", component)), &leaf_cert)?;
        fs::write(pki_dir.join(format!("{}.key", component)), &leaf_key)?;
    }

    let sa_key = KeyPair::generate().map_err(|e| anyhow!("sa keypair: {e}"))?;
    fs::write(pki_dir.join("sa.key"), sa_key.serialize_pem())?;
    fs::write(pki_dir.join("sa.pub"), sa_key.public_key_pem())?;

    let admin_crt = fs::read_to_string(pki_dir.join("admin.crt"))?;
    let admin_key = fs::read_to_string(pki_dir.join("admin.key"))?;
    let kubeconfig_path = kubeconfig_dir.join("admin.kubeconfig");
    let kubeconfig = render_kubeconfig(
        cluster_name,
        advertise_address,
        &ca_cert_pem,
        &admin_crt,
        &admin_key,
    );
    fs::write(&kubeconfig_path, kubeconfig)?;

    fs::write(manifests_dir.join("rbac.yaml"), default_rbac_yaml())?;
    fs::write(manifests_dir.join("namespaces.yaml"), default_namespaces_yaml())?;

    // Mint a bootstrap token so workers can join.  Stored as a JSON file the
    // production-mode apiserver listener loads on startup.
    let bootstrap_token = mint_bootstrap_token();
    let token_file = serde_json::json!({
        "tokens": [{
            "token": bootstrap_token,
            "created_at": OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| String::from("unknown")),
        }]
    });
    fs::write(
        data_dir.join("bootstrap-tokens.json"),
        serde_json::to_string_pretty(&token_file)?,
    )?;
    // etcd snapshot dir — serve restores from `snapshot.bin` if it exists.
    fs::create_dir_all(&etcd_dir)?;

    let parsed_peers: Vec<String> = peers_csv
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    if !matches!(bootstrap_strategy, "single" | "multi") {
        return Err(anyhow!(
            "--bootstrap-strategy must be `single` or `multi`, got `{}`",
            bootstrap_strategy
        ));
    }
    if bootstrap_strategy == "multi" && parsed_peers.is_empty() {
        return Err(anyhow!(
            "--bootstrap-strategy=multi requires at least one `--peers <id:url>` entry"
        ));
    }

    let manifest = ClusterManifest {
        cluster_name: cluster_name.to_string(),
        advertise_address: advertise_address.to_string(),
        created_at: OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| String::from("unknown")),
        pki_root_cn: root_cn,
        components: COMPONENTS.iter().map(|s| s.to_string()).collect(),
        kubeconfig_path: kubeconfig_path.display().to_string(),
        data_dir: data_dir.display().to_string(),
        bootstrap_strategy: bootstrap_strategy.to_string(),
        peers: parsed_peers,
        local_node_id: Some(node_id),
    };
    fs::write(
        data_dir.join("cluster.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;

    println!("Cave cluster initialized.");
    println!("  data dir:     {}", data_dir.display());
    println!("  cluster name: {}", cluster_name);
    println!("  advertise:    {}", advertise_address);
    println!("  components:   {}", COMPONENTS.join(", "));
    println!("  kubeconfig:   {}", kubeconfig_path.display());
    println!("  bootstrap strategy: {}", bootstrap_strategy);
    println!("  local node id:      {}", node_id);
    if !manifest.peers.is_empty() {
        println!("  peers:");
        for p in &manifest.peers {
            println!("    - {}", p);
        }
    }
    println!("  bootstrap token (for worker joins):");
    println!("    {}", bootstrap_token);
    println!();
    println!("Next: cave-runtime --data-dir {} serve", data_dir.display());
    println!("  └─ cave-etcd will listen on https://{}:2379", advertise_host);
    println!("  └─ cave-apiserver will listen on https://{}", advertise_address);
    Ok(())
}

fn mint_bootstrap_token() -> String {
    // 32 hex chars (128 bits) is plenty for a bootstrap token.  Source from
    // the OS RNG via uuid v4 (already a dependency) and strip dashes.
    uuid::Uuid::new_v4().simple().to_string()
}

fn generate_root_ca(common_name: &str) -> Result<(String, String, rcgen::Certificate, KeyPair)> {
    let key = KeyPair::generate().map_err(|e| anyhow!("ca keypair: {e}"))?;
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name.push(DnType::CommonName, common_name);
    params.distinguished_name.push(DnType::OrganizationName, "cave-runtime");
    params.not_before = OffsetDateTime::now_utc();
    params.not_after = OffsetDateTime::now_utc() + Duration::days(3650);
    params.key_usages.push(KeyUsagePurpose::KeyCertSign);
    params.key_usages.push(KeyUsagePurpose::CrlSign);
    let cert = params.self_signed(&key).map_err(|e| anyhow!("ca self_signed: {e}"))?;
    Ok((cert.pem(), key.serialize_pem(), cert, key))
}

fn generate_leaf(
    ca_cert: &rcgen::Certificate,
    ca_key: &KeyPair,
    component: &str,
    cluster_name: &str,
    advertise_host: &str,
) -> Result<(String, String)> {
    let key = KeyPair::generate().map_err(|e| anyhow!("{component} keypair: {e}"))?;
    let mut params = CertificateParams::default();
    let cn = format!("system:{}", component);
    params.distinguished_name.push(DnType::CommonName, &cn);
    params
        .distinguished_name
        .push(DnType::OrganizationName, format!("cave-cluster:{}", cluster_name));
    params.not_before = OffsetDateTime::now_utc();
    params.not_after = OffsetDateTime::now_utc() + Duration::days(365);
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params.key_usages.push(KeyUsagePurpose::KeyEncipherment);
    // Server-style components get DNS + IP SANs for the advertise host
    // plus the ServerAuth EKU. Without an EKU, rustls (correctly) refuses
    // the cert when a chain-validating client pins the CA.
    if matches!(component, "apiserver" | "etcd" | "kubelet") {
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ServerAuth);
        // kubelet also acts as a client to the apiserver.
        if component == "kubelet" {
            params
                .extended_key_usages
                .push(ExtendedKeyUsagePurpose::ClientAuth);
        }
        if let Ok(ia5) = rcgen::Ia5String::try_from(advertise_host.to_string()) {
            params.subject_alt_names.push(SanType::DnsName(ia5));
        }
        if let Ok(ip) = advertise_host.parse() {
            params.subject_alt_names.push(SanType::IpAddress(ip));
        }
        if let Ok(localhost) = rcgen::Ia5String::try_from("localhost".to_string()) {
            params.subject_alt_names.push(SanType::DnsName(localhost));
        }
    } else {
        // Client-style components (admin, scheduler, controller-manager)
        // present client certs against the apiserver.
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ClientAuth);
    }
    let cert = params
        .signed_by(&key, ca_cert, ca_key)
        .map_err(|e| anyhow!("{component} sign: {e}"))?;
    Ok((cert.pem(), key.serialize_pem()))
}

pub fn render_kubeconfig(
    cluster_name: &str,
    advertise_address: &str,
    ca_pem: &str,
    admin_crt_pem: &str,
    admin_key_pem: &str,
) -> String {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD;
    format!(
        "apiVersion: v1\n\
         kind: Config\n\
         current-context: {cluster}-admin@{cluster}\n\
         clusters:\n\
         - name: {cluster}\n  cluster:\n    server: https://{addr}\n    certificate-authority-data: {ca}\n\
         contexts:\n\
         - name: {cluster}-admin@{cluster}\n  context:\n    cluster: {cluster}\n    user: {cluster}-admin\n\
         users:\n\
         - name: {cluster}-admin\n  user:\n    client-certificate-data: {crt}\n    client-key-data: {key}\n",
        cluster = cluster_name,
        addr = advertise_address,
        ca = b64.encode(ca_pem.as_bytes()),
        crt = b64.encode(admin_crt_pem.as_bytes()),
        key = b64.encode(admin_key_pem.as_bytes()),
    )
}

fn default_rbac_yaml() -> &'static str {
    "# Default RBAC: cluster-admin role + binding for the bootstrap admin user.\n\
     apiVersion: rbac.authorization.k8s.io/v1\n\
     kind: ClusterRole\n\
     metadata:\n  name: cluster-admin\n\
     rules:\n\
     - apiGroups: [\"*\"]\n  resources: [\"*\"]\n  verbs: [\"*\"]\n\
     - nonResourceURLs: [\"*\"]\n  verbs: [\"*\"]\n\
     ---\n\
     apiVersion: rbac.authorization.k8s.io/v1\n\
     kind: ClusterRoleBinding\n\
     metadata:\n  name: cave-cluster-admin\n\
     roleRef:\n\
       apiGroup: rbac.authorization.k8s.io\n\
       kind: ClusterRole\n\
       name: cluster-admin\n\
     subjects:\n\
     - kind: User\n  name: system:admin\n  apiGroup: rbac.authorization.k8s.io\n"
}

fn default_namespaces_yaml() -> &'static str {
    "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: default\n---\n\
     apiVersion: v1\nkind: Namespace\nmetadata:\n  name: kube-system\n---\n\
     apiVersion: v1\nkind: Namespace\nmetadata:\n  name: kube-public\n---\n\
     apiVersion: v1\nkind: Namespace\nmetadata:\n  name: cave-system\n"
}

pub async fn join(
    data_dir: &Path,
    bootstrap_token: &str,
    master_address: &str,
    node_name: &str,
    ca_bundle: Option<&Path>,
) -> Result<()> {
    info!(?data_dir, %master_address, "cluster join");
    if bootstrap_token.len() < 16 {
        return Err(anyhow!("bootstrap_token too short (need >= 16 chars)"));
    }
    fs::create_dir_all(data_dir)?;
    let resolved_node_name = if node_name.is_empty() {
        hostname().unwrap_or_else(|| format!("cave-worker-{}", uuid::Uuid::new_v4().simple()))
    } else {
        node_name.to_string()
    };

    let base = if master_address.starts_with("http://") || master_address.starts_with("https://") {
        master_address.to_string()
    } else {
        format!("https://{}", master_address)
    };
    let base = base.trim_end_matches('/').to_string();

    // ── Step 1: resolve the master CA cert (pin) ─────────────────────────
    let cached_ca_path = data_dir.join("master-ca.crt");
    let (ca_pem, pin_source) = resolve_master_ca(&base, ca_bundle, &cached_ca_path).await?;
    info!(source = pin_source, "master CA pinned");

    let pinned_cert = reqwest::Certificate::from_pem(ca_pem.as_bytes())
        .context("parse pinned master CA as reqwest::Certificate")?;
    let client = reqwest::Client::builder()
        .add_root_certificate(pinned_cert)
        .danger_accept_invalid_hostnames(true) // self-signed leaf may not have a matching SAN for the operator's address
        .timeout(StdDuration::from_secs(10))
        .build()?;

    // ── Step 2: bootstrap-token POST ─────────────────────────────────────
    let join_url = format!("{}/api/v1/bootstrap/join", base);
    let body = serde_json::json!({
        "token": bootstrap_token,
        "node_name": resolved_node_name,
    });
    let resp = client
        .post(&join_url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {}", join_url))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!(
            "master rejected join (status {}): {}",
            status,
            text.lines().next().unwrap_or("")
        ));
    }
    let parsed: serde_json::Value = serde_json::from_str(&text).context("parse join response")?;

    // ── Step 3: CSR — generate kubelet keypair locally, submit to master ─
    let csr_outcome = request_kubelet_certificate(
        &client,
        &base,
        bootstrap_token,
        &resolved_node_name,
        data_dir,
    )
    .await;
    let csr_summary = match &csr_outcome {
        Ok(_) => "issued".to_string(),
        Err(e) => {
            warn!(error = %e, "kubelet CSR request failed — join continues without a node cert");
            format!("failed ({})", e)
        }
    };

    let join_config = format!(
        "# Auto-generated by `cave-runtime cluster join` ({}).\n\
         master_address: {master}\n\
         node_name: {node}\n\
         bootstrap_token_redacted: {redacted}\n\
         status: {server_status}\n\
         server_message: {msg}\n\
         ca_pin_source: {pin_source}\n\
         kubelet_csr: {csr_summary}\n",
        OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "unknown".into()),
        master = master_address,
        node = resolved_node_name,
        redacted = format!(
            "{}...{}",
            &bootstrap_token[..4],
            &bootstrap_token[bootstrap_token.len() - 4..]
        ),
        server_status = parsed
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown"),
        msg = parsed
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        pin_source = pin_source,
        csr_summary = csr_summary,
    );
    fs::write(data_dir.join("join.yaml"), join_config)?;
    println!(
        "Joined cluster — wrote {}",
        data_dir.join("join.yaml").display()
    );
    println!("  master:       {}", master_address);
    println!("  node name:    {}", resolved_node_name);
    println!(
        "  server status: {}",
        parsed.get("status").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!("  CA pin:       {}", pin_source);
    println!("  kubelet CSR:  {}", csr_summary);
    csr_outcome?;
    Ok(())
}

/// Resolve the master CA certificate (PEM). Returns `(pem, source_label)`.
///
/// Order of precedence:
/// 1. `--ca-bundle PATH` is provided → load from that path.
/// 2. `<data_dir>/master-ca.crt` already cached → load + pin against it.
/// 3. TOFU — make one unverified TLS request to `GET <base>/api/v1/bootstrap/ca`,
///    cache the returned PEM at `<data_dir>/master-ca.crt`.
async fn resolve_master_ca(
    master_base: &str,
    explicit_ca_bundle: Option<&Path>,
    cached_path: &Path,
) -> Result<(String, &'static str)> {
    if let Some(p) = explicit_ca_bundle {
        let pem = fs::read_to_string(p)
            .with_context(|| format!("read --ca-bundle {}", p.display()))?;
        validate_pem_cert(&pem, p)?;
        return Ok((pem, "--ca-bundle"));
    }
    if cached_path.exists() {
        let pem = fs::read_to_string(cached_path)
            .with_context(|| format!("read cached CA {}", cached_path.display()))?;
        validate_pem_cert(&pem, cached_path)?;
        return Ok((pem, "cached"));
    }
    // TOFU fetch.
    let tofu_client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(StdDuration::from_secs(10))
        .build()?;
    let ca_url = format!("{}/api/v1/bootstrap/ca", master_base);
    let resp = tofu_client
        .get(&ca_url)
        .send()
        .await
        .with_context(|| format!("GET {} (TOFU)", ca_url))?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "master returned status {} when fetching CA via TOFU",
            resp.status()
        ));
    }
    let pem = resp.text().await.context("read CA response body")?;
    validate_pem_cert(&pem, cached_path)?;
    if let Some(parent) = cached_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(cached_path, &pem)
        .with_context(|| format!("cache CA at {}", cached_path.display()))?;
    Ok((pem, "TOFU"))
}

fn validate_pem_cert(pem: &str, source: impl std::fmt::Debug) -> Result<()> {
    if !pem.contains("BEGIN CERTIFICATE") {
        return Err(anyhow!(
            "CA bundle at {:?} does not look like a PEM certificate (missing BEGIN CERTIFICATE)",
            source
        ));
    }
    Ok(())
}

/// Submit a kubelet CSR to the master and persist the issued cert.
async fn request_kubelet_certificate(
    client: &reqwest::Client,
    master_base: &str,
    bootstrap_token: &str,
    node_name: &str,
    data_dir: &Path,
) -> Result<()> {
    // Generate kubelet keypair + a self-PEM CSR. The master signs against the
    // cluster CA and returns the leaf cert.
    let kubelet_kp = KeyPair::generate().map_err(|e| anyhow!("kubelet keypair: {e}"))?;
    let mut csr_params = CertificateParams::default();
    let cn = format!("system:node:{}", node_name);
    csr_params.distinguished_name.push(DnType::CommonName, &cn);
    csr_params
        .distinguished_name
        .push(DnType::OrganizationName, "system:nodes");
    if let Ok(ia5) = rcgen::Ia5String::try_from(node_name.to_string()) {
        csr_params.subject_alt_names.push(SanType::DnsName(ia5));
    }
    let csr = csr_params
        .serialize_request(&kubelet_kp)
        .map_err(|e| anyhow!("serialize CSR: {e}"))?;
    let csr_pem = csr.pem().map_err(|e| anyhow!("CSR pem: {e}"))?;

    let csr_url = format!("{}/api/v1/certificatesigningrequests", master_base);
    let body = serde_json::json!({
        "token": bootstrap_token,
        "node_name": node_name,
        "csr_pem": csr_pem,
        "usage": "kubelet-client",
    });
    let resp = client
        .post(&csr_url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {}", csr_url))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!(
            "master rejected CSR (status {}): {}",
            status,
            text.lines().next().unwrap_or("")
        ));
    }
    let parsed: serde_json::Value =
        serde_json::from_str(&text).context("parse CSR response")?;
    let signed_pem = parsed
        .get("certificate")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("CSR response missing `certificate` field"))?;
    let ca_pem = parsed
        .get("ca")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("CSR response missing `ca` field"))?;

    let kubelet_dir = data_dir.join("kubelet");
    fs::create_dir_all(&kubelet_dir)
        .with_context(|| format!("create {}", kubelet_dir.display()))?;
    fs::write(kubelet_dir.join("tls.crt"), signed_pem)?;
    fs::write(kubelet_dir.join("tls.key"), kubelet_kp.serialize_pem())?;
    fs::write(kubelet_dir.join("ca.crt"), ca_pem)?;
    Ok(())
}

fn hostname() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()))
}

pub async fn status(kubeconfig_path: &Path) -> Result<()> {
    let raw = fs::read_to_string(kubeconfig_path)
        .with_context(|| format!("read kubeconfig {}", kubeconfig_path.display()))?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&raw).context("parse kubeconfig YAML")?;
    let server = yaml
        .get("clusters")
        .and_then(|c| c.get(0))
        .and_then(|c0| c0.get("cluster"))
        .and_then(|c| c.get("server"))
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow!("kubeconfig: clusters[0].cluster.server missing"))?
        .to_string();
    let cluster_name = yaml
        .get("clusters")
        .and_then(|c| c.get(0))
        .and_then(|c0| c0.get("name"))
        .and_then(|s| s.as_str())
        .unwrap_or("<unknown>")
        .to_string();

    println!("Cluster:    {}", cluster_name);
    println!("Server:     {}", server);

    // Prefer the cluster's own CA from the kubeconfig's
    // `certificate-authority-data` blob — that is the strongest pin we have
    // when we're acting as the admin. Fall back to "no verification" only if
    // the blob is missing so this command still tells you something useful
    // for an out-of-band kubeconfig.
    let ca_blob_b64 = yaml
        .get("clusters")
        .and_then(|c| c.get(0))
        .and_then(|c0| c0.get("cluster"))
        .and_then(|c| c.get("certificate-authority-data"))
        .and_then(|s| s.as_str());
    let client = match ca_blob_b64 {
        Some(b64) => {
            use base64::Engine as _;
            let pem = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .context("decode certificate-authority-data")?;
            let cert = reqwest::Certificate::from_pem(&pem)
                .context("parse certificate-authority-data as PEM")?;
            reqwest::Client::builder()
                .add_root_certificate(cert)
                .danger_accept_invalid_hostnames(true)
                .timeout(StdDuration::from_secs(3))
                .build()?
        }
        None => reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(StdDuration::from_secs(3))
            .build()?,
    };

    // apiserver /healthz
    let api_url = format!("{}/healthz", server.trim_end_matches('/'));
    match client.get(&api_url).send().await {
        Ok(resp) => {
            let s = resp.status();
            let body = resp.text().await.unwrap_or_default();
            println!(
                "apiserver:  {} — {}",
                s,
                body.lines().next().unwrap_or("").trim()
            );
        }
        Err(e) => {
            println!("apiserver:  UNREACHABLE — {}", e);
        }
    }

    // etcd /healthz on port 2379 of the same host.
    if let Some(etcd_url) = etcd_url_from_apiserver(&server) {
        match client.get(&etcd_url).send().await {
            Ok(resp) => {
                let s = resp.status();
                let body = resp.text().await.unwrap_or_default();
                println!(
                    "etcd:       {} — {}",
                    s,
                    body.lines().next().unwrap_or("").trim()
                );
            }
            Err(e) => {
                println!("etcd:       UNREACHABLE — {}", e);
            }
        }
    }

    // Node count
    let nodes_url = format!("{}/api/v1/nodes", server.trim_end_matches('/'));
    match client.get(&nodes_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.text().await {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    let count = json.get("items").and_then(|i| i.as_array()).map(|a| a.len()).unwrap_or(0);
                    println!("Nodes:      {}", count);
                }
            }
        }
        Ok(resp) => println!("Nodes:      (status {})", resp.status()),
        Err(_) => println!("Nodes:      (unreachable)"),
    }
    Ok(())
}

fn etcd_url_from_apiserver(server: &str) -> Option<String> {
    // server looks like https://127.0.0.1:6443 — swap the port for 2379.
    let trimmed = server.trim_end_matches('/');
    let (scheme_host, _port) = trimmed.rsplit_once(':')?;
    Some(format!("{}:2379/api/etcd/health", scheme_host))
}

pub fn destroy(data_dir: &Path, force: bool) -> Result<()> {
    if !force {
        return Err(anyhow!(
            "refusing to destroy {} without --force",
            data_dir.display()
        ));
    }
    if !data_dir.exists() {
        println!("Nothing to destroy at {} (already absent).", data_dir.display());
        return Ok(());
    }
    let backup = data_dir.with_extension(format!(
        "backup-{}",
        OffsetDateTime::now_utc().unix_timestamp()
    ));
    fs::rename(data_dir, &backup).with_context(|| {
        format!(
            "rename {} -> {} (backup before destroy)",
            data_dir.display(),
            backup.display()
        )
    })?;
    fs::remove_dir_all(&backup).with_context(|| format!("remove {}", backup.display()))?;
    println!("Destroyed {} (rename-and-remove).", data_dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_creates_expected_layout() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("cluster");
        init(&dd, "unit-test", "127.0.0.1:6443", "single", "", 1, false).expect("init");
        assert!(dd.join("cluster.json").is_file());
        assert!(dd.join("pki/ca.crt").is_file());
        assert!(dd.join("pki/ca.key").is_file());
        assert!(dd.join("pki/admin.crt").is_file());
        assert!(dd.join("pki/sa.key").is_file());
        assert!(dd.join("kubeconfig/admin.kubeconfig").is_file());
        assert!(dd.join("manifests/rbac.yaml").is_file());
        assert!(dd.join("manifests/namespaces.yaml").is_file());

        for c in COMPONENTS {
            assert!(
                dd.join("pki").join(format!("{}.crt", c)).is_file(),
                "missing leaf cert: {}",
                c
            );
        }

        let manifest: ClusterManifest =
            serde_json::from_str(&fs::read_to_string(dd.join("cluster.json")).unwrap()).unwrap();
        assert_eq!(manifest.cluster_name, "unit-test");
        assert_eq!(manifest.components.len(), COMPONENTS.len());
    }

    #[test]
    fn init_is_idempotent_guard() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("cluster");
        init(&dd, "guard", "127.0.0.1:6443", "single", "", 1, false).unwrap();
        let err = init(&dd, "guard", "127.0.0.1:6443", "single", "", 1, false).unwrap_err();
        assert!(err.to_string().contains("already initialized"));
    }

    #[test]
    fn destroy_requires_force() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("cluster");
        init(&dd, "destroy-test", "127.0.0.1:6443", "single", "", 1, false).unwrap();
        assert!(destroy(&dd, false).is_err());
        assert!(dd.join("cluster.json").is_file(), "data dir must persist");
        destroy(&dd, true).expect("force destroy");
        assert!(!dd.exists(), "data dir must be gone");
    }

    #[test]
    fn kubeconfig_is_valid_yaml_with_b64_blobs() {
        let cfg = render_kubeconfig(
            "test-cluster",
            "127.0.0.1:6443",
            "CA-PEM",
            "ADMIN-CRT-PEM",
            "ADMIN-KEY-PEM",
        );
        let parsed: serde_yaml::Value = serde_yaml::from_str(&cfg).expect("yaml parse");
        let server = parsed["clusters"][0]["cluster"]["server"].as_str().unwrap();
        assert_eq!(server, "https://127.0.0.1:6443");
        // Spot-check that the CA blob is base64 (so contains no PEM whitespace)
        let ca_b64 = parsed["clusters"][0]["cluster"]["certificate-authority-data"]
            .as_str()
            .unwrap();
        assert!(!ca_b64.contains(' '));
        assert!(!ca_b64.contains('\n'));
    }

    #[tokio::test]
    async fn join_rejects_short_token() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("worker");
        // The short-token guard fires before any network call, so this is
        // safe to run without a live master.
        assert!(join(&dd, "short", "10.0.0.1:6443", "worker-1", None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn join_fails_when_master_unreachable() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("worker");
        // A bind on 127.0.0.1:1 will always refuse — exercises the network-error path.
        let res = join(
            &dd,
            "abcdef0123456789",
            "https://127.0.0.1:1",
            "worker-1",
            None,
        )
        .await;
        assert!(res.is_err(), "must fail when master unreachable");
    }

    #[test]
    fn init_writes_bootstrap_token() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("cluster");
        init(&dd, "tok-test", "127.0.0.1:6443", "single", "", 1, false).unwrap();
        let raw = fs::read_to_string(dd.join("bootstrap-tokens.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let tokens = parsed["tokens"].as_array().unwrap();
        assert_eq!(tokens.len(), 1);
        let tok = tokens[0]["token"].as_str().unwrap();
        assert!(tok.len() >= 16, "bootstrap token too short: {}", tok);
    }

    #[test]
    fn etcd_url_swaps_port_to_2379() {
        let u = super::etcd_url_from_apiserver("https://10.0.0.5:6443").unwrap();
        assert_eq!(u, "https://10.0.0.5:2379/api/etcd/health");
    }

    #[test]
    fn mint_bootstrap_token_is_unique() {
        let a = super::mint_bootstrap_token();
        let b = super::mint_bootstrap_token();
        assert_ne!(a, b);
        assert!(a.len() >= 16);
    }
}
