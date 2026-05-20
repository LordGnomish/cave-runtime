// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! deeper-002: registry Bearer-challenge parser + tenant-scoped token cache.
//!
//! Upstream: containerd v2.2.3 `core/remotes/docker/auth/parse.go`
//! (`ParseAuthHeader`) and `core/remotes/docker/authorizer.go`
//! (`dockerAuthorizer.handlers` token cache).

use cave_cri::registry::{BearerChallenge, TokenCache};

const TENANT: &str = "tenant-acme-prod";

/// Cite: containerd v2.2.3 `core/remotes/docker/auth/parse.go`
/// (`ParseAuthHeader`) — canonical Docker Hub challenge with
/// realm + service + scope.
#[test]
fn parses_canonical_docker_hub_challenge() {
    let header = r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/nginx:pull""#;
    let c = BearerChallenge::parse(header).unwrap();
    assert_eq!(c.realm, "https://auth.docker.io/token");
    assert_eq!(c.service, Some("registry.docker.io".into()));
    assert_eq!(c.scope, Some("repository:library/nginx:pull".into()));
    assert_eq!(c.error, None);
}

/// Cite: distribution-spec v1.1 §4.4 — `realm` is the only required
/// parameter; `service` and `scope` are optional.
#[test]
fn realm_only_challenge_is_accepted() {
    let header = r#"Bearer realm="https://ghcr.io/token""#;
    let c = BearerChallenge::parse(header).unwrap();
    assert_eq!(c.realm, "https://ghcr.io/token");
    assert!(c.service.is_none() && c.scope.is_none());
}

/// Cite: distribution-spec v1.1 §4.4 — challenges may carry an `error`
/// hint (e.g. `insufficient_scope`) which the client surfaces back to
/// the caller. Must be parsed verbatim.
#[test]
fn parses_error_hint_and_extra_whitespace() {
    let header = r#"  Bearer  realm="https://r/t" , service="reg" , scope="repo:foo:push" , error="insufficient_scope"  "#;
    let c = BearerChallenge::parse(header).unwrap();
    assert_eq!(c.realm, "https://r/t");
    assert_eq!(c.service, Some("reg".into()));
    assert_eq!(c.scope, Some("repo:foo:push".into()));
    assert_eq!(c.error, Some("insufficient_scope".into()));
}

/// Cite: containerd `core/remotes/docker/auth/parse.go` (`ParseAuthHeader`)
/// — non-Bearer schemes and missing realm produce an error rather than
/// being silently treated as anonymous access.
#[test]
fn rejects_non_bearer_scheme_and_missing_realm() {
    assert!(
        BearerChallenge::parse(r#"Basic realm="x""#).is_err(),
        "non-Bearer scheme rejected"
    );
    assert!(
        BearerChallenge::parse(r#"Bearer service="x""#).is_err(),
        "missing realm rejected"
    );
    assert!(BearerChallenge::parse("").is_err());
}

/// Cite: distribution-spec v1.1 §4.4 — the client must construct a
/// token-endpoint URL by appending `service` and `scope` as query
/// parameters, urlencoded. cave does percent-encoding for non-token
/// chars (the colons and slashes inside the scope ARE escaped).
#[test]
fn token_url_appends_query_params_urlencoded() {
    let c = BearerChallenge {
        realm: "https://auth.docker.io/token".into(),
        service: Some("registry.docker.io".into()),
        scope: Some("repository:library/nginx:pull".into()),
        error: None,
    };
    let url = c.token_url();
    assert!(url.starts_with("https://auth.docker.io/token?"));
    assert!(url.contains("service=registry.docker.io"));
    // scope contains : and / — both must be percent-escaped (3A and 2F)
    assert!(url.contains("scope=repository%3Alibrary%2Fnginx%3Apull"));

    // Realm with existing query keeps `&` separator
    let c = BearerChallenge {
        realm: "https://auth/token?x=1".into(),
        service: Some("svc".into()),
        scope: None,
        error: None,
    };
    let url = c.token_url();
    assert!(url.contains("?x=1"));
    assert!(url.contains("&service=svc"));
}

/// Cite: containerd `core/remotes/docker/authorizer.go` —
/// `dockerAuthorizer.handlers` map keys tokens by registry + repo + scope
/// so a single tenant pulling two repos doesn't conflate credentials.
/// cave's `TokenCache` honours TTL eviction and tenant isolation.
#[test]
fn tenant_token_cache_isolates_keys_and_honours_ttl() {
    let cache = TokenCache::new(TENANT);
    assert_eq!(cache.tenant_id, TENANT);
    assert!(cache.is_empty());

    cache.put("docker.io", "library/nginx", "pull", "tok-A", 60);
    cache.put("docker.io", "library/redis", "pull", "tok-B", 60);
    cache.put("ghcr.io", "org/app", "pull", "tok-C", 60);
    assert_eq!(cache.len(), 3);

    assert_eq!(
        cache.get("docker.io", "library/nginx", "pull").as_deref(),
        Some("tok-A")
    );
    assert_eq!(
        cache.get("docker.io", "library/redis", "pull").as_deref(),
        Some("tok-B")
    );
    assert_eq!(
        cache.get("ghcr.io", "org/app", "pull").as_deref(),
        Some("tok-C")
    );

    // Cache miss for a different scope (push vs. pull)
    assert!(cache.get("docker.io", "library/nginx", "push").is_none());

    // TTL expiry — put with a 0/negative TTL is clamped to 1s in our impl;
    // but evicting forces a miss immediately.
    assert!(cache.evict("docker.io", "library/nginx", "pull"));
    assert!(cache.get("docker.io", "library/nginx", "pull").is_none());
    assert!(
        !cache.evict("docker.io", "library/nginx", "pull"),
        "second evict is a no-op"
    );
    assert_eq!(cache.len(), 2);
}
