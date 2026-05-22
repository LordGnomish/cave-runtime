---
crate: cave-secrets
upstream_repo: trufflesecurity/trufflehog
upstream_file: pkg/common/entropy.go
upstream_fn: hasHighEntropy
status: draft
tier: 1
created_at: 2026-04-23T05:45:59.915121+00:00
---

## Upstream reference

`trufflesecurity/trufflehog` → `pkg/common/entropy.go` → `hasHighEntropy`

## Failing test

```rust
#[tokio::test]
async fn test_hashighentropy() {
    use cave_secrets::hashighentropy;

    // High entropy strings (should return true)
    let high_entropy_strings = vec![
        "aB3$kL9@mN2#pQ7&rS5*tU1?vW4+xY6/zC8!dE0",
        "xK9$mL2@nQ7#pR4&sT6*uV1?wX3+yZ5/cB8!dA0",
        "fGhIjKlMnOpQrStUvWxYz1234567890!@#$%^&*()",
    ];

    for s in high_entropy_strings {
        assert!(hashighentropy(s), "Expected high entropy for: {}", s);
    }

    // Low entropy strings (should return false)
    let low_entropy_strings = vec![
        "aaaaaaaaaa",
        "1234567890",
        "hello world",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ];

    for s in low_entropy_strings {
        assert!(!hashighentropy(s), "Expected low entropy for: {}", s);
    }

    // Edge cases
    assert!(!hashighentropy(""), "Empty string should be low entropy");
    assert!(!hashighentropy("a"), "Single char should be low entropy");
    assert!(hashighentropy("aB"), "Two different chars should be high entropy");
}
```

## Implementation skeleton

```rust
pub fn hashighentropy(s: &str) -> bool {
    todo!("Tier 2")
}
```
