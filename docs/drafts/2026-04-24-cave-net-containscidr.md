---
crate: cave-net
upstream_repo: cilium/cilium
upstream_file: pkg/cidr/cidr.go
upstream_fn: ContainsCIDR
status: draft
tier: 1
created_at: 2026-04-24T16:42:20.859407+00:00
---

## Upstream reference

`cilium/cilium` → `pkg/cidr/cidr.go` → `ContainsCIDR`

## Failing test

```rust
#[tokio::test]
async fn test_containscidr() {
    use cave_net::containscidr;
    
    // Test case 1: exact match
    assert!(containscidr("192.168.1.0/24", "192.168.1.0/24").unwrap());
    
    // Test case 2: superset contains subset
    assert!(containscidr("192.168.0.0/16", "192.168.1.0/24").unwrap());
    
    // Test case 3: subset does not contain superset
    assert!(!containscidr("192.168.1.0/24", "192.168.0.0/16").unwrap());
    
    // Test case 4: overlapping but not contained
    assert!(!containscidr("192.168.1.0/24", "192.168.2.0/24").unwrap());
    
    // Test case 5: IPv6 superset contains subset
    assert!(containscidr("2001:db8::/32", "2001:db8:1::/48").unwrap());
    
    // Test case 6: IPv6 exact match
    assert!(containscidr("2001:db8::/32", "2001:db8::/32").unwrap());
    
    // Test case 7: different families
    assert!(!containscidr("192.168.0.0/16", "2001:db8::/32").unwrap());
    
    // Test case 8: invalid CIDR returns false
    assert!(!containscidr("invalid", "192.168.1.0/24").unwrap());
    assert!(!containscidr("192.168.1.0/24", "invalid").unwrap());
    
    // Test case 9: /0 superset contains everything
    assert!(containscidr("0.0.0.0/0", "10.0.0.0/8").unwrap());
    assert!(containscidr("::/0", "2001:db8::/32").unwrap());
}
```

## Implementation skeleton

```rust
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

pub fn containscidr(cidr1: &str, cidr2: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let parse_cidr = |s: &str| -> Result<Option<(IpAddr, u8)>, Box<dyn std::error::Error>> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 2 {
            return Ok(None);
        }
        let ip = IpAddr::from_str(parts[0])?;
        let prefix_len: u8 = parts[1].parse()?;
        Ok(Some((ip, prefix_len)))
    };

    let (ip1, prefix1) = match parse_cidr(cidr1)? {
        Some(v) => v,
        None => return Ok(false),
    };
    let (ip2, prefix2) = match parse_cidr(cidr2)? {
        Some(v) => v,
        None => return Ok(false),
    };

    // Different IP families cannot contain each other
    if ip1.version() != ip2.version() {
        return Ok(false);
    }

    // If cidr1 has a smaller prefix (larger network), it cannot contain cidr2
    if prefix1 > prefix2 {
        return Ok(false);
    }

    // Check if ip2 is within the network of ip1
    match (ip1, ip2) {
        (IpAddr::V4(v1), IpAddr::V4(v2)) => {
            let mask = if prefix1 == 0 {
                Ipv4Addr::new(0, 0, 0, 0)
            } else {
                let mask_bits = u32::MAX << (32 - prefix1);
                Ipv4Addr::from(mask_bits)
            };
            Ok(v2.octets()[0] & mask.octets()[0] == v1.octets()[0] & mask.octets()[0] &&
               v2.octets()[1] & mask.octets()[1] == v1.octets()[1] & mask.octets()[1] &&
               v2.octets()[2] & mask.octets()[2] == v1.octets()[2] & mask.octets()[2] &&
               v2.octets()[3] & mask.octets()[3] == v1.octets()[3] & mask.octets()[3])
        }
        (IpAddr::V6(v1), IpAddr::V6(v2)) => {
            let mask = if prefix1 == 0 {
                Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)
            } else {
                let mut mask = [0u16; 8];
                let full_words = (prefix1 as usize) / 16;
                let remaining_bits = (prefix1 as usize) % 16;
                for i in 0..full_words {
                    mask[i] = 0xFFFF;
                }
                if remaining_bits > 0 {
                    mask[full_words] = (0xFFFFu16 << (16 - remaining_bits)) as u16;
                }
                Ipv6Addr::new(mask[0], mask[1], mask[2], mask[3], mask[4], mask[5], mask[6], mask[7])
            };
            let v1_bytes = v1.octets();
            let v2_bytes = v2.octets();
            let mask_bytes = mask.octets();
            Ok((0..16).all(|i| (v2_bytes[i] & mask_bytes[i]) == (v1_bytes[i] & mask_bytes[i])))
        }
        _ => Ok(false),
    }
}
```
