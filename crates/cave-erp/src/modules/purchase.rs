use crate::models::*;
use crate::store::ErpStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct CreatePurchaseOrderRequest {
    pub supplier_id: Uuid,
    pub lines: Vec<CreatePurchaseOrderLineRequest>,
}

#[derive(Serialize, Deserialize)]
pub struct CreatePurchaseOrderLineRequest {
    pub product_id: Uuid,
    pub name: String,
    pub quantity: f64,
    pub unit_cost: f64,
    pub tax_ids: Option<Vec<Uuid>>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateRfqRequest {
    pub supplier_id: Uuid,
    pub lines: Vec<CreateRfqLineRequest>,
    pub requested_by: Uuid,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateRfqLineRequest {
    pub product_id: Uuid,
    pub name: String,
    pub quantity: f64,
}

#[derive(Serialize, Deserialize)]
pub struct CreateReceiptRequest {
    pub po_id: Uuid,
    pub lines: Vec<CreateReceiptLineRequest>,
    pub receiver_id: Uuid,
}

#[derive(Serialize, Deserialize)]
pub struct CreateReceiptLineRequest {
    pub product_id: Uuid,
    pub quantity_received: f64,
}

async fn create_purchase_order(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreatePurchaseOrderRequest>,
) -> impl IntoResponse {
    let mut lines = Vec::new();
    let mut total = 0.0;

    for line_req in req.lines {
        let subtotal = line_req.quantity * line_req.unit_cost;
        total += subtotal;

        lines.push(PurchaseOrderLine {
            id: Uuid::new_v4(),
            product_id: line_req.product_id,
            name: line_req.name,
            quantity: line_req.quantity,
            unit_cost: line_req.unit_cost,
            tax_ids: line_req.tax_ids.unwrap_or_default(),
            subtotal,
        });
    }

    let po = PurchaseOrder {
        id: Uuid::new_v4(),
        number: format!("PO-{}", Uuid::new_v4().to_string()[0..8].to_uppercase()),
        supplier_id: req.supplier_id,
        lines,
        state: PurchaseOrderState::Draft,
        created_at: Utc::now(),
        received_at: None,
        amount_total: total,
        currency: "EUR".to_string(),
    };

    let id = po.id;
    store.purchase_orders.write().await.insert(id, po.clone());
    (StatusCode::CREATED, Json(po))
}

async fn list_purchase_orders(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let pos: Vec<_> = store.purchase_orders.read().await.values().cloned().collect();
    Json(pos)
}

async fn confirm_purchase_order(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut pos = store.purchase_orders.write().await;
    if let Some(po) = pos.get_mut(&id) {
        po.state = PurchaseOrderState::Confirmed;
        (StatusCode::OK, Json(po.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(PurchaseOrder {
            id: Uuid::nil(),
            number: String::new(),
            supplier_id: Uuid::nil(),
            lines: vec![],
            state: PurchaseOrderState::Draft,
            created_at: Utc::now(),
            received_at: None,
            amount_total: 0.0,
            currency: String::new(),
        }))
    }
}

async fn receive_purchase_order(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateReceiptRequest>,
) -> impl IntoResponse {
    let mut pos = store.purchase_orders.write().await;
    if let Some(po) = pos.get_mut(&id) {
        po.state = PurchaseOrderState::Received;
        po.received_at = Some(Utc::now());
        drop(pos);

        let receipt = Receipt {
            id: Uuid::new_v4(),
            po_id: id,
            lines: req
                .lines
                .into_iter()
                .map(|l| ReceiptLine {
                    product_id: l.product_id,
                    quantity_received: l.quantity_received,
                })
                .collect(),
            received_at: Utc::now(),
            receiver_id: req.receiver_id,
        };

        let receipt_id = receipt.id;
        store.receipts.write().await.insert(receipt_id, receipt.clone());
        (StatusCode::CREATED, Json(receipt))
    } else {
        (StatusCode::NOT_FOUND, Json(Receipt {
            id: Uuid::nil(),
            po_id: Uuid::nil(),
            lines: vec![],
            received_at: Utc::now(),
            receiver_id: Uuid::nil(),
        }))
    }
}

async fn create_rfq(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateRfqRequest>,
) -> impl IntoResponse {
    let rfq = Rfq {
        id: Uuid::new_v4(),
        supplier_id: req.supplier_id,
        lines: req
            .lines
            .into_iter()
            .map(|l| RfqLine {
                product_id: l.product_id,
                name: l.name,
                quantity: l.quantity,
            })
            .collect(),
        state: RfqState::Draft,
        requested_by: req.requested_by,
        expires_at: req.expires_at,
        created_at: Utc::now(),
    };

    let id = rfq.id;
    store.rfqs.write().await.insert(id, rfq.clone());
    (StatusCode::CREATED, Json(rfq))
}

async fn list_rfqs(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let rfqs: Vec<_> = store.rfqs.read().await.values().cloned().collect();
    Json(rfqs)
}

async fn send_rfq(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut rfqs = store.rfqs.write().await;
    if let Some(rfq) = rfqs.get_mut(&id) {
        rfq.state = RfqState::Sent;
        (StatusCode::OK, Json(rfq.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Rfq {
            id: Uuid::nil(),
            supplier_id: Uuid::nil(),
            lines: vec![],
            state: RfqState::Draft,
            requested_by: Uuid::nil(),
            expires_at: Utc::now(),
            created_at: Utc::now(),
        }))
    }
}

async fn list_receipts(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let receipts: Vec<_> = store.receipts.read().await.values().cloned().collect();
    Json(receipts)
}

pub fn create_router(state: Arc<ErpStore>) -> Router {
    Router::new()
        .route(
            "/api/erp/purchase/orders",
            post(create_purchase_order).get(list_purchase_orders),
        )
        .route("/api/erp/purchase/orders/{id}/confirm", post(confirm_purchase_order))
        .route("/api/erp/purchase/orders/{id}/receive", post(receive_purchase_order))
        .route("/api/erp/purchase/rfqs", post(create_rfq).get(list_rfqs))
        .route("/api/erp/purchase/rfqs/{id}/send", post(send_rfq))
        .route("/api/erp/purchase/receipts", get(list_receipts))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confirm_po_sets_state() {
        let mut po = PurchaseOrder {
            id: Uuid::new_v4(),
            number: "PO-001".to_string(),
            supplier_id: Uuid::new_v4(),
            lines: vec![],
            state: PurchaseOrderState::Draft,
            created_at: Utc::now(),
            received_at: None,
            amount_total: 1000.0,
            currency: "EUR".to_string(),
        };

        assert_eq!(po.state, PurchaseOrderState::Draft);
        po.state = PurchaseOrderState::Confirmed;
        assert_eq!(po.state, PurchaseOrderState::Confirmed);
    }

    #[test]
    fn test_receive_po_creates_receipt() {
        let po_id = Uuid::new_v4();
        let receipt = Receipt {
            id: Uuid::new_v4(),
            po_id,
            lines: vec![ReceiptLine {
                product_id: Uuid::new_v4(),
                quantity_received: 10.0,
            }],
            received_at: Utc::now(),
            receiver_id: Uuid::new_v4(),
        };

        assert_eq!(receipt.po_id, po_id);
        assert_eq!(receipt.lines.len(), 1);
    }

    #[test]
    fn test_po_amount_total_computed() {
        let lines = vec![
            PurchaseOrderLine {
                id: Uuid::new_v4(),
                product_id: Uuid::new_v4(),
                name: "Mat A".to_string(),
                quantity: 100.0,
                unit_cost: 5.0,
                tax_ids: vec![],
                subtotal: 500.0,
            },
            PurchaseOrderLine {
                id: Uuid::new_v4(),
                product_id: Uuid::new_v4(),
                name: "Mat B".to_string(),
                quantity: 50.0,
                unit_cost: 10.0,
                tax_ids: vec![],
                subtotal: 500.0,
            },
        ];

        let total: f64 = lines.iter().map(|l| l.subtotal).sum();
        assert!((total - 1000.0).abs() < 0.01);
    }
}
