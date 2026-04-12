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
    if k < 2 || k > n || n > 255 {
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
