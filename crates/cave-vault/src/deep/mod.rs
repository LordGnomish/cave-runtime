//! Deeper, sans-IO state-machine modules layered on top of the existing vault
//! engines. These are unit-testable in isolation and mirror the OpenBao /
//! HashiCorp Vault data models documented at
//! <https://openbao.org/docs/secrets/> and
//! <https://developer.hashicorp.com/vault/docs/secrets>.
//!
//! * [`lease_engine`] — TTL + renewal + revocation tree.
//! * [`dynamic_secret`] — generic dynamic-credential issuance machine.
//! * [`kv_versioning`] — KV-v2 versioning, soft-delete, destroy.
//! * [`transit_envelope`] — DEK / KEK envelope encryption with key rotation.
//! * [`pki_chain`] — issuer chain + CRL state.
//! * [`tenant`] — default-deny tenant ACL.

pub mod dynamic_secret;
pub mod kv_versioning;
pub mod lease_engine;
pub mod pki_chain;
pub mod tenant;
pub mod transit_envelope;
