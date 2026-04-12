//! Redis Cluster support — hash slot computation, CRC16, CLUSTER commands.

use std::collections::HashMap;

// ── CRC16 ─────────────────────────────────────────────────────────────────────

const CRC16_TABLE: [u16; 256] = generate_crc16_table();

const fn generate_crc16_table() -> [u16; 256] {
    let mut table = [0u16; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u16;
        let mut j = 0;
        while j < 8 {
            if crc & 0x0001 != 0 {
                crc = (crc >> 1) ^ 0x8408;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc = (crc >> 8) ^ CRC16_TABLE[((crc ^ b as u16) & 0xFF) as usize];
    }
    crc
}

/// Compute the Redis hash slot (0–16383) for a key.
/// Respects {hashtag} extraction.
pub fn hash_slot(key: &[u8]) -> u16 {
    let effective = if let Some(start) = key.iter().position(|&b| b == b'{') {
        if let Some(end) = key[start + 1..].iter().position(|&b| b == b'}') {
            let tag = &key[start + 1..start + 1 + end];
            if !tag.is_empty() { tag } else { key }
        } else {
            key
        }
    } else {
        key
    };
    crc16(effective) % 16384
}

// ── Cluster State ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ClusterState {
    pub enabled: bool,
    pub myself_id: String,
    pub myself_addr: String,
    pub nodes: HashMap<String, ClusterNode>,
    pub epoch: u64,
    pub state: ClusterStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClusterStatus {
    Ok,
    Fail,
}

impl ClusterState {
    pub fn new() -> Self {
        ClusterState {
            enabled: false,
            myself_id: generate_node_id(),
            myself_addr: "127.0.0.1:6379".into(),
            nodes: HashMap::new(),
            epoch: 0,
            state: ClusterStatus::Ok,
        }
    }

    pub fn info(&self) -> Vec<(String, String)> {
        if !self.enabled {
            return vec![
                ("cluster_enabled".into(), "0".into()),
                ("cluster_state".into(), "ok".into()),
                ("cluster_slots_assigned".into(), "0".into()),
                ("cluster_slots_ok".into(), "0".into()),
                ("cluster_slots_pfail".into(), "0".into()),
                ("cluster_slots_fail".into(), "0".into()),
                ("cluster_known_nodes".into(), "0".into()),
                ("cluster_size".into(), "0".into()),
                ("cluster_current_epoch".into(), "0".into()),
                ("cluster_my_epoch".into(), "0".into()),
                ("cluster_stats_messages_sent".into(), "0".into()),
                ("cluster_stats_messages_received".into(), "0".into()),
                ("total_cluster_links_buffer_limit_exceeded".into(), "0".into()),
            ];
        }
        vec![
            ("cluster_enabled".into(), "1".into()),
            ("cluster_state".into(), match self.state { ClusterStatus::Ok => "ok", ClusterStatus::Fail => "fail" }.into()),
            ("cluster_slots_assigned".into(), "16384".into()),
            ("cluster_slots_ok".into(), "16384".into()),
            ("cluster_slots_pfail".into(), "0".into()),
            ("cluster_slots_fail".into(), "0".into()),
            ("cluster_known_nodes".into(), self.nodes.len().to_string()),
            ("cluster_size".into(), "1".into()),
            ("cluster_current_epoch".into(), self.epoch.to_string()),
            ("cluster_my_epoch".into(), self.epoch.to_string()),
            ("cluster_stats_messages_sent".into(), "0".into()),
            ("cluster_stats_messages_received".into(), "0".into()),
            ("total_cluster_links_buffer_limit_exceeded".into(), "0".into()),
        ]
    }

    pub fn nodes_string(&self) -> String {
        if self.nodes.is_empty() {
            // Return myself as sole node
            return format!(
                "{} {} master - 0 0 0 connected 0-16383\n",
                self.myself_id, self.myself_addr
            );
        }
        self.nodes
            .values()
            .map(|n| {
                format!(
                    "{} {} {} - 0 {} {} connected {}\n",
                    n.id,
                    n.addr,
                    if n.is_master { "master" } else { "slave" },
                    n.config_epoch,
                    n.config_epoch,
                    n.slots
                        .iter()
                        .map(|(s, e)| format!("{}-{}", s, e))
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            })
            .collect()
    }
}

impl Default for ClusterState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct ClusterNode {
    pub id: String,
    pub addr: String,
    pub is_master: bool,
    pub master_id: Option<String>,
    pub config_epoch: u64,
    pub slots: Vec<(u16, u16)>,
}

fn generate_node_id() -> String {
    use std::fmt::Write;
    let mut id = String::with_capacity(40);
    for _ in 0..20 {
        let b: u8 = rand::random();
        write!(&mut id, "{:02x}", b).unwrap();
    }
    id
}
