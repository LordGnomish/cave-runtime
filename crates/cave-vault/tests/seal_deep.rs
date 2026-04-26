//! deeper-001: Seal/unseal — Shamir threshold validation, double-init
//! rejection, auto-unseal interface (Transit / KMIP / Azure Key Vault /
//! AWS KMS / GCP KMS / OCI KMS). Pinned to openbao v2.5.3.

use cave_vault::core::seal::{
    AutoSealConfig, AutoSealType, SealState, SealStatus,
};

const TENANT: &str = "tenant-acme-prod";

/// Cite: openbao `sdk/helper/shamir/shamir.go:192` (Split, parameter
/// validation) — `threshold` must be ≥ 2 and ≤ shares; shares must be ≥ 1.
#[test]
fn validate_threshold_rejects_illegal_combinations() {
    assert!(SealState::validate_threshold(0, 0).is_err(), "shares=0 rejected");
    assert!(SealState::validate_threshold(5, 0).is_err(), "threshold=0 rejected");
    assert!(SealState::validate_threshold(5, 1).is_err(),
        "threshold=1 (degenerate) rejected");
    assert!(SealState::validate_threshold(3, 5).is_err(),
        "threshold > shares rejected");
    assert!(SealState::validate_threshold(5, 3).is_ok());
    assert!(SealState::validate_threshold(5, 5).is_ok(), "k=n is valid");
}

/// Cite: openbao `vault/seal.go::defaultSeal::Init` — initializing an
/// already-initialized vault returns `ErrAlreadyInitialized`.
#[test]
fn double_initialize_rejected_with_already_initialized() {
    let mut s = SealState::default();
    let _ = s.initialize(5, 3).unwrap();
    let err = s.initialize(5, 3).unwrap_err();
    assert_eq!(err.to_string(), "Vault is already initialized");
    let _tenant = TENANT;
}

/// Cite: openbao `vault/seal.go` unseal flow — calling `unseal()` before
/// `initialize()` returns `ErrNotInitialized`. Sealed-but-uninitialized
/// is a real state distinct from "no shares submitted yet".
#[test]
fn unseal_before_initialize_returns_not_initialized() {
    let mut s = SealState::default();
    assert!(!s.is_initialized());
    assert!(s.is_sealed());
    let err = s.unseal("00aabbccdd").unwrap_err();
    assert_eq!(err.to_string(), "Vault is not initialized");
}

/// Cite: openbao `vault/seal.go` unseal flow — submitting a non-hex
/// payload as a share yields `InvalidRequest`, NOT a panic.
#[test]
fn unseal_with_garbage_share_returns_invalid_request() {
    let mut s = SealState::default();
    let _ = s.initialize(5, 3).unwrap();
    let err = s.unseal("not-hex-zzz").unwrap_err();
    assert!(err.to_string().contains("invalid share encoding"));
    assert_eq!(s.unseal_progress, 0, "rejected share does not consume progress");
    assert_eq!(s.status, SealStatus::Sealed);
}

/// Cite: openbao `vault/seal_autoseal.go:42`
/// (`barrierType wrapping.WrapperType`) — every supported KMS backend
/// has a canonical wrapping type identifier emitted in the seal status.
#[test]
fn auto_seal_type_barrier_identifiers_match_openbao_naming() {
    use AutoSealType::*;
    assert_eq!(Transit.barrier_type(),       "transit");
    assert_eq!(AzureKeyVault.barrier_type(), "azurekeyvault");
    assert_eq!(AwsKms.barrier_type(),        "awskms");
    assert_eq!(GcpCkms.barrier_type(),       "gcpckms");
    assert_eq!(OciKms.barrier_type(),        "ocikms");
    assert_eq!(Pkcs11.barrier_type(),        "pkcs11");
    assert_eq!(Shamir.barrier_type(),        "shamir");

    // StoredKeysSupported / RecoveryKeySupported — Shamir is the only
    // type that does NEITHER.
    assert!(!Shamir.stores_keys_remotely());
    assert!(!Shamir.supports_recovery_key());
    for t in [Transit, AzureKeyVault, AwsKms, GcpCkms, OciKms, Pkcs11] {
        assert!(t.stores_keys_remotely(), "{} stores keys remotely", t.barrier_type());
        assert!(t.supports_recovery_key(), "{} supports recovery key", t.barrier_type());
    }
}

/// Cite: openbao `vault/seal.go:329` (SealConfig.baseValidate) +
/// `vault/seal_autoseal.go::Init` — non-Shamir auto-seal configs MUST
/// declare a non-empty endpoint and a recovery quorum; Shamir configs
/// MUST NOT carry recovery shares (they use the regular Shamir threshold).
#[test]
fn auto_seal_config_validation_per_backend() {
    // Shamir: rejects recovery shares (none allowed).
    let bad_shamir = AutoSealConfig {
        seal_type: AutoSealType::Shamir,
        recovery_shares: 5,
        recovery_threshold: 3,
        endpoint: String::new(),
        key_id: String::new(),
    };
    assert!(bad_shamir.validate().is_err());

    let ok_shamir = AutoSealConfig {
        seal_type: AutoSealType::Shamir,
        recovery_shares: 0,
        recovery_threshold: 0,
        endpoint: String::new(),
        key_id: String::new(),
    };
    assert!(ok_shamir.validate().is_ok());

    // Transit auto-seal: missing endpoint ⇒ rejected.
    let bad_transit = AutoSealConfig {
        seal_type: AutoSealType::Transit,
        recovery_shares: 5,
        recovery_threshold: 3,
        endpoint: String::new(),
        key_id: format!("{}-master", TENANT),
    };
    assert!(bad_transit.validate().is_err());

    // Auto-seal: threshold > shares ⇒ rejected.
    let bad_threshold = AutoSealConfig {
        seal_type: AutoSealType::AzureKeyVault,
        recovery_shares: 3,
        recovery_threshold: 5,
        endpoint: "https://vault.azure.net".into(),
        key_id: format!("{}-master", TENANT),
    };
    assert!(bad_threshold.validate().is_err());

    // Happy path
    let ok = AutoSealConfig {
        seal_type: AutoSealType::Transit,
        recovery_shares: 5,
        recovery_threshold: 3,
        endpoint: "https://upstream-vault.internal:8200".into(),
        key_id: format!("{}-master", TENANT),
    };
    assert!(ok.validate().is_ok());
}
