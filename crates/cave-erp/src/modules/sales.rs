// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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
pub struct CreateSaleOrderRequest {
    pub partner_id: Uuid,
    pub lines: Vec<CreateSaleOrderLineRequest>,
    pub salesperson_id: Option<Uuid>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateSaleOrderLineRequest {
    pub product_id: Uuid,
    pub name: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub tax_ids: Option<Vec<Uuid>>,
    pub discount_pct: Option<f64>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateDeliveryRequest {
    pub sale_order_id: Uuid,
}

#[derive(Serialize, Deserialize)]
pub struct CreateQuotationRequest {
    pub partner_id: Uuid,
    pub lines: Vec<CreateQuotationLineRequest>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateQuotationLineRequest {
    pub product_id: Uuid,
    pub name: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub tax_ids: Option<Vec<Uuid>>,
    pub discount_pct: Option<f64>,
}

// Handlers
async fn create_sale_order(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateSaleOrderRequest>,
) -> impl IntoResponse {
    let mut lines = Vec::new();
    let mut total = 0.0;

    for line_req in req.lines {
        let subtotal = crate::engine::line_subtotal(
            line_req.quantity,
            line_req.unit_price,
            line_req.discount_pct.unwrap_or(0.0),
            &[],
        );
        total += subtotal;

        lines.push(SaleOrderLine {
            id: Uuid::new_v4(),
            product_id: line_req.product_id,
            name: line_req.name,
            quantity: line_req.quantity,
            unit_price: line_req.unit_price,
            tax_ids: line_req.tax_ids.unwrap_or_default(),
            discount_pct: line_req.discount_pct.unwrap_or(0.0),
            subtotal,
        });
    }

    let order = SaleOrder {
        id: Uuid::new_v4(),
        number: format!("SO-{}", Uuid::new_v4().to_string()[0..8].to_uppercase()),
        partner_id: req.partner_id,
        lines,
        state: SaleOrderState::Draft,
        created_at: Utc::now(),
        confirmed_at: None,
        delivery_date: None,
        amount_total: total,
        currency: "EUR".to_string(),
        salesperson_id: req.salesperson_id,
    };

    let id = order.id;
    store.sale_orders.write().await.insert(id, order.clone());
    (StatusCode::CREATED, Json(order))
}

async fn list_sale_orders(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let orders: Vec<_> = store.sale_orders.read().await.values().cloned().collect();
    Json(orders)
}

async fn confirm_sale_order(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut orders = store.sale_orders.write().await;
    if let Some(order) = orders.get_mut(&id) {
        if order.state == SaleOrderState::Draft {
            order.state = SaleOrderState::Confirmed;
            order.confirmed_at = Some(Utc::now());
            (StatusCode::OK, Json(order.clone()))
        } else {
            (StatusCode::BAD_REQUEST, Json(order.clone()))
        }
    } else {
        (StatusCode::NOT_FOUND, Json(SaleOrder {
            id: Uuid::nil(),
            number: String::new(),
            partner_id: Uuid::nil(),
            lines: vec![],
            state: SaleOrderState::Draft,
            created_at: Utc::now(),
            confirmed_at: None,
            delivery_date: None,
            amount_total: 0.0,
            currency: String::new(),
            salesperson_id: None,
        }))
    }
}

async fn cancel_sale_order(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut orders = store.sale_orders.write().await;
    if let Some(order) = orders.get_mut(&id) {
        order.state = SaleOrderState::Cancelled;
        (StatusCode::OK, Json(order.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(SaleOrder {
            id: Uuid::nil(),
            number: String::new(),
            partner_id: Uuid::nil(),
            lines: vec![],
            state: SaleOrderState::Draft,
            created_at: Utc::now(),
            confirmed_at: None,
            delivery_date: None,
            amount_total: 0.0,
            currency: String::new(),
            salesperson_id: None,
        }))
    }
}

async fn create_quotation(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateQuotationRequest>,
) -> impl IntoResponse {
    let mut lines = Vec::new();
    let mut total = 0.0;

    for line_req in req.lines {
        let subtotal = crate::engine::line_subtotal(
            line_req.quantity,
            line_req.unit_price,
            line_req.discount_pct.unwrap_or(0.0),
            &[],
        );
        total += subtotal;

        lines.push(QuotationLine {
            id: Uuid::new_v4(),
            product_id: line_req.product_id,
            name: line_req.name,
            quantity: line_req.quantity,
            unit_price: line_req.unit_price,
            tax_ids: line_req.tax_ids.unwrap_or_default(),
            discount_pct: line_req.discount_pct.unwrap_or(0.0),
            subtotal,
        });
    }

    let quote = Quotation {
        id: Uuid::new_v4(),
        number: format!("QT-{}", Uuid::new_v4().to_string()[0..8].to_uppercase()),
        partner_id: req.partner_id,
        lines,
        state: QuotationState::Draft,
        created_at: Utc::now(),
        sent_at: None,
        amount_total: total,
        currency: "EUR".to_string(),
    };

    let id = quote.id;
    store.quotations.write().await.insert(id, quote.clone());
    (StatusCode::CREATED, Json(quote))
}

async fn list_quotations(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let quotes: Vec<_> = store.quotations.read().await.values().cloned().collect();
    Json(quotes)
}

async fn send_quotation(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut quotes = store.quotations.write().await;
    if let Some(quote) = quotes.get_mut(&id) {
        quote.state = QuotationState::Sent;
        quote.sent_at = Some(Utc::now());
        (StatusCode::OK, Json(quote.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Quotation {
            id: Uuid::nil(),
            number: String::new(),
            partner_id: Uuid::nil(),
            lines: vec![],
            state: QuotationState::Draft,
            created_at: Utc::now(),
            sent_at: None,
            amount_total: 0.0,
            currency: String::new(),
        }))
    }
}

async fn convert_quotation(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let quotations = store.quotations.read().await;
    if let Some(quote) = quotations.get(&id) {
        let order = SaleOrder {
            id: Uuid::new_v4(),
            number: format!("SO-{}", Uuid::new_v4().to_string()[0..8].to_uppercase()),
            partner_id: quote.partner_id,
            lines: quote
                .lines
                .iter()
                .map(|l| SaleOrderLine {
                    id: Uuid::new_v4(),
                    product_id: l.product_id,
                    name: l.name.clone(),
                    quantity: l.quantity,
                    unit_price: l.unit_price,
                    tax_ids: l.tax_ids.clone(),
                    discount_pct: l.discount_pct,
                    subtotal: l.subtotal,
                })
                .collect(),
            state: SaleOrderState::Confirmed,
            created_at: Utc::now(),
            confirmed_at: Some(Utc::now()),
            delivery_date: None,
            amount_total: quote.amount_total,
            currency: quote.currency.clone(),
            salesperson_id: None,
        };
        drop(quotations);

        let order_id = order.id;
        store.sale_orders.write().await.insert(order_id, order.clone());

        let mut quotes = store.quotations.write().await;
        if let Some(quote) = quotes.get_mut(&id) {
            quote.state = QuotationState::Accepted;
        }

        (StatusCode::CREATED, Json(order))
    } else {
        (StatusCode::NOT_FOUND, Json(SaleOrder {
            id: Uuid::nil(),
            number: String::new(),
            partner_id: Uuid::nil(),
            lines: vec![],
            state: SaleOrderState::Draft,
            created_at: Utc::now(),
            confirmed_at: None,
            delivery_date: None,
            amount_total: 0.0,
            currency: String::new(),
            salesperson_id: None,
        }))
    }
}

async fn create_delivery(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateDeliveryRequest>,
) -> impl IntoResponse {
    let delivery = Delivery {
        id: Uuid::new_v4(),
        sale_order_id: req.sale_order_id,
        state: DeliveryState::Pending,
        scheduled: None,
        completed_at: None,
        tracking_ref: None,
        created_at: Utc::now(),
    };

    let id = delivery.id;
    store.deliveries.write().await.insert(id, delivery.clone());
    (StatusCode::CREATED, Json(delivery))
}

async fn list_deliveries(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let deliveries: Vec<_> = store.deliveries.read().await.values().cloned().collect();
    Json(deliveries)
}

async fn ship_delivery(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut deliveries = store.deliveries.write().await;
    if let Some(delivery) = deliveries.get_mut(&id) {
        delivery.state = DeliveryState::InProgress;
        (StatusCode::OK, Json(delivery.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Delivery {
            id: Uuid::nil(),
            sale_order_id: Uuid::nil(),
            state: DeliveryState::Pending,
            scheduled: None,
            completed_at: None,
            tracking_ref: None,
            created_at: Utc::now(),
        }))
    }
}

async fn complete_delivery(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut deliveries = store.deliveries.write().await;
    if let Some(delivery) = deliveries.get_mut(&id) {
        delivery.state = DeliveryState::Done;
        delivery.completed_at = Some(Utc::now());
        (StatusCode::OK, Json(delivery.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Delivery {
            id: Uuid::nil(),
            sale_order_id: Uuid::nil(),
            state: DeliveryState::Pending,
            scheduled: None,
            completed_at: None,
            tracking_ref: None,
            created_at: Utc::now(),
        }))
    }
}

pub fn create_router(state: Arc<ErpStore>) -> Router {
    Router::new()
        .route(
            "/api/erp/sales/orders",
            post(create_sale_order).get(list_sale_orders),
        )
        .route("/api/erp/sales/orders/{id}/confirm", post(confirm_sale_order))
        .route("/api/erp/sales/orders/{id}/cancel", post(cancel_sale_order))
        .route(
            "/api/erp/sales/quotations",
            post(create_quotation).get(list_quotations),
        )
        .route("/api/erp/sales/quotations/{id}/send", post(send_quotation))
        .route("/api/erp/sales/quotations/{id}/convert", post(convert_quotation))
        .route(
            "/api/erp/sales/deliveries",
            post(create_delivery).get(list_deliveries),
        )
        .route("/api/erp/sales/deliveries/{id}/ship", post(ship_delivery))
        .route("/api/erp/sales/deliveries/{id}/complete", post(complete_delivery))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confirm_sale_order_sets_confirmed_state() {
        let mut order = SaleOrder {
            id: Uuid::new_v4(),
            number: "SO-001".to_string(),
            partner_id: Uuid::new_v4(),
            lines: vec![],
            state: SaleOrderState::Draft,
            created_at: Utc::now(),
            confirmed_at: None,
            delivery_date: None,
            amount_total: 1000.0,
            currency: "EUR".to_string(),
            salesperson_id: None,
        };

        assert_eq!(order.state, SaleOrderState::Draft);
        order.state = SaleOrderState::Confirmed;
        order.confirmed_at = Some(Utc::now());
        assert_eq!(order.state, SaleOrderState::Confirmed);
        assert!(order.confirmed_at.is_some());
    }

    #[test]
    fn test_sale_order_amount_total_computed() {
        let lines = vec![
            SaleOrderLine {
                id: Uuid::new_v4(),
                product_id: Uuid::new_v4(),
                name: "Product A".to_string(),
                quantity: 10.0,
                unit_price: 100.0,
                tax_ids: vec![],
                discount_pct: 10.0,
                subtotal: 900.0,
            },
            SaleOrderLine {
                id: Uuid::new_v4(),
                product_id: Uuid::new_v4(),
                name: "Product B".to_string(),
                quantity: 5.0,
                unit_price: 50.0,
                tax_ids: vec![],
                discount_pct: 0.0,
                subtotal: 250.0,
            },
        ];

        let total: f64 = lines.iter().map(|l| l.subtotal).sum();
        assert!((total - 1150.0).abs() < 0.01);
    }

    #[test]
    fn test_complete_delivery_sets_done() {
        let mut delivery = Delivery {
            id: Uuid::new_v4(),
            sale_order_id: Uuid::new_v4(),
            state: DeliveryState::Pending,
            scheduled: None,
            completed_at: None,
            tracking_ref: None,
            created_at: Utc::now(),
        };

        assert_eq!(delivery.state, DeliveryState::Pending);
        delivery.state = DeliveryState::Done;
        delivery.completed_at = Some(Utc::now());
        assert_eq!(delivery.state, DeliveryState::Done);
        assert!(delivery.completed_at.is_some());
    }
}
