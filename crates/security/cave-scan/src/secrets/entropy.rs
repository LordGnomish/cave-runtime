// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: gitleaks/gitleaks@9febafb detect/detect.go (shannonEntropy)
//! Shannon entropy + base64/hex detection.

/// Shannon entropy in bits-per-character.
///
/// Frequency-weighted: H = -Σ p_i · log2(p_i). Returns 0.0 for empty input.
pub fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    let mut n = 0u32;
    for b in s.bytes() {
        counts[b as usize] += 1;
        n += 1;
    }
    let nf = n as f64;
    let mut h = 0.0;
    for &c in counts.iter() {
        if c == 0 {
            continue;
        }
        let p = c as f64 / nf;
        h -= p * p.log2();
    }
    h
}

/// Heuristic: is the string a base64 token (≥20 chars, alphabet match)?
pub fn looks_like_base64(s: &str) -> bool {
    if s.len() < 20 {
        return false;
    }
    s.bytes().all(|b| {
        b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=' || b == b'_' || b == b'-'
    })
}

/// Heuristic: is the string a hex token (≥32 chars, hex alphabet)?
pub fn looks_like_hex(s: &str) -> bool {
    s.len() >= 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Score a candidate: True if entropy crosses gitleaks's 4.5 threshold for
/// base64 / 3.0 for hex.
pub fn high_entropy_secret(s: &str) -> bool {
    if looks_like_base64(s) && shannon_entropy(s) > 4.5 {
        return true;
    }
    if looks_like_hex(s) && shannon_entropy(s) > 3.0 {
        return true;
    }
    false
}
