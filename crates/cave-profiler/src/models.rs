use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileSession {
    pub id: Uuid,
    pub service: String,
    pub profile_type: ProfileType,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub samples: u64,
    pub frames: Vec<StackFrame>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileType {
    Cpu,
    Memory,
    Goroutine,
    Mutex,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StackFrame {
    pub function: String,
    pub file: String,
    pub line: u32,
    pub self_samples: u64,
    pub cumulative_samples: u64,
}
