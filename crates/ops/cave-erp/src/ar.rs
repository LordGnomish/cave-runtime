// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Accounts-receivable balance engine.
//!
//! Computes the outstanding balance for a partner by aggregating their
//! posted/paid invoices against done payments.  Mirrors the
//! ERPNext / Odoo Accounts-Receivable report surface.

use crate::models::{Invoice, InvoiceKind, InvoiceState, Payment, PaymentState};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Which direction a partner balance runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BalanceDirection {
    /// Customer owes us money (positive outstanding).
    Receivable,
    /// We owe the partner money (credit balance, e.g. over-payment).
    Payable,
    /// Fully settled or no invoices.
    None,
}

/// Aggregated AR (or AP) position for a partner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartnerBalance {
    pub partner_id: Uuid,
    /// Total amount on all open (posted/paid) invoices.
    pub invoiced: f64,
    /// Total confirmed payments applied.
    pub paid: f64,
    /// invoiced − paid  (clamped — may not go below 0 for Receivable).
    pub outstanding: f64,
    pub direction: BalanceDirection,
    /// Count of invoices included (draft invoices excluded).
    pub invoice_count: usize,
}

/// Compute the accounts-receivable balance for `partner_id`.
///
/// Only **Posted** and **Paid** customer invoices contribute to the balance.
/// Draft and Cancelled invoices are excluded.
/// Only `PaymentState::Done` payments are counted.
pub fn compute_partner_balance(
    partner_id: Uuid,
    invoices: &[Invoice],
    payments: &[Payment],
) -> PartnerBalance {
    // Filter invoices: must belong to partner, be customer invoices, and not Draft/Cancelled
    let relevant_invoices: Vec<&Invoice> = invoices
        .iter()
        .filter(|inv| {
            inv.partner_id == partner_id
                && inv.kind == InvoiceKind::Customer
                && (inv.state == InvoiceState::Posted || inv.state == InvoiceState::Paid)
        })
        .collect();

    let invoice_count = relevant_invoices.len();
    let invoiced: f64 = relevant_invoices.iter().map(|inv| inv.amount_total).sum();

    // Build set of relevant invoice IDs
    let relevant_ids: std::collections::HashSet<Uuid> =
        relevant_invoices.iter().map(|inv| inv.id).collect();

    // Payments linked to those invoices
    let paid: f64 = payments
        .iter()
        .filter(|p| p.state == PaymentState::Done && relevant_ids.contains(&p.invoice_id))
        .map(|p| p.amount)
        .sum();

    let outstanding = (invoiced - paid).max(0.0);

    let direction = if invoice_count == 0 || outstanding < 0.001 {
        BalanceDirection::None
    } else {
        BalanceDirection::Receivable
    };

    PartnerBalance {
        partner_id,
        invoiced,
        paid,
        outstanding,
        direction,
        invoice_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{InvoiceLine, PaymentMethod};
    use chrono::Utc;

    fn inv(partner_id: Uuid, amount: f64, state: InvoiceState) -> Invoice {
        Invoice {
            id: Uuid::new_v4(),
            number: "INV-TEST".to_string(),
            partner_id,
            kind: InvoiceKind::Customer,
            journal_id: Uuid::new_v4(),
            lines: vec![InvoiceLine {
                id: Uuid::new_v4(),
                product_id: None,
                description: "Test".to_string(),
                quantity: 1.0,
                unit_price: amount,
                tax_ids: vec![],
                subtotal: amount,
            }],
            amount_total: amount,
            state,
            due_date: Utc::now(),
            created_at: Utc::now(),
        }
    }

    fn pay(invoice_id: Uuid, amount: f64) -> Payment {
        Payment {
            id: Uuid::new_v4(),
            invoice_id,
            amount,
            date: Utc::now(),
            method: PaymentMethod::Bank,
            state: PaymentState::Done,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_cancelled_invoice_excluded() {
        let pid = Uuid::new_v4();
        let inv_cancelled = inv(pid, 500.0, InvoiceState::Cancelled);
        let balance = compute_partner_balance(pid, &[inv_cancelled], &[]);
        assert!((balance.outstanding - 0.0).abs() < 0.01);
        assert_eq!(balance.invoice_count, 0);
    }

    #[test]
    fn test_draft_payment_not_counted() {
        let pid = Uuid::new_v4();
        let i = inv(pid, 300.0, InvoiceState::Posted);
        let mut p = pay(i.id, 300.0);
        p.state = PaymentState::Draft; // not done yet

        let balance = compute_partner_balance(pid, &[i], &[p]);
        // Payment not counted → full 300 still outstanding
        assert!((balance.outstanding - 300.0).abs() < 0.01);
    }
}
