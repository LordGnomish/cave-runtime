pub fn hash_slot(key: &str) -> u16 {
    let key_bytes = extract_hash_key(key);
    crc16(key_bytes) % 16384
}

fn extract_hash_key(key: &str) -> &[u8] {
    let bytes = key.as_bytes();
    // Find first '{' and matching '}'
    if let Some(start) = bytes.iter().position(|&b| b == b'{') {
        if let Some(end) = bytes[start + 1..].iter().position(|&b| b == b'}') {
            let inner = &bytes[start + 1..start + 1 + end];
            if !inner.is_empty() {
                return inner;
            }
        }
    }
    bytes
}

/// CRC16-CCITT (polynomial 0x1021)
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[derive(Debug, Clone)]
pub struct ClusterNode {
    pub id: String,
    pub addr: String,
    pub slots: std::ops::RangeInclusive<u16>,
    pub is_master: bool,
}

#[derive(Debug, Clone)]
pub struct ClusterConfig {
    pub nodes: Vec<ClusterNode>,
    pub local_slots: std::ops::RangeInclusive<u16>,
}

impl ClusterConfig {
    pub fn new_single_node() -> Self {
        let node = ClusterNode {
            id: "local".to_string(),
            addr: "127.0.0.1:6379".to_string(),
            slots: 0..=16383,
            is_master: true,
        };
        Self {
            nodes: vec![node],
            local_slots: 0..=16383,
        }
    }

    pub fn node_for_slot(&self, slot: u16) -> Option<&ClusterNode> {
        self.nodes.iter().find(|n| n.slots.contains(&slot))
    }

    pub fn node_for_key(&self, key: &str) -> Option<&ClusterNode> {
        let slot = hash_slot(key);
        self.node_for_slot(slot)
    }

    pub fn is_local(&self, key: &str) -> bool {
        let slot = hash_slot(key);
        self.local_slots.contains(&slot)
    }
}
