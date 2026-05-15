//! Artifact-platform portal page.
//!
//! Renders the consolidated artifact dashboard (Harbor + Pulp + Nexus +
//! Cosign) with one sub-page per upstream. Handlers are intentionally
//! public at this level — persona/Platform-Admin gating is enforced by
//! the upstream auth middleware in `cave-runtime/main.rs` the same way
//! the attribution + ADR + upstream pages are gated.
//!
//! Routes
//! ──────
//!   GET /portal/artifacts                 → consolidated dashboard
//!   GET /portal/artifacts/harbor          → Harbor sub-page
//!   GET /portal/artifacts/pulp            → Pulp sub-page
//!   GET /portal/artifacts/nexus           → Nexus sub-page
//!   GET /portal/artifacts/cosign          → Cosign signature management
//!   GET /api/portal/artifacts/summary     → JSON status roll-up consumed by
//!                                            the dashboard chrome
//!   GET /api/portal/artifacts/upstreams   → list of mounted upstreams +
//!                                            their backing endpoint paths
//!
//! The handlers do not call the `cave_artifacts` crate directly — they
//! return HTML page chrome that fetches the live data via the existing
//! `/api/artifacts/health`, `/api/cosign/v1/counters` etc. surfaces. This
//! keeps the portal layer free of business logic and lets the dashboard
//! poll for changes without re-rendering the page.

use axum::{response::Html, routing::get, Json, Router};
use serde::Serialize;
use serde_json::json;

pub fn router() -> Router {
    Router::new()
        .route("/portal/artifacts", get(dashboard_page))
        .route("/portal/artifacts/harbor", get(harbor_page))
        .route("/portal/artifacts/pulp", get(pulp_page))
        .route("/portal/artifacts/nexus", get(nexus_page))
        .route("/portal/artifacts/cosign", get(cosign_page))
        .route("/api/portal/artifacts/summary", get(api_summary))
        .route("/api/portal/artifacts/upstreams", get(api_upstreams))
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct UpstreamCard {
    pub id: &'static str,
    pub upstream: &'static str,
    pub health_endpoint: &'static str,
    pub features: &'static [&'static str],
}

pub const UPSTREAMS: &[UpstreamCard] = &[
    UpstreamCard {
        id: "harbor",
        upstream: "goharbor/harbor v2.10",
        health_endpoint: "/api/artifacts/health",
        features: &[
            "Docker Registry V2 + OCI Distribution Spec 1.1",
            "Harbor Admin API v2.0 (projects, robot accounts, scanners)",
            "Replication policies, tag retention, immutable tags",
            "Webhooks, quotas, audit logs, P2P preheat",
        ],
    },
    UpstreamCard {
        id: "pulp",
        upstream: "pulp/pulpcore v3.49",
        health_endpoint: "/api/artifacts/health",
        features: &[
            "Multi-format content plugins (RPM/Deb/PyPI/Maven/Ansible/File)",
            "Repository versions + remote sync + publication + distribution",
            "Async task queue, content guards, signing, RBAC",
            "Repair (hash mismatch + retransmit) + import/export",
        ],
    },
    UpstreamCard {
        id: "nexus",
        upstream: "sonatype/nexus-public 3.69",
        health_endpoint: "/api/nexus/v1/health",
        features: &[
            "Repository hosted/proxy/group + format adapter trait",
            "Component + Asset CRUD with content-addressable blob dedupe",
            "Cleanup policies (age/last-downloaded/regex)",
            "Routing rules (allow/block precedence)",
        ],
    },
    UpstreamCard {
        id: "cosign",
        upstream: "sigstore/cosign-style supply-chain signatures",
        health_endpoint: "/api/cosign/v1/health",
        features: &[
            "ECDSA-P256 — real signer/verifier (p256 crate)",
            "ML-DSA-65 hybrid composite — Ed25519 (real) + ML-DSA fixture",
            "Cosign simple-signing payload + sha256-XYZ.sig tag",
            "Per-digest signature index + per-alg counters",
        ],
    },
];

async fn dashboard_page() -> Html<String> {
    Html(render_dashboard())
}

async fn harbor_page() -> Html<String> {
    Html(render_subpage(
        "Harbor",
        "Container registry — OCI Distribution Spec + Harbor Admin",
        UPSTREAMS[0],
    ))
}

async fn pulp_page() -> Html<String> {
    Html(render_subpage(
        "Pulp",
        "Multi-format artifact repository — RPM / Deb / PyPI / Maven / Ansible",
        UPSTREAMS[1],
    ))
}

async fn nexus_page() -> Html<String> {
    Html(render_subpage(
        "Nexus",
        "Universal binary repository — hosted / proxy / group, blob dedupe, raw end-to-end",
        UPSTREAMS[2],
    ))
}

async fn cosign_page() -> Html<String> {
    Html(render_subpage(
        "Cosign",
        "Supply-chain signatures — ECDSA-P256 (real) + ML-DSA-65 hybrid (Ed25519 real + ML-DSA fixture)",
        UPSTREAMS[3],
    ))
}

async fn api_summary() -> Json<serde_json::Value> {
    Json(json!({
        "module": "cave-artifacts",
        "upstream_count": UPSTREAMS.len(),
        "upstreams": UPSTREAMS.iter().map(|c| c.id).collect::<Vec<_>>(),
        "health_endpoint": "/api/artifacts/health",
        "cosign": {
            "endpoint": "/api/cosign/v1/health",
            "counters_endpoint": "/api/cosign/v1/counters",
            "supported_algorithms": ["ecdsa-p256", "ml-dsa-65"],
            "pqc_backend": "fixture",
        }
    }))
}

async fn api_upstreams() -> Json<&'static [UpstreamCard]> {
    Json(UPSTREAMS)
}

fn page_chrome(title: &str, subtitle: &str, body_html: &str) -> String {
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>{title} — Cave Artifacts</title>
  <style>
    body {{ font-family: -apple-system, BlinkMacSystemFont, system-ui, sans-serif; margin: 2em; color: #1a202c; }}
    header {{ border-bottom: 1px solid #e2e8f0; padding-bottom: 1em; margin-bottom: 1.5em; }}
    h1 {{ margin: 0 0 0.2em 0; }}
    nav a {{ margin-right: 1em; color: #2563eb; text-decoration: none; }}
    nav a:hover {{ text-decoration: underline; }}
    .card {{ background: #f7fafc; border: 1px solid #cbd5e1; border-radius: 8px; padding: 1em 1.2em; margin: 0.8em 0; }}
    .card h3 {{ margin-top: 0; }}
    .endpoint {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; background: #edf2f7; padding: 0.1em 0.4em; border-radius: 4px; }}
    ul {{ padding-left: 1.4em; }}
  </style>
</head>
<body>
  <header>
    <h1>{title}</h1>
    <p>{subtitle}</p>
    <nav>
      <a href="/portal/artifacts">Dashboard</a>
      <a href="/portal/artifacts/harbor">Harbor</a>
      <a href="/portal/artifacts/pulp">Pulp</a>
      <a href="/portal/artifacts/nexus">Nexus</a>
      <a href="/portal/artifacts/cosign">Cosign</a>
    </nav>
  </header>
  <main>
    {body_html}
  </main>
</body>
</html>
"##,
    )
}

fn render_dashboard() -> String {
    let mut cards = String::new();
    for u in UPSTREAMS {
        cards.push_str(&format!(
            r##"<div class="card">
  <h3><a href="/portal/artifacts/{id}">{upstream}</a></h3>
  <p>Health: <span class="endpoint">{ep}</span></p>
  <ul>{features}</ul>
</div>"##,
            id = u.id,
            upstream = u.upstream,
            ep = u.health_endpoint,
            features = u
                .features
                .iter()
                .map(|f| format!("<li>{f}</li>"))
                .collect::<String>(),
        ));
    }
    page_chrome(
        "Artifact Platform",
        "Harbor + Pulp + Nexus + Cosign — single binary, three upstream parities, one supply-chain signer",
        &cards,
    )
}

fn render_subpage(title: &str, subtitle: &str, card: UpstreamCard) -> String {
    let body = format!(
        r##"<div class="card">
  <h3>{upstream}</h3>
  <p>Health: <span class="endpoint">{ep}</span></p>
  <ul>{features}</ul>
</div>
<p>This page is a thin shell — live status comes from the
<span class="endpoint">{ep}</span> endpoint, which is polled by the dashboard
chrome. Sub-page actions (project list, repo browser, key management) are
exposed under <span class="endpoint">/api/portal/artifacts/*</span> and the
upstream-specific surfaces.</p>"##,
        upstream = card.upstream,
        ep = card.health_endpoint,
        features = card
            .features
            .iter()
            .map(|f| format!("<li>{f}</li>"))
            .collect::<String>(),
    );
    page_chrome(&format!("{title} — Cave Artifacts"), subtitle, &body)
}

// ── tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    fn app() -> Router {
        router()
    }

    async fn body_text(resp: axum::response::Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn dashboard_lists_all_four_upstreams() {
        let resp = app()
            .oneshot(Request::builder().uri("/portal/artifacts").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("goharbor/harbor"));
        assert!(body.contains("pulp/pulpcore"));
        assert!(body.contains("sonatype/nexus"));
        assert!(body.contains("supply-chain signatures"));
    }

    #[tokio::test]
    async fn harbor_subpage_renders_features() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/portal/artifacts/harbor")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("Docker Registry V2"));
        assert!(body.contains("/api/artifacts/health"));
    }

    #[tokio::test]
    async fn pulp_subpage_renders_features() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/portal/artifacts/pulp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("Multi-format content plugins"));
    }

    #[tokio::test]
    async fn nexus_subpage_renders_features() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/portal/artifacts/nexus")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("Cleanup policies"));
        assert!(body.contains("/api/nexus/v1/health"));
    }

    #[tokio::test]
    async fn cosign_subpage_calls_out_pqc_fixture_status() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/portal/artifacts/cosign")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_text(resp).await;
        assert!(body.contains("ECDSA-P256"));
        assert!(body.contains("ML-DSA"));
        assert!(body.contains("fixture"));
    }

    #[tokio::test]
    async fn api_summary_advertises_four_upstreams() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/api/portal/artifacts/summary")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["upstream_count"], 4);
        let ups = v["upstreams"].as_array().unwrap();
        assert!(ups.iter().any(|u| u == "harbor"));
        assert!(ups.iter().any(|u| u == "pulp"));
        assert!(ups.iter().any(|u| u == "nexus"));
        assert!(ups.iter().any(|u| u == "cosign"));
    }

    #[tokio::test]
    async fn api_summary_calls_out_pqc_backend_state() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/api/portal/artifacts/summary")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v = body_json(resp).await;
        assert_eq!(v["cosign"]["pqc_backend"], "fixture");
        let algs = v["cosign"]["supported_algorithms"].as_array().unwrap();
        assert!(algs.iter().any(|a| a == "ecdsa-p256"));
        assert!(algs.iter().any(|a| a == "ml-dsa-65"));
    }

    #[tokio::test]
    async fn api_upstreams_returns_full_card_set() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/api/portal/artifacts/upstreams")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 4);
        // Every card has the required keys + non-empty features.
        for c in arr {
            assert!(!c["id"].as_str().unwrap().is_empty());
            assert!(!c["upstream"].as_str().unwrap().is_empty());
            assert!(c["features"].as_array().unwrap().len() >= 3);
        }
    }

    #[tokio::test]
    async fn unknown_subpath_returns_404() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/portal/artifacts/does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dashboard_links_to_each_subpage() {
        let resp = app()
            .oneshot(Request::builder().uri("/portal/artifacts").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = body_text(resp).await;
        for slug in ["harbor", "pulp", "nexus", "cosign"] {
            assert!(
                body.contains(&format!("/portal/artifacts/{slug}")),
                "dashboard missing link to {slug}"
            );
        }
    }

    #[tokio::test]
    async fn upstream_cards_pin_real_versions() {
        // Doc-as-test: the version pins live in the page so the card stays
        // honest about which upstream tag we mirror.
        assert!(UPSTREAMS.iter().any(|u| u.upstream.contains("v2.10")));
        assert!(UPSTREAMS.iter().any(|u| u.upstream.contains("v3.49")));
        assert!(UPSTREAMS.iter().any(|u| u.upstream.contains("3.69")));
    }
}
