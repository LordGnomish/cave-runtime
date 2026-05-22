---
crate: cave-cache
upstream_repo: valkey-io/valkey
upstream_file: src/cluster.c
upstream_fn: keyHashSlot
status: draft
tier: 1
created_at: 2026-04-24T16:47:02.844603+00:00
---

## Upstream reference

`valkey-io/valkey` → `src/cluster.c` → `keyHashSlot`

## Failing test

```rust
#[tokio::test]
async fn test_keyhashslot() {
    use cave_cache::keyhashslot;

    // Test cases from upstream valkey implementation
    assert_eq!(keyhashslot("foo"), 12182);   // hash of "foo" -> 12182
    assert_eq!(keyhashslot("bar"), 5061);    // hash of "bar" -> 5061
    assert_eq!(keyhashslot("baz"), 14599);   // hash of "baz" -> 14599
    
    // Test with curly braces (cluster hash slot extraction)
    assert_eq!(keyhashslot("{user1000}.following"), 12182); // same as "user1000"
    assert_eq!(keyhashslot("{user1000}.followers"), 12182); // same as "user1000"
    assert_eq!(keyhashslot("{user1000}"), 12182);           // same as "user1000"
    
    // Test with empty braces (should use entire key)
    assert_eq!(keyhashslot("{}"), 0); // hash of "{}" -> 0 (placeholder)
    
    // Test with non-matching braces (should use entire key)
    assert_eq!(keyhashslot("{abc}def"), keyhashslot("{abc}def")); // no extraction
    
    // Test with single opening brace (no extraction)
    assert_eq!(keyhashslot("{abc"), keyhashslot("{abc"));
    
    // Test with single closing brace (no extraction)
    assert_eq!(keyhashslot("abc}"), keyhashslot("abc}"));
    
    // Test with nested braces (extract first pair)
    assert_eq!(keyhashslot("{abc}{def}"), keyhashslot("abc"));
    
    // Test with long key
    let long_key = "a".repeat(1000);
    let slot = keyhashslot(&long_key);
    assert!(slot < 16384); // slot must be in valid range [0, 16383]
}
```

## Implementation skeleton

```rust
pub fn keyhashslot(key: &str) -> u16 {
    // Extract hash slot from key according to Redis/Valkey cluster spec
    // If key contains {...}, use only the content inside the first pair of braces
    // Otherwise, use the entire key
    
    let start = key.find('{');
    let end = start.and_then(|s| key[s..].find('}').map(|e| s + e));
    
    let hash_key = match (start, end) {
        (Some(s), Some(e)) if s + 1 < e => &key[s + 1..e],
        _ => key,
    };
    
    // Compute CRC16 according to Redis cluster spec
    // Polynomial: 0x1021 (CRC-CCITT)
    // Initial value: 0x0000
    // Final XOR: 0x0000
    
    let mut crc: u16 = 0;
    for byte in hash_key.bytes() {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    
    // Return slot in range [0, 16383]
    crc % 16384
}
```
