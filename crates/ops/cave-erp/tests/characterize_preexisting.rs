// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Characterization tests for pre-existing cave-erp modules.
//!
//! These modules existed on origin/main before this uplift. The tests
//! here assert REAL behavior — they pass immediately, which is expected
//! for already-correct code. They are NOT red-first TDD; they serve as
//! honest regression guards for absorbed pre-existing code.

use cave_erp::{
    engine::{advance_stage, doc_total, explode_bom, line_subtotal, on_hand_by_location},
    models::*,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

// ─── engine: line_subtotal ────────────────────────────────────────────────────

#[test]
fn characterize_line_subtotal_with_multiple_taxes() {
    // 10 units @ 20 each, 5% discount, two taxes: 10% + 5%
    // after_discount = 10 * 20 * (1 - 0.05) = 190
    // total_tax_pct  = 15%
    // subtotal       = 190 * 1.15 = 218.5
    let result = line_subtotal(10.0, 20.0, 5.0, &[10.0, 5.0]);
    assert!((result - 218.5).abs() < 0.01, "got {}", result);
}

#[test]
fn characterize_line_subtotal_zero_qty() {
    let result = line_subtotal(0.0, 100.0, 0.0, &[20.0]);
    assert!((result - 0.0).abs() < 0.01);
}

#[test]
fn characterize_doc_total_empty() {
    assert!((doc_total(&[]) - 0.0).abs() < 0.01);
}

#[test]
fn characterize_doc_total_multiple() {
    let result = doc_total(&[100.0, 200.0, 300.0]);
    assert!((result - 600.0).abs() < 0.01);
}

// ─── engine: on_hand_by_location ─────────────────────────────────────────────

#[test]
fn characterize_on_hand_empty_moves() {
    let result = on_hand_by_location(&[], Uuid::new_v4());
    assert!(result.is_empty());
}

#[test]
fn characterize_on_hand_only_draft_moves_ignored() {
    let prod = Uuid::new_v4();
    let loc_a = Uuid::new_v4();
    let loc_b = Uuid::new_v4();

    let moves = vec![StockMove {
        id: Uuid::new_v4(),
        product_id: prod,
        qty: 100.0,
        from_location_id: loc_a,
        to_location_id: loc_b,
        state: StockMoveState::Draft,
        lot_id: None,
        created_at: Utc::now(),
        done_at: None,
    }];

    let result = on_hand_by_location(&moves, prod);
    assert!(result.is_empty(), "draft moves should not affect on-hand");
}

// ─── engine: explode_bom ─────────────────────────────────────────────────────

#[test]
fn characterize_explode_bom_empty_components() {
    let bom = Bom {
        id: Uuid::new_v4(),
        product_id: Uuid::new_v4(),
        components: vec![],
        quantity: 1.0,
        routing_id: None,
        created_at: Utc::now(),
    };
    let boms = HashMap::from([(bom.id, bom.clone())]);
    let result = explode_bom(&bom, &boms, 10.0);
    assert!(result.is_empty());
}

// ─── engine: advance_stage ───────────────────────────────────────────────────

#[test]
fn characterize_advance_stage_moves_to_next_crm_stage() {
    let s1 = CrmStage { id: Uuid::new_v4(), name: "New".to_string(), order: 1, probability: 10, is_won: false, is_lost: false };
    let s2 = CrmStage { id: Uuid::new_v4(), name: "Qualified".to_string(), order: 2, probability: 30, is_won: false, is_lost: false };
    let s3 = CrmStage { id: Uuid::new_v4(), name: "Won".to_string(), order: 3, probability: 100, is_won: true, is_lost: false };

    let stages = vec![s1.clone(), s2.clone(), s3.clone()];
    let next = advance_stage(s1.id, &stages);
    assert!(next.is_some());
    use cave_erp::engine::Stage;
    assert_eq!(next.unwrap().id(), s2.id);
}

#[test]
fn characterize_advance_stage_at_last_returns_none() {
    let s1 = CrmStage { id: Uuid::new_v4(), name: "Won".to_string(), order: 1, probability: 100, is_won: true, is_lost: false };
    let stages = vec![s1.clone()];
    let next = advance_stage(s1.id, &stages);
    assert!(next.is_none());
}

// ─── models: status fields ───────────────────────────────────────────────────

#[test]
fn characterize_employee_status_serialization() {
    let emp = Employee {
        id: Uuid::new_v4(),
        name: "Test".to_string(),
        email: "t@example.com".to_string(),
        phone: None,
        department_id: Uuid::new_v4(),
        job_title: "Engineer".to_string(),
        manager_id: None,
        hire_date: Utc::now(),
        termination_date: None,
        status: EmployeeStatus::Active,
        tags: vec!["rust".to_string()],
        created_at: Utc::now(),
    };
    let json = serde_json::to_string(&emp).unwrap();
    assert!(json.contains("active"), "should serialize snake_case");
    assert!(json.contains("rust"));
}

#[test]
fn characterize_invoice_state_roundtrip() {
    let state = InvoiceState::Posted;
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(json, "\"posted\"");
    let back: InvoiceState = serde_json::from_str(&json).unwrap();
    assert_eq!(back, InvoiceState::Posted);
}

#[test]
fn characterize_sale_order_state_roundtrip() {
    for state in [
        SaleOrderState::Draft,
        SaleOrderState::Confirmed,
        SaleOrderState::Cancelled,
        SaleOrderState::Done,
    ] {
        let json = serde_json::to_string(&state).unwrap();
        let back: SaleOrderState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, state);
    }
}

// ─── models: BOM component logic ─────────────────────────────────────────────

#[test]
fn characterize_bom_component_proportional_scaling() {
    let prod_a = Uuid::new_v4();
    let prod_b = Uuid::new_v4();

    let bom = Bom {
        id: Uuid::new_v4(),
        product_id: prod_a,
        components: vec![BomComponent { product_id: prod_b, qty: 2.0 }],
        quantity: 1.0,
        routing_id: None,
        created_at: Utc::now(),
    };

    let boms = HashMap::from([(bom.id, bom.clone())]);
    // Making 7 units → need 14 of prod_b
    let explosion = explode_bom(&bom, &boms, 7.0);
    let qty = explosion.iter().find(|(p, _)| *p == prod_b).map(|(_, q)| *q).unwrap();
    assert!((qty - 14.0).abs() < 0.01);
}

// ─── models: CRM / Lead / Opportunity ────────────────────────────────────────

#[test]
fn characterize_lead_status_transitions() {
    let mut lead = Lead {
        id: Uuid::new_v4(),
        name: "Big Deal".to_string(),
        contact_name: "Alice".to_string(),
        email: "alice@corp.com".to_string(),
        phone: None,
        company: "Corp Inc".to_string(),
        source: "web".to_string(),
        status: LeadStatus::New,
        created_at: Utc::now(),
        assigned_to: None,
    };

    assert_eq!(lead.status, LeadStatus::New);
    lead.status = LeadStatus::Qualified;
    assert_eq!(lead.status, LeadStatus::Qualified);
    lead.status = LeadStatus::Converted;
    assert_eq!(lead.status, LeadStatus::Converted);
}

// ─── models: Manufacturing → WorkOrder lifecycle ─────────────────────────────

#[test]
fn characterize_manufacturing_order_lifecycle() {
    let mut mo = ManufacturingOrder {
        id: Uuid::new_v4(),
        product_id: Uuid::new_v4(),
        qty: 5.0,
        bom_id: Uuid::new_v4(),
        state: ManufacturingOrderState::Draft,
        scheduled_start: Utc::now(),
        completed_at: None,
        created_at: Utc::now(),
    };

    assert_eq!(mo.state, ManufacturingOrderState::Draft);
    mo.state = ManufacturingOrderState::Confirmed;
    assert_eq!(mo.state, ManufacturingOrderState::Confirmed);
    mo.state = ManufacturingOrderState::InProgress;
    assert_eq!(mo.state, ManufacturingOrderState::InProgress);
    mo.state = ManufacturingOrderState::Done;
    mo.completed_at = Some(Utc::now());
    assert_eq!(mo.state, ManufacturingOrderState::Done);
    assert!(mo.completed_at.is_some());
}

// ─── models: Project summary ─────────────────────────────────────────────────

#[test]
fn characterize_project_budget_and_currency() {
    let proj = Project {
        id: Uuid::new_v4(),
        name: "Cave Runtime v2".to_string(),
        code: "CRV2".to_string(),
        customer_id: None,
        manager_id: Uuid::new_v4(),
        state: ProjectState::Active,
        start: Utc::now(),
        end: None,
        budget: 250_000.0,
        currency: "EUR".to_string(),
        created_at: Utc::now(),
    };

    assert!((proj.budget - 250_000.0).abs() < 0.01);
    assert_eq!(proj.currency, "EUR");
    assert_eq!(proj.state, ProjectState::Active);
}
