// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Predefined control → CAVE module mappings and control seeding.

use crate::models::{Control, Framework};
use chrono::Utc;
use uuid::Uuid;

// ────────────────────────────────────────────────────────────────────────────
// Lightweight mapping type (no DB, used by engine at assessment time)
// ────────────────────────────────────────────────────────────────────────────

/// A CAVE module that satisfies part of a control requirement.
pub struct ModuleMapping {
    /// CAVE module name, e.g. "cave-auth".
    pub cave_module: String,
    /// Human description of why this module satisfies the control.
    pub description: String,
}

/// Return the expected CAVE modules for a given control.
///
/// Returns an empty vec for controls without predefined mappings; the engine
/// treats those as requiring manual evidence collection.
pub fn get_mappings_for_control(control: &Control) -> Vec<ModuleMapping> {
    match (&control.framework, control.identifier.as_str()) {
        // SOC2 CC6.1 — Logical Access Controls
        (Framework::Soc2, "CC6.1") => vec![
            ModuleMapping {
                cave_module: "cave-auth".into(),
                description: "RBAC configured and enforced".into(),
            },
            ModuleMapping {
                cave_module: "cave-pam".into(),
                description: "Privileged access managed".into(),
            },
        ],

        // SOC2 CC6.8 — Vulnerability Management
        (Framework::Soc2, "CC6.8") => vec![
            ModuleMapping {
                cave_module: "cave-vulns".into(),
                description: "Vulnerability scanning active".into(),
            },
            ModuleMapping {
                cave_module: "cave-scan".into(),
                description: "Code scanning active".into(),
            },
        ],

        // SOC2 CC7.2 — Incident Detection and Response
        (Framework::Soc2, "CC7.2") => vec![
            ModuleMapping {
                cave_module: "cave-incidents".into(),
                description: "Incident response process active".into(),
            },
            ModuleMapping {
                cave_module: "cave-alerts".into(),
                description: "Monitoring and alerting active".into(),
            },
        ],

        // ISO27001 A.12.4 — Logging and Monitoring
        (Framework::Iso27001, "A.12.4") => vec![
            ModuleMapping {
                cave_module: "cave-logs".into(),
                description: "Centralised logging active".into(),
            },
            ModuleMapping {
                cave_module: "cave-metrics".into(),
                description: "Metrics monitoring active".into(),
            },
        ],

        // GDPR Art.32 — Security of Processing
        (Framework::Gdpr, "Art.32") => vec![
            ModuleMapping {
                cave_module: "cave-vault".into(),
                description: "Encryption at rest via secrets vault".into(),
            },
            ModuleMapping {
                cave_module: "cave-pii".into(),
                description: "PII detection and masking active".into(),
            },
        ],

        _ => vec![],
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Seed data
// ────────────────────────────────────────────────────────────────────────────

/// Return the built-in control definitions for all supported frameworks.
///
/// Called once at startup to populate the in-memory store.
pub fn seed_controls() -> Vec<Control> {
    let now = Utc::now();

    vec![
        // ── SOC2 ─────────────────────────────────────────────────────────
        Control {
            id: Uuid::new_v4(),
            framework: Framework::Soc2,
            identifier: "CC6.1".into(),
            name: "Logical Access Controls".into(),
            description: "Logical access security measures restrict access to information assets \
                          to authorized individuals."
                .into(),
            category: "Logical and Physical Access Controls".into(),
            required: true,
            created_at: now,
        },
        Control {
            id: Uuid::new_v4(),
            framework: Framework::Soc2,
            identifier: "CC6.8".into(),
            name: "Vulnerability Management".into(),
            description: "The entity implements controls to prevent or detect and act upon the \
                          introduction of unauthorized or malicious software."
                .into(),
            category: "Logical and Physical Access Controls".into(),
            required: true,
            created_at: now,
        },
        Control {
            id: Uuid::new_v4(),
            framework: Framework::Soc2,
            identifier: "CC7.2".into(),
            name: "Incident Detection and Response".into(),
            description: "The entity monitors system components and the operation of those \
                          components for anomalies indicative of malicious acts."
                .into(),
            category: "System Operations".into(),
            required: true,
            created_at: now,
        },
        // ── ISO 27001 ─────────────────────────────────────────────────────
        Control {
            id: Uuid::new_v4(),
            framework: Framework::Iso27001,
            identifier: "A.12.4".into(),
            name: "Logging and Monitoring".into(),
            description: "Event logs recording user activities, exceptions, faults and \
                          information security events shall be produced, kept and regularly \
                          reviewed."
                .into(),
            category: "Operations Security".into(),
            required: true,
            created_at: now,
        },
        // ── GDPR ──────────────────────────────────────────────────────────
        Control {
            id: Uuid::new_v4(),
            framework: Framework::Gdpr,
            identifier: "Art.32".into(),
            name: "Security of Processing".into(),
            description: "Implementation of appropriate technical and organisational measures to \
                          ensure a level of security appropriate to the risk, including \
                          encryption of personal data."
                .into(),
            category: "Security".into(),
            required: true,
            created_at: now,
        },
    ]
}
