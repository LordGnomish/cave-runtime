---
crate: cave-secrets
upstream_repo: trufflesecurity/trufflehog
upstream_file: pkg/detectors/stripe/stripe.go
upstream_fn: FromData
status: draft
tier: 1
created_at: 2026-04-23T06:09:10.733749+00:00
---

## Upstream reference

`trufflesecurity/trufflehog` → `pkg/detectors/stripe/stripe.go` → `FromData`

## Failing test

```rust
#[tokio::test]
async fn test_fromdata_valid_stripe_key() {
    use cave_secrets::fromdata;
    use cave_commons::types::SecretType;

    // Valid Stripe secret key (starts with sk_live_)
    let data = b"sk_live_4eC39HqLyjWDarjtT1zdp7dc";
    let result = fromdata(data).await;
    
    assert!(result.is_ok());
    let secret = result.unwrap();
    assert_eq!(secret.r#type, SecretType::Stripe);
    assert_eq!(secret.display_name, "Stripe Secret Key");
    assert_eq!(secret.value, b"sk_live_4eC39HqLyjWDarjtT1zdp7dc");
    assert!(secret.is_secret);
}

## Implementation
```

## Implementation skeleton

```rust
pub async fn fromdata(data: &[u8]) -> Result<cave_commons::types::Secret, cave_commons::Error> {
    todo!("Tier 2")
}
```
