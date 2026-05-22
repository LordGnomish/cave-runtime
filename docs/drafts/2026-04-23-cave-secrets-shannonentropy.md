---
crate: cave-secrets
upstream_repo: trufflesecurity/trufflehog
upstream_file: pkg/common/entropy.go
upstream_fn: shannonEntropy
status: draft
tier: 1
created_at: 2026-04-23T05:44:42.653479+00:00
---

## Upstream reference

`trufflesecurity/trufflehog` → `pkg/common/entropy.go` → `shannonEntropy`

## Failing test

```rust
#[tokio::test]
async fn test_shannon_entropy() {
    use cave_secrets::shannonentropy;

    // Test with high-entropy string (random-looking hex)
    let high_entropy = "a1b2c3d4e5f6789012345678901234567890abcd";
    let entropy = shannonentropy(high_entropy).await;
    assert!(entropy > 3.5, "High-entropy string should have entropy > 3.5, got {}", entropy);

    // Test with low-entropy string (repetitive)
    let low_entropy = "aaaaaa";
    let entropy = shannonentropy(low_entropy).await;
    assert!(entropy < 1.0, "Low-entropy string should have entropy < 1.0, got {}", entropy);

    // Test with empty string (should return 0.0)
    let empty = "";
    let entropy = shannonentropy(empty).await;
    assert_eq!(entropy, 0.0, "Empty string should have entropy 0.0");

    // Test with mixed-case ASCII (realistic secret-like input)
    let mixed = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
    let entropy = shannonentropy(mixed).await;
    assert!(entropy > 4.0, "Mixed-case alphanumeric string should have entropy > 4.0, got {}", entropy);
}
```

## Implementation skeleton

```rust
pub async fn shannonentropy(input: &str) -> f64 {
    todo!("Tier 2")
}
```
