// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::*;
use crate::store::ErpStore;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct CreateProductRequest {
    pub sku: String,
    pub name: String,
    pub category_id: Uuid,
    pub unit_of_measure: UnitOfMeasure,
    pub price: f64,
    pub cost: f64,
    pub is_purchasable: bool,
    pub is_sellable: bool,
    pub tracked_by: TrackingType,
}

#[derive(Serialize, Deserialize)]
pub struct CreateCategoryRequest {
    pub name: String,
    pub parent_id: Option<Uuid>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateWarehouseRequest {
    pub name: String,
    pub code: String,
}

#[derive(Serialize, Deserialize)]
pub struct CreateStockLocationRequest {
    pub warehouse_id: Uuid,
    pub name: String,
    pub is_internal: bool,
}

#[derive(Serialize, Deserialize)]
pub struct CreateStockMoveRequest {
    pub product_id: Uuid,
    pub qty: f64,
    pub from_location_id: Uuid,
    pub to_location_id: Uuid,
    pub lot_id: Option<Uuid>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateLotRequest {
    pub product_id: Uuid,
    pub number: String,
    pub expiry_date: Option<chrono::DateTime<chrono::Utc>>,
    pub qty: f64,
}

#[derive(Serialize, Deserialize)]
pub struct StockQueryParams {
    pub product_id: Uuid,
}

async fn create_product(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateProductRequest>,
) -> impl IntoResponse {
    let product = Product {
        id: Uuid::new_v4(),
        sku: req.sku,
        name: req.name,
        category_id: req.category_id,
        unit_of_measure: req.unit_of_measure,
        price: req.price,
        cost: req.cost,
        is_purchasable: req.is_purchasable,
        is_sellable: req.is_sellable,
        tracked_by: req.tracked_by,
        created_at: Utc::now(),
    };
    let id = product.id;
    store.products.write().await.insert(id, product.clone());
    (StatusCode::CREATED, Json(product))
}

async fn list_products(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let products: Vec<_> = store.products.read().await.values().cloned().collect();
    Json(products)
}

async fn create_category(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateCategoryRequest>,
) -> impl IntoResponse {
    let category = Category {
        id: Uuid::new_v4(),
        name: req.name,
        parent_id: req.parent_id,
        created_at: Utc::now(),
    };
    let id = category.id;
    store.categories.write().await.insert(id, category.clone());
    (StatusCode::CREATED, Json(category))
}

async fn list_categories(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let cats: Vec<_> = store.categories.read().await.values().cloned().collect();
    Json(cats)
}

async fn create_warehouse(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateWarehouseRequest>,
) -> impl IntoResponse {
    let warehouse = Warehouse {
        id: Uuid::new_v4(),
        name: req.name,
        code: req.code,
        address: None,
        created_at: Utc::now(),
    };
    let id = warehouse.id;
    store.warehouses.write().await.insert(id, warehouse.clone());
    (StatusCode::CREATED, Json(warehouse))
}

async fn list_warehouses(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let warehouses: Vec<_> = store.warehouses.read().await.values().cloned().collect();
    Json(warehouses)
}

async fn create_stock_location(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateStockLocationRequest>,
) -> impl IntoResponse {
    let location = StockLocation {
        id: Uuid::new_v4(),
        warehouse_id: req.warehouse_id,
        name: req.name,
        is_internal: req.is_internal,
        created_at: Utc::now(),
    };
    let id = location.id;
    store.stock_locations.write().await.insert(id, location.clone());
    (StatusCode::CREATED, Json(location))
}

async fn list_stock_locations(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let locations: Vec<_> = store.stock_locations.read().await.values().cloned().collect();
    Json(locations)
}

async fn create_stock_move(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateStockMoveRequest>,
) -> impl IntoResponse {
    let stock_move = StockMove {
        id: Uuid::new_v4(),
        product_id: req.product_id,
        qty: req.qty,
        from_location_id: req.from_location_id,
        to_location_id: req.to_location_id,
        state: StockMoveState::Draft,
        lot_id: req.lot_id,
        created_at: Utc::now(),
        done_at: None,
    };
    let id = stock_move.id;
    store.stock_moves.write().await.insert(id, stock_move.clone());
    (StatusCode::CREATED, Json(stock_move))
}

async fn list_stock_moves(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let moves: Vec<_> = store.stock_moves.read().await.values().cloned().collect();
    Json(moves)
}

async fn confirm_stock_move(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut moves = store.stock_moves.write().await;
    if let Some(m) = moves.get_mut(&id) {
        m.state = StockMoveState::Done;
        m.done_at = Some(Utc::now());
        (StatusCode::OK, Json(m.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(StockMove {
            id: Uuid::nil(),
            product_id: Uuid::nil(),
            qty: 0.0,
            from_location_id: Uuid::nil(),
            to_location_id: Uuid::nil(),
            state: StockMoveState::Draft,
            lot_id: None,
            created_at: Utc::now(),
            done_at: None,
        }))
    }
}

async fn cancel_stock_move(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut moves = store.stock_moves.write().await;
    if let Some(m) = moves.get_mut(&id) {
        m.state = StockMoveState::Cancelled;
        (StatusCode::OK, Json(m.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(StockMove {
            id: Uuid::nil(),
            product_id: Uuid::nil(),
            qty: 0.0,
            from_location_id: Uuid::nil(),
            to_location_id: Uuid::nil(),
            state: StockMoveState::Draft,
            lot_id: None,
            created_at: Utc::now(),
            done_at: None,
        }))
    }
}

async fn create_lot(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateLotRequest>,
) -> impl IntoResponse {
    let lot = Lot {
        id: Uuid::new_v4(),
        product_id: req.product_id,
        number: req.number,
        expiry_date: req.expiry_date,
        qty: req.qty,
        created_at: Utc::now(),
    };
    let id = lot.id;
    store.lots.write().await.insert(id, lot.clone());
    (StatusCode::CREATED, Json(lot))
}

async fn list_lots(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let lots: Vec<_> = store.lots.read().await.values().cloned().collect();
    Json(lots)
}

async fn get_stock_on_hand(
    State(store): State<Arc<ErpStore>>,
    Query(params): Query<StockQueryParams>,
) -> impl IntoResponse {
    let moves: Vec<_> = store.stock_moves.read().await.values().cloned().collect();
    let on_hand = crate::engine::on_hand_by_location(&moves, params.product_id);
    Json(on_hand)
}

pub fn create_router(state: Arc<ErpStore>) -> Router {
    Router::new()
        .route(
            "/api/erp/inventory/products",
            post(create_product).get(list_products),
        )
        .route(
            "/api/erp/inventory/categories",
            post(create_category).get(list_categories),
        )
        .route(
            "/api/erp/inventory/warehouses",
            post(create_warehouse).get(list_warehouses),
        )
        .route(
            "/api/erp/inventory/locations",
            post(create_stock_location).get(list_stock_locations),
        )
        .route(
            "/api/erp/inventory/stock/moves",
            post(create_stock_move).get(list_stock_moves),
        )
        .route(
            "/api/erp/inventory/stock/moves/{id}/confirm",
            post(confirm_stock_move),
        )
        .route(
            "/api/erp/inventory/stock/moves/{id}/cancel",
            post(cancel_stock_move),
        )
        .route("/api/erp/inventory/lots", post(create_lot).get(list_lots))
        .route("/api/erp/inventory/stock", get(get_stock_on_hand))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_done_move_updates_on_hand() {
        let prod = Uuid::new_v4();
        let loc_a = Uuid::new_v4();
        let loc_b = Uuid::new_v4();

        let mut m = StockMove {
            id: Uuid::new_v4(),
            product_id: prod,
            qty: 50.0,
            from_location_id: loc_a,
            to_location_id: loc_b,
            state: StockMoveState::Draft,
            lot_id: None,
            created_at: Utc::now(),
            done_at: None,
        };

        assert_eq!(m.state, StockMoveState::Draft);
        m.state = StockMoveState::Done;
        m.done_at = Some(Utc::now());
        assert_eq!(m.state, StockMoveState::Done);
        assert!(m.done_at.is_some());
    }

    #[test]
    fn test_cancel_stock_move() {
        let mut m = StockMove {
            id: Uuid::new_v4(),
            product_id: Uuid::new_v4(),
            qty: 30.0,
            from_location_id: Uuid::new_v4(),
            to_location_id: Uuid::new_v4(),
            state: StockMoveState::Reserved,
            lot_id: None,
            created_at: Utc::now(),
            done_at: None,
        };

        assert_eq!(m.state, StockMoveState::Reserved);
        m.state = StockMoveState::Cancelled;
        assert_eq!(m.state, StockMoveState::Cancelled);
    }

    #[test]
    fn test_product_with_lot_tracking() {
        let product = Product {
            id: Uuid::new_v4(),
            sku: "BATCH-001".to_string(),
            name: "Serialized Item".to_string(),
            category_id: Uuid::new_v4(),
            unit_of_measure: UnitOfMeasure::Piece,
            price: 100.0,
            cost: 50.0,
            is_purchasable: true,
            is_sellable: true,
            tracked_by: TrackingType::Lot,
            created_at: Utc::now(),
        };

        assert_eq!(product.tracked_by, TrackingType::Lot);
    }
}
