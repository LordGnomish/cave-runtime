---
crate: cave-etcd
upstream_repo: etcd-io/etcd
upstream_file: server/storage/wal/record.go
upstream_fn: Decode
status: draft
tier: 1
created_at: 2026-04-24T16:37:46.088435+00:00
---

## Upstream reference

`etcd-io/etcd` → `server/storage/wal/record.go` → `Decode`

## Failing test

```rust
#[tokio::test]
async fn test_decode_valid_record() {
    use cave_etcd::decode;
    use std::io::Cursor;

    // Create a minimal valid record: crc (4 bytes) + type (1 byte) + length (4 bytes) + data + crc (4 bytes)
    let data = b"hello world";
    let record_type = 1u8; // kSnapshotRecord in etcd
    let length = data.len() as u32;
    
    // Calculate CRCs (simplified for test; real implementation uses crc32)
    let data_crc = 0x12345678u32; // placeholder
    let trailer_crc = 0x87654321u32; // placeholder

    let mut buf = Vec::new();
    buf.extend_from_slice(&data_crc.to_le_bytes());
    buf.push(record_type);
    buf.extend_from_slice(&length.to_le_bytes());
    buf.extend_from_slice(data);
    buf.extend_from_slice(&trailer_crc.to_le_bytes());

    let mut cursor = Cursor::new(buf);
    let result = decode(&mut cursor).await.unwrap();

    assert_eq!(result.record_type, record_type);
    assert_eq!(result.data, data);
}
```

## Implementation skeleton

```rust
use std::io::{self, Read};
use tokio::io::AsyncRead;

pub struct Record {
    pub record_type: u8,
    pub data: Vec<u8>,
}

pub async fn decode<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Record> {
    todo!("Tier 2")
}
```
