// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-certs — DNS-01 + HTTP-01 solver tests.

use cave_acme::{Challenge, ChallengeStatus, ChallengeType, Jwk};
use cave_certs::solvers::{Dns01Solver, Http01Solver};

const TENANT: &str = "tenant-acme-prod";

fn jwk() -> Jwk {
    Jwk::EC { crv: "P-256".into(),
        x: "f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU".into(),
        y: "x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0".into() }
}

fn challenge(kind: ChallengeType, token: &str) -> Challenge {
    Challenge {
        id: format!("ch-{}-{}", kind.as_str(), token),
        kind, status: ChallengeStatus::Pending,
        url: format!("/acme/chall/{}/{}", token, kind.as_str()),
        token: token.into(), validated_at: None, error: None,
    }
}

/// Cite: RFC 8555 §8.4 (DNS-01) + cert-manager
/// `pkg/issuer/acme/dns/dns.go::Present` — `present()` writes the
/// `_acme-challenge.<domain>` TXT record; `cleanup()` removes it.
#[test]
fn dns01_solver_present_then_cleanup_round_trip() {
    let mut s = Dns01Solver::new(TENANT);
    let domain = format!("svc.{}.cave-runtime.test", TENANT);
    let ch = challenge(ChallengeType::Dns01, "TOK-dns-1");

    let record = s.present(&domain, &ch, &jwk()).unwrap();
    assert_eq!(record, format!("_acme-challenge.{}", domain));
    assert_eq!(s.len(), 1);
    let value = s.lookup(&record).unwrap();
    assert_eq!(value.len(), 43, "base64url SHA-256 = 43 chars");

    assert!(s.cleanup(&domain), "first cleanup removes the record");
    assert!(!s.cleanup(&domain), "second cleanup is a no-op");
    assert!(s.is_empty());
}

/// Cite: solver type discipline — passing a non-DNS-01 challenge to
/// the DNS-01 solver MUST fail rather than silently misroute.
#[test]
fn dns01_solver_rejects_wrong_challenge_type() {
    let mut s = Dns01Solver::new(TENANT);
    let ch = challenge(ChallengeType::Http01, "TOK");
    let err = s.present("svc.example.test", &ch, &jwk()).unwrap_err();
    assert!(err.contains("Http01"));
}

/// Cite: RFC 8555 §8.3 (HTTP-01) — the solver mounts the keyAuth at
/// `/.well-known/acme-challenge/<token>` and serves it as the bare
/// response body.
#[test]
fn http01_solver_present_serve_then_cleanup() {
    let mut s = Http01Solver::new(TENANT);
    let ch = challenge(ChallengeType::Http01, "TOK-http-1");

    let path = s.present(&ch, &jwk()).unwrap();
    assert_eq!(path, "/.well-known/acme-challenge/TOK-http-1");
    let body = s.serve(&path).unwrap();
    // Body must be the bare keyAuth (token + "." + thumbprint).
    assert!(body.starts_with("TOK-http-1."));
    assert!(body.contains(&jwk().thumbprint()));

    assert!(s.cleanup(&ch));
    assert!(s.serve(&path).is_none(), "GET on cleaned-up path returns nothing");
}
