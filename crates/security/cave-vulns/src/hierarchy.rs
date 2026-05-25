// SPDX-License-Identifier: AGPL-3.0-or-later
//! ProductType → Product → Engagement → Test hierarchy.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/models.py
//!         (`Product_Type`:839, `Product`:1128, `Engagement`:1535,
//!          `Test`:2163). Ported the security-essentials of each;
//!          ACLs/notification configs/upload uploads/PDF reports
//!          stay in the per-feature modules (`risk_accept`, `reports`, …).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Top-level grouping ("Web Apps", "Internal Services", …).
/// Source: dojo/models.py:839 (`class Product_Type`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductType {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub critical_product: bool,
    pub key_product: bool,
    pub created: DateTime<Utc>,
}

impl ProductType {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: None,
            critical_product: false,
            key_product: false,
            created: Utc::now(),
        }
    }
}

/// A scanned application/service.
/// Source: dojo/models.py:1128 (`class Product`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Product {
    pub id: Uuid,
    pub product_type_id: Uuid,
    pub name: String,
    pub description: String,
    pub business_criticality: BusinessCriticality,
    pub platform: Option<String>,
    pub lifecycle: Lifecycle,
    pub origin: Option<String>,
    pub user_records: Option<u64>,
    pub revenue: Option<f64>,
    pub external_audience: bool,
    pub internet_accessible: bool,
    pub tags: Vec<String>,
    pub created: DateTime<Utc>,
}

impl Product {
    pub fn new(product_type_id: Uuid, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            product_type_id,
            name: name.into(),
            description: String::new(),
            business_criticality: BusinessCriticality::None,
            platform: None,
            lifecycle: Lifecycle::Production,
            origin: None,
            user_records: None,
            revenue: None,
            external_audience: false,
            internet_accessible: false,
            tags: Vec::new(),
            created: Utc::now(),
        }
    }
}

/// Source: dojo/models.py:1128 — `business_criticality` choices.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BusinessCriticality {
    VeryHigh,
    High,
    Medium,
    Low,
    VeryLow,
    None,
}

/// Source: dojo/models.py:1128 — `lifecycle` choices.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Lifecycle {
    Construction,
    Production,
    Retirement,
}

/// A bounded testing window against a Product.
/// Source: dojo/models.py:1535 (`class Engagement`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Engagement {
    pub id: Uuid,
    pub product_id: Uuid,
    pub name: String,
    pub engagement_type: EngagementType,
    pub target_start: DateTime<Utc>,
    pub target_end: DateTime<Utc>,
    pub status: EngagementStatus,
    pub lead_email: Option<String>,
    pub description: Option<String>,
    pub deduplication_on_engagement: bool,
    pub created: DateTime<Utc>,
}

impl Engagement {
    pub fn new(product_id: Uuid, name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            product_id,
            name: name.into(),
            engagement_type: EngagementType::CICD,
            target_start: now,
            target_end: now + chrono::Duration::days(30),
            status: EngagementStatus::InProgress,
            lead_email: None,
            description: None,
            deduplication_on_engagement: false,
            created: now,
        }
    }
}

/// Source: dojo/models.py:1535 — `engagement_type` choices.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EngagementType {
    Interactive,
    CICD,
}

/// Source: dojo/models.py:1535 — `status` choices.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EngagementStatus {
    NotStarted,
    Blocked,
    Cancelled,
    Completed,
    InProgress,
    OnHold,
    WaitingForResource,
}

/// A single scan run within an Engagement.
/// Source: dojo/models.py:2163 (`class Test`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Test {
    pub id: Uuid,
    pub engagement_id: Uuid,
    pub test_type: String,
    pub scan_type: Option<String>,
    pub target_start: DateTime<Utc>,
    pub target_end: DateTime<Utc>,
    pub percent_complete: u8,
    pub version: Option<String>,
    pub created: DateTime<Utc>,
}

impl Test {
    pub fn new(engagement_id: Uuid, test_type: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            engagement_id,
            test_type: test_type.into(),
            scan_type: None,
            target_start: now,
            target_end: now,
            percent_complete: 100,
            version: None,
            created: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_type_constructor_sets_name() {
        let pt = ProductType::new("Web Apps");
        assert_eq!(pt.name, "Web Apps");
        assert!(!pt.critical_product);
    }

    #[test]
    fn product_links_to_product_type() {
        let pt = ProductType::new("Internal");
        let p = Product::new(pt.id, "Acme Portal");
        assert_eq!(p.product_type_id, pt.id);
        assert_eq!(p.lifecycle, Lifecycle::Production);
    }

    #[test]
    fn engagement_default_duration_30_days() {
        let p = Product::new(Uuid::new_v4(), "x");
        let e = Engagement::new(p.id, "Q3 2026");
        let dur = e.target_end.signed_duration_since(e.target_start);
        assert_eq!(dur.num_days(), 30);
        assert_eq!(e.status, EngagementStatus::InProgress);
    }

    #[test]
    fn test_links_to_engagement() {
        let p = Product::new(Uuid::new_v4(), "x");
        let e = Engagement::new(p.id, "x");
        let t = Test::new(e.id, "Trivy Scan");
        assert_eq!(t.engagement_id, e.id);
        assert_eq!(t.test_type, "Trivy Scan");
        assert_eq!(t.percent_complete, 100);
    }

    #[test]
    fn all_types_serde_roundtrip() {
        let pt = ProductType::new("x");
        let p = Product::new(pt.id, "y");
        let e = Engagement::new(p.id, "z");
        let t = Test::new(e.id, "w");
        for j in [
            serde_json::to_string(&pt).unwrap(),
            serde_json::to_string(&p).unwrap(),
            serde_json::to_string(&e).unwrap(),
            serde_json::to_string(&t).unwrap(),
        ] {
            assert!(!j.is_empty());
        }
    }
}
