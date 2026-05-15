use crate::error::{VaultError, VaultResult};
use base64::Engine as _;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

/// GF(256) field operations using the AES polynomial (x^8+x^4+x^3+x+1)
fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut result: u8 = 0;
    let mut high_bit;
    for _ in 0..8 {
        if b & 1 != 0 {
            result ^= a;
        }
        high_bit = a & 0x80;
        a <<= 1;
        if high_bit != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    result
}

fn gf_div(a: u8, b: u8) -> u8 {
    if b == 0 { panic!("division by zero in GF(256)"); }
    if a == 0 { return 0; }
    gf_mul(a, gf_inv(b))
}

fn gf_inv(a: u8) -> u8 {
    if a == 0 { return 0; }
    let mut r: u8 = 1;
    let mut base = a;
    let mut exp: u8 = 254;
    while exp > 0 {
        if exp & 1 != 0 {
            r = gf_mul(r, base);
        }
        base = gf_mul(base, base);
        exp >>= 1;
    }
    r
}

/// Split `secret` bytes into `n` shares of which any `k` can reconstruct.
pub fn split_secret(secret: &[u8], n: u8, k: u8) -> VaultResult<Vec<Vec<u8>>> {
    if k < 2 || k > n {
        return Err(VaultError::InvalidRequest("invalid share parameters".into()));
    }
    let rng = SystemRandom::new();
    let mut shares: Vec<Vec<u8>> = (1..=n).map(|i| vec![i]).collect();

    for &secret_byte in secret {
        let mut coeffs = vec![secret_byte];
        let mut rand_bytes = vec![0u8; (k - 1) as usize];
        rng.fill(&mut rand_bytes).map_err(|_| VaultError::Crypto("rng failure".into()))?;
        coeffs.extend_from_slice(&rand_bytes);

        for (i, share) in shares.iter_mut().enumerate() {
            let x = (i + 1) as u8;
            let mut y = coeffs[coeffs.len() - 1];
            for &c in coeffs[..coeffs.len()-1].iter().rev() {
                y = gf_mul(y, x) ^ c;
            }
            share.push(y);
        }
    }
    Ok(shares)
}

/// Reconstruct secret from at least `k` shares using Lagrange interpolation.
pub fn combine_shares(shares: &[Vec<u8>]) -> VaultResult<Vec<u8>> {
    if shares.is_empty() {
        return Err(VaultError::InvalidRequest("no shares provided".into()));
    }
    let secret_len = shares[0].len() - 1;
    if shares.iter().any(|s| s.len() - 1 != secret_len) {
        return Err(VaultError::InvalidRequest("shares have inconsistent length".into()));
    }
    let xs: Vec<u8> = shares.iter().map(|s| s[0]).collect();
    let mut secret = Vec::with_capacity(secret_len);

    for byte_idx in 0..secret_len {
        let ys: Vec<u8> = shares.iter().map(|s| s[byte_idx + 1]).collect();
        let mut result: u8 = 0;
        for i in 0..xs.len() {
            let mut num: u8 = 1;
            let mut denom: u8 = 1;
            for j in 0..xs.len() {
                if i != j {
                    num = gf_mul(num, xs[j]);
                    denom = gf_mul(denom, xs[i] ^ xs[j]);
                }
            }
            result ^= gf_mul(ys[i], gf_div(num, denom));
        }
        secret.push(result);
    }
    Ok(secret)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SealStatus {
    Sealed,
    Unsealed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealState {
    pub status: SealStatus,
    pub initialized: bool,
    pub threshold: u8,
    pub shares: u8,
    pub unseal_progress: u8,
    pub unseal_nonce: String,
    #[serde(skip)]
    pub pending_shares: Vec<Vec<u8>>,
    #[serde(skip)]
    pub master_key: Option<Vec<u8>>,
    pub root_token: Option<String>,
    pub key_shares: Vec<String>,
}

impl Default for SealState {
    fn default() -> Self {
        Self {
            status: SealStatus::Sealed,
            initialized: false,
            threshold: 0,
            shares: 0,
            unseal_progress: 0,
            unseal_nonce: uuid::Uuid::new_v4().to_string(),
            pending_shares: Vec::new(),
            master_key: None,
            root_token: None,
            key_shares: Vec::new(),
        }
    }
}

impl SealState {
    pub fn is_sealed(&self) -> bool {
        self.status == SealStatus::Sealed
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn initialize(&mut self, shares: u8, threshold: u8) -> VaultResult<(String, Vec<String>)> {
        if self.initialized {
            return Err(VaultError::AlreadyInitialized);
        }
        Self::validate_threshold(shares, threshold)?;
        let rng = SystemRandom::new();
        let mut master_key = vec![0u8; 32];
        rng.fill(&mut master_key).map_err(|_| VaultError::Crypto("rng failure".into()))?;

        let share_bytes = split_secret(&master_key, shares, threshold)?;
        let hex_shares: Vec<String> = share_bytes.iter().map(|s| hex::encode(s)).collect();

        let mut root_token_bytes = vec![0u8; 16];
        rng.fill(&mut root_token_bytes).map_err(|_| VaultError::Crypto("rng failure".into()))?;
        let root_token = format!("hvs.{}", base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(&root_token_bytes));

        self.initialized = true;
        self.threshold = threshold;
        self.shares = shares;
        self.key_shares = hex_shares.clone();
        self.root_token = Some(root_token.clone());

        Ok((root_token, hex_shares))
    }

    pub fn unseal(&mut self, share_hex: &str) -> VaultResult<bool> {
        if !self.initialized {
            return Err(VaultError::NotInitialized);
        }
        if !self.is_sealed() {
            return Ok(false);
        }
        let share = hex::decode(share_hex)
            .map_err(|_| VaultError::InvalidRequest("invalid share encoding".into()))?;

        self.pending_shares.push(share);
        self.unseal_progress = self.pending_shares.len() as u8;

        if self.unseal_progress >= self.threshold {
            let master_key = combine_shares(&self.pending_shares)?;
            self.master_key = Some(master_key);
            self.pending_shares.clear();
            self.unseal_progress = 0;
            self.status = SealStatus::Unsealed;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn seal(&mut self) {
        self.status = SealStatus::Sealed;
        self.master_key = None;
        self.pending_shares.clear();
        self.unseal_progress = 0;
        self.unseal_nonce = uuid::Uuid::new_v4().to_string();
    }

    /// Cite: openbao `vault/seal.go` SealConfig.Validate +
    /// `sdk/helper/shamir/shamir.go:192` (Split) bounds — `n` must fit in
    /// `[1, 255]`, `k` must fit in `[1, n]`. cave additionally requires
    /// `k >= 2` to match the upstream Shamir implementation (a 1-of-n
    /// "scheme" is degenerate and rejected by openbao at the API layer).
    pub fn validate_threshold(shares: u8, threshold: u8) -> VaultResult<()> {
        if shares == 0 {
            return Err(VaultError::InvalidRequest("shares must be >= 1".into()));
        }
        if threshold < 2 {
            return Err(VaultError::InvalidRequest("threshold must be >= 2".into()));
        }
        if threshold > shares {
            return Err(VaultError::InvalidRequest(
                "threshold cannot exceed shares".into(),
            ));
        }
        Ok(())
    }
}

// ─── Auto-seal interface (deeper-001) ───────────────────────────────────────
//
// Cite: openbao `vault/seal_autoseal.go:39` (autoSeal struct) +
// `:55` (NewAutoSeal). The auto-seal flow delegates the master-key
// wrap/unwrap to a remote KMS backend; the local Shamir shares are
// replaced with recovery shares whose only job is to bootstrap the seal
// configuration after a quorum-loss event.

/// Recognised KMS backends for auto-unseal. Mirrors the wrapping types
/// declared in `github.com/openbao/go-kms-wrapping/v2` (see also
/// `vault/seal_autoseal.go:42` `barrierType wrapping.WrapperType`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoSealType {
    /// `vault/seal/transit` — wraps the master key via a remote Vault
    /// running the Transit secrets engine.
    Transit,
    /// `vault/seal/azurekeyvault` — Azure Key Vault HSM.
    AzureKeyVault,
    /// `vault/seal/awskms` — AWS KMS.
    AwsKms,
    /// `vault/seal/gcpckms` — GCP Cloud KMS.
    GcpCkms,
    /// `vault/seal/ocikms` — Oracle Cloud Infrastructure KMS.
    OciKms,
    /// `vault/seal/pkcs11` — KMIP / PKCS#11 HSM.
    Pkcs11,
    /// Built-in Shamir (no auto-unseal). Used when no `seal {}` block
    /// is configured.
    Shamir,
}

impl AutoSealType {
    /// Wrapping-type identifier emitted in the seal status payload.
    /// Cite: openbao `vault/seal_autoseal.go:104` (BarrierType).
    pub fn barrier_type(&self) -> &'static str {
        match self {
            Self::Transit       => "transit",
            Self::AzureKeyVault => "azurekeyvault",
            Self::AwsKms        => "awskms",
            Self::GcpCkms       => "gcpckms",
            Self::OciKms        => "ocikms",
            Self::Pkcs11        => "pkcs11",
            Self::Shamir        => "shamir",
        }
    }

    /// Cite: `vault/seal_autoseal.go:113` `StoredKeysSupported() ⇒ Generic`.
    /// Every auto-seal backend stores the master key remotely; only
    /// Shamir keeps it derived from local shares.
    pub fn stores_keys_remotely(&self) -> bool {
        !matches!(self, Self::Shamir)
    }

    /// Cite: `vault/seal_autoseal.go:116` `RecoveryKeySupported() ⇒ true`.
    /// Auto-seal types replace the unseal flow with a recovery-key
    /// flow whose threshold is configured separately.
    pub fn supports_recovery_key(&self) -> bool {
        self.stores_keys_remotely()
    }
}

/// Cite: openbao `vault/seal.go:279` (SealConfig). cave's view is the
/// subset of fields driven by the operator config + the unseal RPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoSealConfig {
    pub seal_type: AutoSealType,
    /// Number of recovery shares (only meaningful when
    /// `seal_type.supports_recovery_key()`).
    pub recovery_shares: u8,
    /// Threshold for recovery-share quorum.
    pub recovery_threshold: u8,
    /// Backend-specific endpoint (Vault Transit address, Azure Key Vault
    /// URL, etc).  Opaque to cave; the wrapper validates it.
    pub endpoint: String,
    /// Optional named key inside the backend (e.g. transit key name,
    /// Azure Key Vault key id).
    pub key_id: String,
}

impl AutoSealConfig {
    pub fn validate(&self) -> VaultResult<()> {
        if self.seal_type == AutoSealType::Shamir {
            if self.recovery_shares != 0 || self.recovery_threshold != 0 {
                return Err(VaultError::InvalidRequest(
                    "Shamir seal does not use recovery shares".into(),
                ));
            }
            return Ok(());
        }
        if self.endpoint.trim().is_empty() {
            return Err(VaultError::InvalidRequest(
                "auto-seal endpoint must not be empty".into(),
            ));
        }
        if self.recovery_shares == 0 {
            return Err(VaultError::InvalidRequest(
                "auto-seal requires at least 1 recovery share".into(),
            ));
        }
        if self.recovery_threshold < 1 || self.recovery_threshold > self.recovery_shares {
            return Err(VaultError::InvalidRequest(
                "recovery threshold must be in [1, recovery_shares]".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shamir_round_trip_2_of_3() {
        let secret = b"my secret master key 32 bytes!!";
        let shares = split_secret(secret, 3, 2).unwrap();
        assert_eq!(shares.len(), 3);

        // reconstruct from any 2
        let reconstructed = combine_shares(&shares[0..2]).unwrap();
        assert_eq!(reconstructed, secret);

        let reconstructed2 = combine_shares(&shares[1..3]).unwrap();
        assert_eq!(reconstructed2, secret);
    }

    #[test]
    fn test_shamir_round_trip_3_of_5() {
        let secret = b"another secret value for testing";
        let shares = split_secret(secret, 5, 3).unwrap();
        assert_eq!(shares.len(), 5);

        let reconstructed = combine_shares(&[shares[0].clone(), shares[2].clone(), shares[4].clone()]).unwrap();
        assert_eq!(reconstructed, secret);
    }

    #[test]
    fn test_split_combine_roundtrip() {
        let secret = b"hello shamir secret bytes XX";
        let shares = split_secret(secret, 4, 3).unwrap();
        assert_eq!(shares.len(), 4);
        let combined = combine_shares(&shares[..3]).unwrap();
        assert_eq!(combined, secret);
    }

    #[test]
    fn test_split_combine_threshold_2() {
        let secret = b"k=2 minimum threshold";
        let shares = split_secret(secret, 5, 2).unwrap();
        let combined = combine_shares(&shares[..2]).unwrap();
        assert_eq!(combined, secret);
    }

    #[test]
    fn test_gf_mul_identity() {
        // 1 is multiplicative identity in GF(256).
        for x in 0u8..=255u8 {
            assert_eq!(gf_mul(x, 1), x);
            assert_eq!(gf_mul(1, x), x);
        }
    }

    #[test]
    fn test_gf_inv() {
        // gf_inv is involutive: gf_inv(gf_inv(x)) == x for x != 0
        // and gf_mul(x, gf_inv(x)) == 1 for x != 0.
        for x in 1u8..=255u8 {
            assert_eq!(gf_inv(gf_inv(x)), x);
            assert_eq!(gf_mul(x, gf_inv(x)), 1);
        }
    }

    #[test]
    fn test_validate_threshold_rejects_invalid() {
        assert!(SealState::validate_threshold(0, 2).is_err());
        assert!(SealState::validate_threshold(3, 1).is_err());
        assert!(SealState::validate_threshold(2, 5).is_err());
        assert!(SealState::validate_threshold(5, 3).is_ok());
    }

    #[test]
    fn test_seal_after_unseal_clears_master_key() {
        let mut state = SealState::default();
        let (_root, shares) = state.initialize(3, 2).unwrap();
        state.unseal(&shares[0]).unwrap();
        state.unseal(&shares[1]).unwrap();
        assert!(!state.is_sealed());
        state.seal();
        assert!(state.is_sealed());
        assert!(state.master_key.is_none());
    }

    #[test]
    fn test_autoseal_type_barrier_strings() {
        assert_eq!(AutoSealType::Transit.barrier_type(), "transit");
        assert_eq!(AutoSealType::AwsKms.barrier_type(), "awskms");
        assert_eq!(AutoSealType::AzureKeyVault.barrier_type(), "azurekeyvault");
        assert_eq!(AutoSealType::Shamir.barrier_type(), "shamir");
    }

    #[test]
    fn test_autoseal_shamir_does_not_store_remotely() {
        assert!(!AutoSealType::Shamir.stores_keys_remotely());
        assert!(!AutoSealType::Shamir.supports_recovery_key());
        assert!(AutoSealType::Transit.stores_keys_remotely());
        assert!(AutoSealType::AwsKms.supports_recovery_key());
    }

    #[test]
    fn test_autoseal_config_validate_shamir_rejects_recovery() {
        let bad = AutoSealConfig {
            seal_type: AutoSealType::Shamir,
            recovery_shares: 3,
            recovery_threshold: 2,
            endpoint: String::new(),
            key_id: String::new(),
        };
        assert!(bad.validate().is_err());
        let ok = AutoSealConfig {
            seal_type: AutoSealType::Shamir,
            recovery_shares: 0,
            recovery_threshold: 0,
            endpoint: String::new(),
            key_id: String::new(),
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn test_autoseal_config_validate_remote_requires_endpoint() {
        let bad = AutoSealConfig {
            seal_type: AutoSealType::AwsKms,
            recovery_shares: 5,
            recovery_threshold: 3,
            endpoint: String::new(),
            key_id: "k".into(),
        };
        assert!(bad.validate().is_err());
        let ok = AutoSealConfig {
            seal_type: AutoSealType::AwsKms,
            recovery_shares: 5,
            recovery_threshold: 3,
            endpoint: "https://kms.example.com".into(),
            key_id: "k".into(),
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn test_seal_initialize_unseal() {
        let mut state = SealState::default();
        assert!(!state.is_initialized());
        assert!(state.is_sealed());

        let (root_token, shares) = state.initialize(3, 2).unwrap();
        assert!(root_token.starts_with("hvs."));
        assert_eq!(shares.len(), 3);
        assert!(state.is_initialized());
        assert!(state.is_sealed());

        let result1 = state.unseal(&shares[0]).unwrap();
        assert!(!result1); // still sealed
        let result2 = state.unseal(&shares[1]).unwrap();
        assert!(result2); // now unsealed
        assert!(!state.is_sealed());
    }
}
