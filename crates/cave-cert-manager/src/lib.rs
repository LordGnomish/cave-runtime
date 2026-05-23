// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! cave-cert-manager — Kubernetes-style cert-manager control plane.
//!
//! Upstream parity: `cert-manager/cert-manager` v1.20.2
//! (`e5b7b18450dd2c4b993b95bcd680b1a057205b00`). Provides the
//! Certificate / CertificateRequest / Issuer / ClusterIssuer CRDs, an
//! issuer registry covering ACME (HTTP-01 + DNS-01), CA (self-signed
//! root + intermediate via [`cave_pki`]), Vault (keychain-read token),
//! and SelfSigned, a renewal scheduler that respects `renewBefore`, a
//! secret reconciler that materialises certificates into Kubernetes
//! Secrets, an HTTP API surface, and a `cavectl cert` driver.
//!
//! The ACME state machine is driven through [`cave_acme`] — this crate
//! owns the Certificate ↔ Order projection and the challenge solver
//! (HTTP-01 token publisher + DNS-01 TXT record planner) only.

pub mod acme_issuer;
pub mod ca_issuer;
pub mod cli;
pub mod controller;
pub mod error;
pub mod issuer;
pub mod models;
pub mod renewal;
pub mod routes;
pub mod secret;
pub mod selfsigned_issuer;
pub mod store;
pub mod vault_issuer;

pub use error::{CertManagerError, CertManagerResult};
pub use models::{
    Certificate, CertificateCondition, CertificateConditionType, CertificateRequest,
    CertificateRequestCondition, CertificateRequestStatus, CertificateSpec, CertificateStatus,
    ClusterIssuer, ConditionStatus, IssuerKind, IssuerRef, IssuerResource, IssuerSpec, KeyAlgo,
    KeyEncoding, KeySize, PrivateKeyPolicy, RotationPolicy, SecretRef, Usage, X509Subject,
};
pub use issuer::{IssuerRegistry, IssueOutcome};
pub use renewal::{RenewalPlan, RenewalScheduler};
pub use secret::{SecretMaterializer, SecretRecord};
pub use store::CertManagerStore;

pub const MODULE_NAME: &str = "cert-manager";
pub const UPSTREAM_VERSION: &str = "v1.20.2";
pub const UPSTREAM_SOURCE_SHA: &str = "e5b7b18450dd2c4b993b95bcd680b1a057205b00";
