//! Vulnerability scanning integration hooks.
//! Provides an async hook interface; concrete scanners (Trivy, Grype, etc.)
//! implement ScanHook and are registered with the ScanManager.

use crate::harbor::store::RegistryStore;
use crate::harbor::types::{ScanResult, ScanStatus};
use std::sync::Arc;
use tracing::{error, info};

// ── Hook interface ────────────────────────────────────────────────────────────

/// Implementors perform the actual scan and return results.
pub trait ScanHook: Send + Sync {
    fn name(&self) -> &str;
    fn scan<'a>(
        &'a self,
        manifest_digest: &'a str,
        manifest_bytes: &'a [u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ScanResult> + Send + 'a>>;
}

// ── Noop scanner (default / testing) ─────────────────────────────────────────

pub struct NoopScanner;

impl ScanHook for NoopScanner {
    fn name(&self) -> &str {
        "noop"
    }

    fn scan<'a>(
        &'a self,
        manifest_digest: &'a str,
        _manifest_bytes: &'a [u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ScanResult> + Send + 'a>> {
        let digest = manifest_digest.to_string();
        Box::pin(async move {
            ScanResult {
                manifest_digest: digest,
                scanner: "noop".to_string(),
                status: ScanStatus::NotSupported,
                vulnerabilities: vec![],
            }
        })
    }
}

// ── Manager ───────────────────────────────────────────────────────────────────

pub struct ScanManager {
    store: Arc<RegistryStore>,
    hooks: Vec<Box<dyn ScanHook>>,
}

impl ScanManager {
    pub fn new(store: Arc<RegistryStore>) -> Self {
        Self { store, hooks: vec![Box::new(NoopScanner)] }
    }

    pub fn register_hook(&mut self, hook: Box<dyn ScanHook>) {
        // Replace noop if present when a real scanner is registered.
        self.hooks.retain(|h| h.name() != "noop");
        self.hooks.push(hook);
    }

    /// Trigger all registered scanners for the given manifest.
    /// Spawns background tasks; results are stored in the registry store.
    pub async fn trigger(&self, manifest_digest: &str, manifest_bytes: Vec<u8>) {
        for hook in &self.hooks {
            let name = hook.name().to_string();
            let bytes = manifest_bytes.clone();
            let digest = manifest_digest.to_string();
            let store = Arc::clone(&self.store);
            // We can't move `hook` (it's borrowed), so we call scan and await inline,
            // then spawn storage.
            let result = hook.scan(&digest, &bytes).await;
            match result.status {
                ScanStatus::NotSupported => {
                    info!(target: "cave_registry::scan", scanner = %name, "scan not supported");
                }
                ScanStatus::Failed => {
                    error!(target: "cave_registry::scan", scanner = %name, %digest, "scan failed");
                }
                _ => {
                    info!(
                        target: "cave_registry::scan",
                        scanner = %name,
                        %digest,
                        vuln_count = result.vulnerabilities.len(),
                        "scan complete"
                    );
                }
            }
            store.store_scan_result(result).await;
        }
    }

    pub async fn results(&self, digest: &str) -> Vec<ScanResult> {
        self.store.get_scan_results(digest).await
    }
}

// ── Test scanner ──────────────────────────────────────────────────────────────

#[cfg(test)]
pub struct MockScanner {
    pub vulns: Vec<crate::harbor::types::Vulnerability>,
}

#[cfg(test)]
impl ScanHook for MockScanner {
    fn name(&self) -> &str {
        "mock"
    }

    fn scan<'a>(
        &'a self,
        manifest_digest: &'a str,
        _manifest_bytes: &'a [u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ScanResult> + Send + 'a>> {
        let digest = manifest_digest.to_string();
        let vulns = self.vulns.clone();
        Box::pin(async move {
            ScanResult {
                manifest_digest: digest,
                scanner: "mock".to_string(),
                status: ScanStatus::Complete,
                vulnerabilities: vulns,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harbor::types::Vulnerability;

    #[tokio::test]
    async fn test_noop_scanner_returns_not_supported() {
        let store = Arc::new(RegistryStore::new());
        let mgr = ScanManager::new(Arc::clone(&store));

        mgr.trigger("sha256:deadbeef", b"{}".to_vec()).await;

        let results = mgr.results("sha256:deadbeef").await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ScanStatus::NotSupported);
    }

    #[tokio::test]
    async fn test_mock_scanner_stores_vulnerabilities() {
        let store = Arc::new(RegistryStore::new());
        let mut mgr = ScanManager::new(Arc::clone(&store));
        mgr.register_hook(Box::new(MockScanner {
            vulns: vec![Vulnerability {
                id: "CVE-2024-0001".to_string(),
                severity: "HIGH".to_string(),
                package: "openssl".to_string(),
                version: "1.0.0".to_string(),
                fixed_version: Some("1.0.1".to_string()),
                description: None,
            }],
        }));

        mgr.trigger("sha256:abcdef", b"{}".to_vec()).await;

        let results = mgr.results("sha256:abcdef").await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, ScanStatus::Complete);
        assert_eq!(results[0].vulnerabilities.len(), 1);
        assert_eq!(results[0].vulnerabilities[0].id, "CVE-2024-0001");
    }
}
