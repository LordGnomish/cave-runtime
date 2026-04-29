//! cave-pki — sovereign internal CA hierarchy + chain validation +
//! CRL/OCSP responder.
//!
//! Hierarchy (cite: NIST SP 800-57 Part 1 Rev. 5 §5.3.4 — operator-
//! controlled CA hierarchies; openbao `vault/seal_autoseal.go::root`):
//!
//! ```text
//! Root CA (offline, hardware-backed key)
//!   └─ Platform Intermediate CA (cave-runtime control plane)
//!        └─ Per-tenant Intermediate CA (one per tenant_id)
//!             └─ Leaf certificates (workload identities)
//! ```
//!
//! Multi-tenant: every per-tenant intermediate is bound to its
//! `tenant_id`. Cross-tenant signing requests return
//! `PkiError::CrossTenantDenied`.

pub mod ca;
pub mod chain;
pub mod crl;
pub mod error;
pub mod ocsp;

pub use ca::{Ca, CaKind, CertHandle, KeyAlgorithm};
pub use chain::{ChainValidator, ValidationResult};
pub use crl::{CrlEntry, CrlResponder, RevocationReason};
pub use error::{PkiError, PkiResult};
pub use ocsp::{OcspResponder, OcspStatus};

pub const MODULE_NAME: &str = "pki";
