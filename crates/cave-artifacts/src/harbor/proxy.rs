// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: goharbor/harbor@c80058d52f555c9bd4552ea14c9d3e73ba0e4b12 src/server/middleware/repoproxy/proxy.go + src/pkg/proxy/proxy.go
//! Pull-through proxy — upstream registry adapters for multiple ecosystems.
//!
//! When `cave-registry` receives a request for an artefact it does not yet
//! have, `ProxyClient` figures out which upstream owns that artefact, fetches
//! it, and returns the bytes to the caller. The caller is responsible for
//! deciding what to do with those bytes (cache them, pass them through the
//! scan pipeline, etc.). This module is deliberately storage- and scan-
//! agnostic — it is purely the HTTP adapter layer.
//!
//! See ADR-133 for the broader Paranoid Artifact Proxy design.

use bytes::Bytes;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Supported ecosystems
// ---------------------------------------------------------------------------

/// Ecosystems recognised by the pull-through proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ecosystem {
    PyPI,
    Npm,
    Maven,
    RubyGems,
    Cargo,
    Go,
    NuGet,
    Composer,
    Oci,
}

impl Ecosystem {
    pub fn as_str(self) -> &'static str {
        match self {
            Ecosystem::PyPI => "pypi",
            Ecosystem::Npm => "npm",
            Ecosystem::Maven => "maven",
            Ecosystem::RubyGems => "rubygems",
            Ecosystem::Cargo => "cargo",
            Ecosystem::Go => "go",
            Ecosystem::NuGet => "nuget",
            Ecosystem::Composer => "composer",
            Ecosystem::Oci => "oci",
        }
    }

    pub fn from_slug(s: &str) -> Option<Ecosystem> {
        Some(match s {
            "pypi" => Ecosystem::PyPI,
            "npm" => Ecosystem::Npm,
            "maven" => Ecosystem::Maven,
            "rubygems" => Ecosystem::RubyGems,
            "cargo" => Ecosystem::Cargo,
            "go" => Ecosystem::Go,
            "nuget" => Ecosystem::NuGet,
            "composer" => Ecosystem::Composer,
            "oci" => Ecosystem::Oci,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// Upstream configuration
// ---------------------------------------------------------------------------

/// A single upstream registry we can pull from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamRegistry {
    pub ecosystem: Ecosystem,
    /// Base URL, no trailing slash (e.g. `https://pypi.org/simple`).
    pub base_url: String,
    /// Optional bearer / basic auth header value if the upstream requires it.
    pub auth_header: Option<String>,
    /// If `true`, this upstream is disabled from cache-miss fetch.
    pub disabled: bool,
}

/// Proxy operating mode — controls what happens when the scan pipeline returns
/// `Fail` for a freshly fetched artefact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    /// Don't pull through at all — serve only locally cached artefacts.
    Off,
    /// Fetch + scan but serve regardless of verdict (baseline window).
    ObserveOnly,
    /// Fetch + scan; on `Fail`, return 451 with the ADR-133 §3.4 body.
    Enforce,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub mode: ProxyMode,
    pub upstreams: HashMap<Ecosystem, UpstreamRegistry>,
    /// Timeout for a single upstream request, applied both to metadata index
    /// fetches and artefact downloads.
    pub timeout_seconds: u64,
    /// Explicit blocklist — names that are always rejected regardless of any
    /// scan result. Format: `"<ecosystem>/<name>"`, e.g. `"pypi/malicious-lib"`.
    pub blocklist: Vec<String>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        let upstreams = HashMap::from([
            (
                Ecosystem::PyPI,
                UpstreamRegistry {
                    ecosystem: Ecosystem::PyPI,
                    base_url: "https://pypi.org/simple".to_string(),
                    auth_header: None,
                    disabled: false,
                },
            ),
            (
                Ecosystem::Npm,
                UpstreamRegistry {
                    ecosystem: Ecosystem::Npm,
                    base_url: "https://registry.npmjs.org".to_string(),
                    auth_header: None,
                    disabled: false,
                },
            ),
            (
                Ecosystem::Maven,
                UpstreamRegistry {
                    ecosystem: Ecosystem::Maven,
                    base_url: "https://repo1.maven.org/maven2".to_string(),
                    auth_header: None,
                    disabled: false,
                },
            ),
            (
                Ecosystem::RubyGems,
                UpstreamRegistry {
                    ecosystem: Ecosystem::RubyGems,
                    base_url: "https://rubygems.org".to_string(),
                    auth_header: None,
                    disabled: false,
                },
            ),
            (
                Ecosystem::Cargo,
                UpstreamRegistry {
                    ecosystem: Ecosystem::Cargo,
                    base_url: "https://index.crates.io".to_string(),
                    auth_header: None,
                    disabled: false,
                },
            ),
            (
                Ecosystem::Go,
                UpstreamRegistry {
                    ecosystem: Ecosystem::Go,
                    base_url: "https://proxy.golang.org".to_string(),
                    auth_header: None,
                    disabled: false,
                },
            ),
            (
                Ecosystem::NuGet,
                UpstreamRegistry {
                    ecosystem: Ecosystem::NuGet,
                    base_url: "https://api.nuget.org/v3-flatcontainer".to_string(),
                    auth_header: None,
                    disabled: false,
                },
            ),
            (
                Ecosystem::Composer,
                UpstreamRegistry {
                    ecosystem: Ecosystem::Composer,
                    base_url: "https://repo.packagist.org".to_string(),
                    auth_header: None,
                    disabled: false,
                },
            ),
            (
                Ecosystem::Oci,
                UpstreamRegistry {
                    ecosystem: Ecosystem::Oci,
                    base_url: "https://registry-1.docker.io".to_string(),
                    auth_header: None,
                    disabled: false,
                },
            ),
        ]);

        Self {
            mode: ProxyMode::ObserveOnly,
            upstreams,
            timeout_seconds: 30,
            blocklist: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Proxy client
// ---------------------------------------------------------------------------

pub struct ProxyClient {
    cfg: ProxyConfig,
    http: Client,
}

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("proxy is off; cache-miss cannot be resolved")]
    ProxyOff,
    #[error("ecosystem {0:?} has no upstream configured")]
    NoUpstream(Ecosystem),
    #[error("upstream {0:?} is disabled")]
    UpstreamDisabled(Ecosystem),
    #[error("package {ecosystem}/{name} is on the static blocklist")]
    Blocked { ecosystem: &'static str, name: String },
    #[error("upstream {upstream} returned status {status}")]
    UpstreamStatus { upstream: String, status: u16 },
    #[error("upstream {upstream} fetch failed: {source}")]
    UpstreamFetch {
        upstream: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("http client build failed: {0}")]
    ClientBuild(String),
}

/// Artefact fetched from upstream — bytes + content-type + canonical URL +
/// sha256 digest (computed here so downstream callers don't need to).
#[derive(Debug, Clone)]
pub struct FetchedArtifact {
    pub ecosystem: Ecosystem,
    pub name: String,
    pub version: Option<String>,
    pub content_type: String,
    pub bytes: Bytes,
    pub sha256_hex: String,
    pub upstream_url: String,
}

impl ProxyClient {
    pub fn new(cfg: ProxyConfig) -> Self {
        // `reqwest::Client::builder()` can fail, but only for truly bizarre
        // TLS configurations; fall back to the default client in that case so
        // boot never fails.
        let http = Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_seconds))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { cfg, http }
    }

    pub fn config(&self) -> &ProxyConfig {
        &self.cfg
    }

    pub fn mode(&self) -> ProxyMode {
        self.cfg.mode
    }

    /// Classify a URL path prefix under `/api/registry/{ecosystem}/...` into
    /// an `Ecosystem`.
    pub fn ecosystem_from_prefix(prefix: &str) -> Option<Ecosystem> {
        Ecosystem::from_slug(prefix)
    }

    /// Check the static blocklist for `<ecosystem>/<name>` — runs before any
    /// upstream call to keep the scan pipeline cheap for known-bad names.
    pub fn is_blocked(&self, ecosystem: Ecosystem, name: &str) -> bool {
        let key = format!("{}/{}", ecosystem.as_str(), name);
        self.cfg.blocklist.iter().any(|b| b == &key)
    }

    /// Fetch a package artefact from the configured upstream for this
    /// ecosystem. Works for every supported ecosystem — callers pass the
    /// ecosystem-specific `path` portion (e.g. `"requests/requests-2.31.0.tar.gz"`
    /// for PyPI, `"lodash/-/lodash-4.17.21.tgz"` for npm, etc.). The caller
    /// is responsible for knowing the correct path; this keeps the proxy
    /// client small and testable.
    pub async fn fetch(
        &self,
        ecosystem: Ecosystem,
        package_name: &str,
        version: Option<&str>,
        path: &str,
    ) -> Result<FetchedArtifact, ProxyError> {
        if matches!(self.cfg.mode, ProxyMode::Off) {
            return Err(ProxyError::ProxyOff);
        }
        if self.is_blocked(ecosystem, package_name) {
            return Err(ProxyError::Blocked {
                ecosystem: ecosystem.as_str(),
                name: package_name.to_string(),
            });
        }
        let up = self
            .cfg
            .upstreams
            .get(&ecosystem)
            .ok_or(ProxyError::NoUpstream(ecosystem))?;
        if up.disabled {
            return Err(ProxyError::UpstreamDisabled(ecosystem));
        }

        let url = format!("{}/{}", up.base_url.trim_end_matches('/'), path.trim_start_matches('/'));
        debug!(target: "cave_registry::proxy", %url, "cache-miss upstream fetch");

        let mut req = self.http.get(&url);
        if let Some(auth) = &up.auth_header {
            req = req.header(reqwest::header::AUTHORIZATION, auth);
        }
        // OCI registry dance is more involved (token exchange); for now we
        // honour any `auth_header` but do not implement token refresh. A real
        // deployment overrides this by replacing `ProxyClient` behind a
        // feature flag.
        let resp = req.send().await.map_err(|e| ProxyError::UpstreamFetch {
            upstream: url.clone(),
            source: e,
        })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(ProxyError::UpstreamStatus {
                upstream: url,
                status: status.as_u16(),
            });
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or(default_content_type(ecosystem))
            .to_string();
        let bytes = resp.bytes().await.map_err(|e| ProxyError::UpstreamFetch {
            upstream: url.clone(),
            source: e,
        })?;
        let sha256_hex = {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(&bytes))
        };

        info!(
            target: "cave_registry::proxy",
            ecosystem = ecosystem.as_str(),
            name = package_name,
            version = ?version,
            bytes = bytes.len(),
            "upstream fetch succeeded"
        );

        Ok(FetchedArtifact {
            ecosystem,
            name: package_name.to_string(),
            version: version.map(str::to_string),
            content_type,
            bytes,
            sha256_hex,
            upstream_url: url,
        })
    }

    /// Fetch the index/metadata for a package (e.g. PyPI Simple HTML page for
    /// a project). Follows the same rules as `fetch` but returns the raw
    /// bytes + content-type without computing a digest (the index is not
    /// itself a cached artefact).
    pub async fn fetch_index(
        &self,
        ecosystem: Ecosystem,
        path: &str,
    ) -> Result<(String, Bytes), ProxyError> {
        if matches!(self.cfg.mode, ProxyMode::Off) {
            return Err(ProxyError::ProxyOff);
        }
        let up = self
            .cfg
            .upstreams
            .get(&ecosystem)
            .ok_or(ProxyError::NoUpstream(ecosystem))?;
        if up.disabled {
            return Err(ProxyError::UpstreamDisabled(ecosystem));
        }
        let url = format!("{}/{}", up.base_url.trim_end_matches('/'), path.trim_start_matches('/'));
        let mut req = self.http.get(&url);
        if let Some(auth) = &up.auth_header {
            req = req.header(reqwest::header::AUTHORIZATION, auth);
        }
        let resp = req.send().await.map_err(|e| ProxyError::UpstreamFetch {
            upstream: url.clone(),
            source: e,
        })?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            warn!(target: "cave_registry::proxy", %url, "index not found upstream");
        }
        if !status.is_success() {
            return Err(ProxyError::UpstreamStatus {
                upstream: url,
                status: status.as_u16(),
            });
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = resp.bytes().await.map_err(|e| ProxyError::UpstreamFetch {
            upstream: url,
            source: e,
        })?;
        Ok((content_type, bytes))
    }

    /// Rewrite absolute upstream URLs inside a fetched index (PyPI Simple
    /// pages point at `https://files.pythonhosted.org/...`) so the client
    /// pip/uv keeps hitting cave-registry instead of jumping to the public
    /// CDN. Caller passes the current request host (e.g. `cave-registry.cave.caveplatform.dev`).
    ///
    /// Best-effort — uses a conservative regex rather than a full HTML parser
    /// since PyPI Simple HTML is very regular.
    pub fn rewrite_urls(&self, ecosystem: Ecosystem, body: &str, registry_host: &str) -> String {
        use regex::Regex;
        let host = registry_host.trim_start_matches("https://").trim_start_matches("http://");
        let replacement = format!("https://{host}/api/registry/{eco}/blob", eco = ecosystem.as_str());
        let re = match ecosystem {
            Ecosystem::PyPI => Regex::new(r#"https://files\.pythonhosted\.org[^\s"'<>]+"#).ok(),
            Ecosystem::Npm => Regex::new(r#"https://registry\.npmjs\.org[^\s"'<>]+"#).ok(),
            _ => None,
        };
        match re {
            Some(re) => re.replace_all(body, replacement.as_str()).to_string(),
            None => body.to_string(),
        }
    }
}

fn default_content_type(ecosystem: Ecosystem) -> &'static str {
    match ecosystem {
        Ecosystem::PyPI => "application/x-python-wheel",
        Ecosystem::Npm => "application/octet-stream",
        Ecosystem::Maven => "application/java-archive",
        Ecosystem::RubyGems => "application/octet-stream",
        Ecosystem::Cargo => "application/gzip",
        Ecosystem::Go => "application/zip",
        Ecosystem::NuGet => "application/zip",
        Ecosystem::Composer => "application/zip",
        Ecosystem::Oci => "application/vnd.oci.image.manifest.v1+json",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_roundtrip() {
        for e in [
            Ecosystem::PyPI,
            Ecosystem::Npm,
            Ecosystem::Maven,
            Ecosystem::RubyGems,
            Ecosystem::Cargo,
            Ecosystem::Go,
            Ecosystem::NuGet,
            Ecosystem::Composer,
            Ecosystem::Oci,
        ] {
            assert_eq!(Ecosystem::from_slug(e.as_str()), Some(e));
        }
        assert_eq!(Ecosystem::from_slug("bogus"), None);
    }

    #[test]
    fn default_config_has_all_ecosystems() {
        let cfg = ProxyConfig::default();
        assert_eq!(cfg.upstreams.len(), 9);
        assert_eq!(cfg.mode, ProxyMode::ObserveOnly);
        assert!(cfg.upstreams.contains_key(&Ecosystem::PyPI));
        assert!(cfg.upstreams.contains_key(&Ecosystem::Oci));
    }

    #[test]
    fn blocklist_matches_ecosystem_name() {
        let mut cfg = ProxyConfig::default();
        cfg.blocklist.push("pypi/malicious-lib".to_string());
        let c = ProxyClient::new(cfg);
        assert!(c.is_blocked(Ecosystem::PyPI, "malicious-lib"));
        assert!(!c.is_blocked(Ecosystem::PyPI, "safe-lib"));
        assert!(!c.is_blocked(Ecosystem::Npm, "malicious-lib"));
    }

    #[test]
    fn proxy_off_refuses_fetch() {
        let mut cfg = ProxyConfig::default();
        cfg.mode = ProxyMode::Off;
        let c = ProxyClient::new(cfg);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let err = rt
            .block_on(c.fetch(Ecosystem::PyPI, "requests", None, "requests/"))
            .unwrap_err();
        assert!(matches!(err, ProxyError::ProxyOff));
    }

    #[test]
    fn rewrite_urls_for_pypi() {
        let c = ProxyClient::new(ProxyConfig::default());
        let html = r#"<a href="https://files.pythonhosted.org/packages/ab/cd/requests-2.31.0.tar.gz">requests-2.31.0.tar.gz</a>"#;
        let rewritten = c.rewrite_urls(Ecosystem::PyPI, html, "cave-registry.caveplatform.dev");
        assert!(rewritten.contains("https://cave-registry.caveplatform.dev/api/registry/pypi/blob"));
        assert!(!rewritten.contains("files.pythonhosted.org"));
    }

    #[test]
    fn rewrite_urls_leaves_unknown_ecosystems_alone() {
        let c = ProxyClient::new(ProxyConfig::default());
        let src = "https://example.com/foo";
        let out = c.rewrite_urls(Ecosystem::Maven, src, "host");
        assert_eq!(out, src);
    }
}
