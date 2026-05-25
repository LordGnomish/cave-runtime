// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Runtime {
    Gvisor,
    Kata,
    Firecracker,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SandboxState {
    Created,
    Running,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sandbox {
    pub id: String,
    pub runtime: Runtime,
    pub state: SandboxState,
    pub bundle: String,
    pub annotations: std::collections::BTreeMap<String, String>,
}
