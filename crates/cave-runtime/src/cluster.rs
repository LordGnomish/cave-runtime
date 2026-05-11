//! `cave-runtime cluster` subcommands: `init`, `join`, `status`, `destroy`.
//!
//! Cluster lifecycle plumbing — provisions data dir, PKI hierarchy
//! (root CA + per-component leaves), kubeconfig, RBAC, and default
//! namespaces. The actual control-plane processes are started by
//! `cave-runtime serve` against the same data dir.

use anyhow::{anyhow, Context, Result};
use clap::Subcommand;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose, SanType,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration as StdDuration;
use time::{Duration, OffsetDateTime};
use tracing::info;

/// Cluster lifecycle subcommands.
#[derive(Subcommand, Debug, Clone)]
pub enum ClusterCmd {
    /// Initialize a new single-node Cave cluster.
    Init {
        #[arg(long, default_value = "/var/lib/cave")]
        data_dir: PathBuf,
        #[arg(long, default_value = "cave-local")]
        cluster_name: String,
        #[arg(long, default_value = "127.0.0.1:6443")]
        advertise_address: String,
    },
    /// Worker-node join skeleton (writes bootstrap config; full handshake
    /// pending real apiserver wiring).
    Join {
        #[arg(long)]
        bootstrap_token: String,
        #[arg(long)]
        master_address: String,
        #[arg(long, default_value = "/var/lib/cave")]
        data_dir: PathBuf,
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
#[derive(Debug, Serialize, Deserialize)]
pub struct ClusterManifest {
    pub cluster_name: String,
    pub advertise_address: String,
    pub created_at: String,
    pub pki_root_cn: String,
    pub components: Vec<String>,
    pub kubeconfig_path: String,
    pub data_dir: String,
}

pub async fn dispatch(cmd: ClusterCmd) -> Result<()> {
    match cmd {
        ClusterCmd::Init {
            data_dir,
            cluster_name,
            advertise_address,
        } => init(&data_dir, &cluster_name, &advertise_address),
        ClusterCmd::Join {
            bootstrap_token,
            master_address,
            data_dir,
        } => join(&data_dir, &bootstrap_token, &master_address),
        ClusterCmd::Status { kubeconfig } => status(&kubeconfig).await,
        ClusterCmd::Destroy { data_dir, force } => destroy(&data_dir, force),
    }
}

/// `cluster init` — provision a fresh single-node cluster on disk.
pub fn init(data_dir: &Path, cluster_name: &str, advertise_address: &str) -> Result<()> {
    info!(?data_dir, %cluster_name, %advertise_address, "cluster init");

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

    let root_cn = format!("cave-runtime root CA ({})", cluster_name);
    let (ca_cert_pem, ca_key_pem, ca_cert, ca_key) = generate_root_ca(&root_cn)?;
    fs::write(pki_dir.join("ca.crt"), &ca_cert_pem)?;
    fs::write(pki_dir.join("ca.key"), &ca_key_pem)?;

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
    fs::write(
        etcd_dir.join("README"),
        "Reserved for embedded etcd state (populated by `cave-runtime serve`).\n",
    )?;

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
    println!();
    println!("Next: cave-runtime serve  (with cluster-aware config — TODO)");
    Ok(())
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
    // Server-style components get DNS + IP SANs for the advertise host.
    if matches!(component, "apiserver" | "etcd" | "kubelet") {
        if let Ok(ia5) = rcgen::Ia5String::try_from(advertise_host.to_string()) {
            params.subject_alt_names.push(SanType::DnsName(ia5));
        }
        if let Ok(ip) = advertise_host.parse() {
            params.subject_alt_names.push(SanType::IpAddress(ip));
        }
        if let Ok(localhost) = rcgen::Ia5String::try_from("localhost".to_string()) {
            params.subject_alt_names.push(SanType::DnsName(localhost));
        }
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

pub fn join(data_dir: &Path, bootstrap_token: &str, master_address: &str) -> Result<()> {
    info!(?data_dir, %master_address, "cluster join (skeleton)");
    if bootstrap_token.len() < 16 {
        return Err(anyhow!("bootstrap_token too short (need >= 16 chars)"));
    }
    fs::create_dir_all(data_dir)?;
    let config = format!(
        "# Auto-generated by `cave-runtime cluster join`.\n\
         master_address: {master}\n\
         bootstrap_token_redacted: {redacted}\n\
         status: pending-handshake\n",
        master = master_address,
        redacted = format!("{}...{}",
            &bootstrap_token[..4],
            &bootstrap_token[bootstrap_token.len() - 4..]),
    );
    fs::write(data_dir.join("join.yaml"), config)?;
    println!("Wrote join config to {}", data_dir.join("join.yaml").display());
    println!("NOTE: full bootstrap handshake is not yet wired up; this is a config-only stub.");
    Ok(())
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

    let url = format!("{}/healthz", server.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(StdDuration::from_secs(3))
        .build()?;
    match client.get(&url).send().await {
        Ok(resp) => {
            let s = resp.status();
            let body = resp.text().await.unwrap_or_default();
            println!("Health:     {} — {}", s, body.lines().next().unwrap_or(""));
        }
        Err(e) => {
            println!("Health:     UNREACHABLE — {}", e);
        }
    }
    Ok(())
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
        init(&dd, "unit-test", "127.0.0.1:6443").expect("init");
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
        init(&dd, "guard", "127.0.0.1:6443").unwrap();
        let err = init(&dd, "guard", "127.0.0.1:6443").unwrap_err();
        assert!(err.to_string().contains("already initialized"));
    }

    #[test]
    fn destroy_requires_force() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("cluster");
        init(&dd, "destroy-test", "127.0.0.1:6443").unwrap();
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

    #[test]
    fn join_writes_config_and_redacts_token() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("worker");
        join(&dd, "abcdef0123456789", "10.0.0.1:6443").unwrap();
        let raw = fs::read_to_string(dd.join("join.yaml")).unwrap();
        assert!(raw.contains("10.0.0.1:6443"));
        assert!(raw.contains("abcd...6789"));
        assert!(!raw.contains("abcdef0123456789"));
    }

    #[test]
    fn join_rejects_short_token() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("worker");
        assert!(join(&dd, "short", "10.0.0.1:6443").is_err());
    }
}
