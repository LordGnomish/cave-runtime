// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PiiDetector {
    pub id: Uuid,
    pub name: String,
    pub pii_type: PiiType,
    pub pattern: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PiiType {
    Email,
    PhoneNumber,
    SocialSecurityNumber,
    CreditCard,
    IpAddress,
    Name,
    Address,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PiiFinding {
    pub detector_id: Uuid,
    pub pii_type: PiiType,
    pub line_number: usize,
    pub redacted: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiiScanResult {
    pub findings: Vec<PiiFinding>,
    pub total_findings: usize,
    pub has_high_confidence_pii: bool,
}
