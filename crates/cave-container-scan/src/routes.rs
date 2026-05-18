// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::engine::{ScanOrchestrator, aggregate_verdict};
use crate::models::*;
use crate::scanners::{image::ImageScanner, iac::IacScanner, fs::FsScanner, secret::SecretScanner, yara::YaraScanner, namespace::NamespaceScanner};
use axum::{
    extract::{Path, State as AxumState},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

pub struct ContainerScanStore {
    pub results: Arc<RwLock<HashMap<Uuid, ScanResult>>>,
    pub stats: Arc<RwLock<ScanStats>>,
    pub orchestrator: ScanOrchestrator,
}

impl Default for ContainerScanStore {
    fn default() -> Self {
        let scanners: Vec<Box<dyn crate::engine::Scanner>> = vec![
            Box::new(ImageScanner),
            Box::new(IacScanner),
            Box::new(FsScanner),
            Box::new(SecretScanner),
            Box::new(YaraScanner::new()),
            Box::new(NamespaceScanner),
        ];

        Self {
            results: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(ScanStats::default())),
            orchestrator: ScanOrchestrator::new(scanners),
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "container-scan"
    }))
}

async fn scan_image(
    AxumState(state): AxumState<Arc<ContainerScanStore>>,
    Json(payload): Json<ImageScanRequest>,
) -> (StatusCode, Json<ScanResult>) {
    let req = ScanRequest {
        kind: ScanKind::Image,
        target: ScanTarget::ImageRef(payload.r#ref),
        options: ScanOptions::default(),
    };

    let result = state.orchestrator.run(&req).await;

    // Store result
    let result_id = result.id;
    {
        let mut results = state.results.write().await;
        results.insert(result_id, result.clone());
    }

    // Update stats
    {
        let mut stats = state.stats.write().await;
        stats.total_scans += 1;
        for finding in &result.findings {
            *stats.findings_by_severity.entry(finding.severity).or_insert(0) += 1;
        }
    }

    (StatusCode::OK, Json(result))
}

async fn scan_iac(
    AxumState(state): AxumState<Arc<ContainerScanStore>>,
    Json(payload): Json<IacScanRequest>,
) -> (StatusCode, Json<ScanResult>) {
    let req = ScanRequest {
        kind: ScanKind::Iac,
        target: ScanTarget::IacBundle {
            kind: payload.kind,
            content: payload.content,
        },
        options: ScanOptions::default(),
    };

    let result = state.orchestrator.run(&req).await;

    // Store result
    let result_id = result.id;
    {
        let mut results = state.results.write().await;
        results.insert(result_id, result.clone());
    }

    // Update stats
    {
        let mut stats = state.stats.write().await;
        stats.total_scans += 1;
        for finding in &result.findings {
            *stats.findings_by_severity.entry(finding.severity).or_insert(0) += 1;
        }
    }

    (StatusCode::OK, Json(result))
}

async fn scan_fs(
    AxumState(state): AxumState<Arc<ContainerScanStore>>,
    Json(payload): Json<FsScanRequest>,
) -> (StatusCode, Json<ScanResult>) {
    let target = if let Some(path) = payload.path {
        ScanTarget::FsPath(path)
    } else {
        ScanTarget::Content(vec![])
    };

    let req = ScanRequest {
        kind: ScanKind::Fs,
        target,
        options: ScanOptions::default(),
    };

    let result = state.orchestrator.run(&req).await;

    // Store result
    let result_id = result.id;
    {
        let mut results = state.results.write().await;
        results.insert(result_id, result.clone());
    }

    // Update stats
    {
        let mut stats = state.stats.write().await;
        stats.total_scans += 1;
        for finding in &result.findings {
            *stats.findings_by_severity.entry(finding.severity).or_insert(0) += 1;
        }
    }

    (StatusCode::OK, Json(result))
}

async fn scan_secret(
    AxumState(state): AxumState<Arc<ContainerScanStore>>,
    Json(payload): Json<SecretScanRequest>,
) -> (StatusCode, Json<ScanResult>) {
    let req = ScanRequest {
        kind: ScanKind::Secret,
        target: ScanTarget::Content(payload.content.into_bytes()),
        options: ScanOptions::default(),
    };

    let result = state.orchestrator.run(&req).await;

    // Store result
    let result_id = result.id;
    {
        let mut results = state.results.write().await;
        results.insert(result_id, result.clone());
    }

    // Update stats
    {
        let mut stats = state.stats.write().await;
        stats.total_scans += 1;
        for finding in &result.findings {
            *stats.findings_by_severity.entry(finding.severity).or_insert(0) += 1;
        }
    }

    (StatusCode::OK, Json(result))
}

async fn scan_yara(
    AxumState(state): AxumState<Arc<ContainerScanStore>>,
    Json(payload): Json<YaraScanRequest>,
) -> (StatusCode, Json<ScanResult>) {
    let req = ScanRequest {
        kind: ScanKind::Yara,
        target: ScanTarget::Content(payload.bytes),
        options: ScanOptions::default(),
    };

    let result = state.orchestrator.run(&req).await;

    // Store result
    let result_id = result.id;
    {
        let mut results = state.results.write().await;
        results.insert(result_id, result.clone());
    }

    // Update stats
    {
        let mut stats = state.stats.write().await;
        stats.total_scans += 1;
        for finding in &result.findings {
            *stats.findings_by_severity.entry(finding.severity).or_insert(0) += 1;
        }
    }

    (StatusCode::OK, Json(result))
}

async fn scan_namespace(
    AxumState(state): AxumState<Arc<ContainerScanStore>>,
    Json(payload): Json<NamespaceScanRequest>,
) -> (StatusCode, Json<ScanResult>) {
    let req = ScanRequest {
        kind: ScanKind::Namespace,
        target: ScanTarget::PackageName {
            ecosystem: payload.ecosystem,
            name: payload.name,
        },
        options: ScanOptions::default(),
    };

    let result = state.orchestrator.run(&req).await;

    // Store result
    let result_id = result.id;
    {
        let mut results = state.results.write().await;
        results.insert(result_id, result.clone());
    }

    // Update stats
    {
        let mut stats = state.stats.write().await;
        stats.total_scans += 1;
        for finding in &result.findings {
            *stats.findings_by_severity.entry(finding.severity).or_insert(0) += 1;
        }
    }

    (StatusCode::OK, Json(result))
}

async fn evaluate_verdict(
    AxumState(_state): AxumState<Arc<ContainerScanStore>>,
    Json(payload): Json<VerdictRequest>,
) -> Json<ScanVerdict> {
    let verdict = aggregate_verdict(&payload.findings, payload.floor);
    Json(verdict)
}

async fn list_results(AxumState(state): AxumState<Arc<ContainerScanStore>>) -> Json<Vec<ScanResult>> {
    let results = state.results.read().await;
    let mut vec: Vec<ScanResult> = results.values().cloned().collect();
    vec.sort_by(|a, b| b.finished_at.cmp(&a.finished_at));
    vec.truncate(100);
    Json(vec)
}

async fn get_result(
    AxumState(state): AxumState<Arc<ContainerScanStore>>,
    Path(id): Path<Uuid>,
) -> (StatusCode, Json<Option<ScanResult>>) {
    let results = state.results.read().await;
    let result = results.get(&id).cloned();
    match result {
        Some(r) => (StatusCode::OK, Json(Some(r))),
        None => (StatusCode::NOT_FOUND, Json(None)),
    }
}

async fn list_rules() -> Json<RulesResponse> {
    let yara_rules = vec![
        YaraRuleMetadata {
            id: "M_CryptoMiner_Generic_2024".to_string(),
            name: "Generic Crypto Miner Pattern".to_string(),
            patterns: vec!["stratum+tcp://".to_string(), "xmrig".to_string()],
            severity: Severity::Critical,
        },
        YaraRuleMetadata {
            id: "M_BashDownloader_B".to_string(),
            name: "Bash Downloader Pattern".to_string(),
            patterns: vec!["bash -c.*curl.*|".to_string(), "wget.*|.*bash".to_string()],
            severity: Severity::High,
        },
    ];

    let iac_rules = vec![
        IacRuleMetadata {
            id: "DOCK-001".to_string(),
            name: "Base image uses :latest tag".to_string(),
            kind: "Dockerfile".to_string(),
            severity: Severity::Medium,
        },
        IacRuleMetadata {
            id: "K8S-001".to_string(),
            name: "Privileged container detected".to_string(),
            kind: "Kubernetes".to_string(),
            severity: Severity::Critical,
        },
        IacRuleMetadata {
            id: "TF-001".to_string(),
            name: "S3 bucket allows public read".to_string(),
            kind: "Terraform".to_string(),
            severity: Severity::Critical,
        },
    ];

    let secret_rules = vec![
        SecretRuleMetadata {
            id: "SEC-001".to_string(),
            name: "AWS Access Key detected".to_string(),
            pattern: "AKIA[0-9A-Z]{16}".to_string(),
            severity: Severity::Critical,
        },
        SecretRuleMetadata {
            id: "SEC-002".to_string(),
            name: "GitHub Personal Access Token detected".to_string(),
            pattern: "ghp_[A-Za-z0-9]{36}".to_string(),
            severity: Severity::Critical,
        },
    ];

    let namespace_rules = vec![
        NamespaceRuleMetadata {
            id: "NS-001".to_string(),
            name: "Potential namespace confusion / typosquat".to_string(),
            ecosystem: "PyPI, Npm, Maven, etc.".to_string(),
        },
    ];

    Json(RulesResponse {
        yara: yara_rules,
        iac: iac_rules,
        secret: secret_rules,
        namespace: namespace_rules,
    })
}

async fn get_stats(AxumState(state): AxumState<Arc<ContainerScanStore>>) -> Json<ScanStats> {
    let stats = state.stats.read().await;
    Json(stats.clone())
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn create_router(state: Arc<ContainerScanStore>) -> Router {
    Router::new()
        .route("/api/container-scan/health", get(health))
        .route("/api/container-scan/image", post(scan_image))
        .route("/api/container-scan/iac", post(scan_iac))
        .route("/api/container-scan/fs", post(scan_fs))
        .route("/api/container-scan/secret", post(scan_secret))
        .route("/api/container-scan/yara", post(scan_yara))
        .route("/api/container-scan/namespace", post(scan_namespace))
        .route("/api/container-scan/verdict", post(evaluate_verdict))
        .route("/api/container-scan/results", get(list_results))
        .route("/api/container-scan/results/{id}", get(get_result))
        .route("/api/container-scan/rules", get(list_rules))
        .route("/api/container-scan/stats", get(get_stats))
        .with_state(state)
}
