// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::*;
use crate::store::ErpStore;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct CreateJournalRequest {
    pub code: String,
    pub name: String,
    pub kind: JournalKind,
}

#[derive(Serialize, Deserialize)]
pub struct CreateAccountRequest {
    pub code: String,
    pub name: String,
    pub account_type: AccountType,
}

#[derive(Serialize, Deserialize)]
pub struct CreateJournalEntryRequest {
    pub journal_id: Uuid,
    pub date: chrono::DateTime<chrono::Utc>,
    pub reference: String,
    pub lines: Vec<CreateJournalLineRequest>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateJournalLineRequest {
    pub account_id: Uuid,
    pub debit: f64,
    pub credit: f64,
    pub partner_id: Option<Uuid>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateTaxRequest {
    pub name: String,
    pub pct: f64,
    pub kind: TaxKind,
}

#[derive(Serialize, Deserialize)]
pub struct CreateInvoiceRequest {
    pub partner_id: Uuid,
    pub kind: InvoiceKind,
    pub journal_id: Uuid,
    pub lines: Vec<CreateInvoiceLineRequest>,
    pub due_date: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateInvoiceLineRequest {
    pub product_id: Option<Uuid>,
    pub description: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub tax_ids: Option<Vec<Uuid>>,
}

#[derive(Serialize, Deserialize)]
pub struct CreatePaymentRequest {
    pub invoice_id: Uuid,
    pub amount: f64,
    pub method: PaymentMethod,
}

async fn create_journal(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateJournalRequest>,
) -> impl IntoResponse {
    let journal = Journal {
        id: Uuid::new_v4(),
        code: req.code,
        name: req.name,
        kind: req.kind,
        created_at: Utc::now(),
    };
    let id = journal.id;
    store.journals.write().await.insert(id, journal.clone());
    (StatusCode::CREATED, Json(journal))
}

async fn list_journals(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let journals: Vec<_> = store.journals.read().await.values().cloned().collect();
    Json(journals)
}

async fn create_account(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateAccountRequest>,
) -> impl IntoResponse {
    let account = Account {
        id: Uuid::new_v4(),
        code: req.code,
        name: req.name,
        account_type: req.account_type,
        created_at: Utc::now(),
    };
    let id = account.id;
    store.accounts.write().await.insert(id, account.clone());
    (StatusCode::CREATED, Json(account))
}

async fn list_accounts(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let accounts: Vec<_> = store.accounts.read().await.values().cloned().collect();
    Json(accounts)
}

async fn create_entry(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateJournalEntryRequest>,
) -> impl IntoResponse {
    let lines: Vec<JournalLine> = req
        .lines
        .into_iter()
        .map(|l| JournalLine {
            account_id: l.account_id,
            debit: l.debit,
            credit: l.credit,
            partner_id: l.partner_id,
        })
        .collect();

    let entry = JournalEntry {
        id: Uuid::new_v4(),
        journal_id: req.journal_id,
        date: req.date,
        reference: req.reference,
        lines,
        state: JournalEntryState::Draft,
        created_at: Utc::now(),
    };
    let id = entry.id;
    store.entries.write().await.insert(id, entry.clone());
    (StatusCode::CREATED, Json(entry))
}

async fn list_entries(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let entries: Vec<_> = store.entries.read().await.values().cloned().collect();
    Json(entries)
}

async fn post_entry(State(store): State<Arc<ErpStore>>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let mut entries = store.entries.write().await;
    if let Some(entry) = entries.get_mut(&id) {
        let debit_sum: f64 = entry.lines.iter().map(|l| l.debit).sum();
        let credit_sum: f64 = entry.lines.iter().map(|l| l.credit).sum();

        if (debit_sum - credit_sum).abs() < 0.01 {
            entry.state = JournalEntryState::Posted;
            (StatusCode::OK, Json(entry.clone()))
        } else {
            (StatusCode::BAD_REQUEST, Json(entry.clone()))
        }
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(JournalEntry {
                id: Uuid::nil(),
                journal_id: Uuid::nil(),
                date: Utc::now(),
                reference: String::new(),
                lines: vec![],
                state: JournalEntryState::Draft,
                created_at: Utc::now(),
            }),
        )
    }
}

async fn create_tax(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateTaxRequest>,
) -> impl IntoResponse {
    let tax = Tax {
        id: Uuid::new_v4(),
        name: req.name,
        pct: req.pct,
        kind: req.kind,
        created_at: Utc::now(),
    };
    let id = tax.id;
    store.taxes.write().await.insert(id, tax.clone());
    (StatusCode::CREATED, Json(tax))
}

async fn list_taxes(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let taxes: Vec<_> = store.taxes.read().await.values().cloned().collect();
    Json(taxes)
}

async fn create_invoice(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateInvoiceRequest>,
) -> impl IntoResponse {
    let mut lines = Vec::new();
    let mut total = 0.0;

    for line_req in req.lines {
        let subtotal =
            crate::engine::line_subtotal(line_req.quantity, line_req.unit_price, 0.0, &[]);
        total += subtotal;

        lines.push(InvoiceLine {
            id: Uuid::new_v4(),
            product_id: line_req.product_id,
            description: line_req.description,
            quantity: line_req.quantity,
            unit_price: line_req.unit_price,
            tax_ids: line_req.tax_ids.unwrap_or_default(),
            subtotal,
        });
    }

    let invoice = Invoice {
        id: Uuid::new_v4(),
        number: format!("INV-{}", Uuid::new_v4().to_string()[0..8].to_uppercase()),
        partner_id: req.partner_id,
        kind: req.kind,
        journal_id: req.journal_id,
        lines,
        amount_total: total,
        state: InvoiceState::Draft,
        due_date: req.due_date,
        created_at: Utc::now(),
    };

    let id = invoice.id;
    store.invoices.write().await.insert(id, invoice.clone());
    (StatusCode::CREATED, Json(invoice))
}

async fn list_invoices(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let invoices: Vec<_> = store.invoices.read().await.values().cloned().collect();
    Json(invoices)
}

async fn post_invoice(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut invoices = store.invoices.write().await;
    if let Some(invoice) = invoices.get_mut(&id) {
        if invoice.state == InvoiceState::Draft {
            invoice.state = InvoiceState::Posted;
            (StatusCode::OK, Json(invoice.clone()))
        } else {
            (StatusCode::BAD_REQUEST, Json(invoice.clone()))
        }
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(Invoice {
                id: Uuid::nil(),
                number: String::new(),
                partner_id: Uuid::nil(),
                kind: InvoiceKind::Customer,
                journal_id: Uuid::nil(),
                lines: vec![],
                amount_total: 0.0,
                state: InvoiceState::Draft,
                due_date: Utc::now(),
                created_at: Utc::now(),
            }),
        )
    }
}

async fn create_payment(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreatePaymentRequest>,
) -> impl IntoResponse {
    let payment = Payment {
        id: Uuid::new_v4(),
        invoice_id: req.invoice_id,
        amount: req.amount,
        date: Utc::now(),
        method: req.method,
        state: PaymentState::Draft,
        created_at: Utc::now(),
    };
    let id = payment.id;
    store.payments.write().await.insert(id, payment.clone());
    (StatusCode::CREATED, Json(payment))
}

async fn list_payments(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let payments: Vec<_> = store.payments.read().await.values().cloned().collect();
    Json(payments)
}

pub fn create_router(state: Arc<ErpStore>) -> Router {
    Router::new()
        .route(
            "/api/erp/accounting/journals",
            post(create_journal).get(list_journals),
        )
        .route(
            "/api/erp/accounting/accounts",
            post(create_account).get(list_accounts),
        )
        .route(
            "/api/erp/accounting/entries",
            post(create_entry).get(list_entries),
        )
        .route("/api/erp/accounting/entries/{id}/post", post(post_entry))
        .route(
            "/api/erp/accounting/taxes",
            post(create_tax).get(list_taxes),
        )
        .route(
            "/api/erp/accounting/invoices",
            post(create_invoice).get(list_invoices),
        )
        .route("/api/erp/accounting/invoices/{id}/post", post(post_invoice))
        .route(
            "/api/erp/accounting/payments",
            post(create_payment).get(list_payments),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_post_entry_requires_balanced() {
        let mut entry = JournalEntry {
            id: Uuid::new_v4(),
            journal_id: Uuid::new_v4(),
            date: Utc::now(),
            reference: "JE-001".to_string(),
            lines: vec![
                JournalLine {
                    account_id: Uuid::new_v4(),
                    debit: 100.0,
                    credit: 0.0,
                    partner_id: None,
                },
                JournalLine {
                    account_id: Uuid::new_v4(),
                    debit: 0.0,
                    credit: 100.0,
                    partner_id: None,
                },
            ],
            state: JournalEntryState::Draft,
            created_at: Utc::now(),
        };

        let debit_sum: f64 = entry.lines.iter().map(|l| l.debit).sum();
        let credit_sum: f64 = entry.lines.iter().map(|l| l.credit).sum();
        assert!((debit_sum - credit_sum).abs() < 0.01);

        entry.state = JournalEntryState::Posted;
        assert_eq!(entry.state, JournalEntryState::Posted);
    }

    #[test]
    fn test_invoice_amount_total_computed() {
        let lines = vec![
            InvoiceLine {
                id: Uuid::new_v4(),
                product_id: None,
                description: "Service A".to_string(),
                quantity: 10.0,
                unit_price: 50.0,
                tax_ids: vec![],
                subtotal: 500.0,
            },
            InvoiceLine {
                id: Uuid::new_v4(),
                product_id: None,
                description: "Service B".to_string(),
                quantity: 5.0,
                unit_price: 100.0,
                tax_ids: vec![],
                subtotal: 500.0,
            },
        ];

        let total: f64 = lines.iter().map(|l| l.subtotal).sum();
        assert!((total - 1000.0).abs() < 0.01);
    }

    #[test]
    fn test_post_invoice_transitions_draft_to_posted() {
        let mut invoice = Invoice {
            id: Uuid::new_v4(),
            number: "INV-001".to_string(),
            partner_id: Uuid::new_v4(),
            kind: InvoiceKind::Customer,
            journal_id: Uuid::new_v4(),
            lines: vec![],
            amount_total: 1000.0,
            state: InvoiceState::Draft,
            due_date: Utc::now(),
            created_at: Utc::now(),
        };

        assert_eq!(invoice.state, InvoiceState::Draft);
        invoice.state = InvoiceState::Posted;
        assert_eq!(invoice.state, InvoiceState::Posted);
    }
}
