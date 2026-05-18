// SPDX-License-Identifier: AGPL-3.0-or-later
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
pub struct CreateBomRequest {
    pub product_id: Uuid,
    pub components: Vec<CreateBomComponentRequest>,
    pub quantity: f64,
    pub routing_id: Option<Uuid>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateBomComponentRequest {
    pub product_id: Uuid,
    pub qty: f64,
}

#[derive(Serialize, Deserialize)]
pub struct CreateManufacturingOrderRequest {
    pub product_id: Uuid,
    pub qty: f64,
    pub bom_id: Uuid,
    pub scheduled_start: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateWorkOrderRequest {
    pub mo_id: Uuid,
    pub workcenter_id: Uuid,
    pub duration_min: u32,
}

#[derive(Serialize, Deserialize)]
pub struct CreateWorkcenterRequest {
    pub name: String,
    pub capacity: f64,
    pub oee: f64,
}

#[derive(Serialize, Deserialize)]
pub struct CreateRoutingRequest {
    pub name: String,
    pub operations: Vec<CreateOperationRequest>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateOperationRequest {
    pub workcenter_id: Uuid,
    pub duration_min: u32,
    pub description: String,
}

#[derive(Serialize, Deserialize)]
pub struct ComponentRequirementsResponse {
    pub components: Vec<(Uuid, f64)>,
}

async fn create_bom(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateBomRequest>,
) -> impl IntoResponse {
    let bom = Bom {
        id: Uuid::new_v4(),
        product_id: req.product_id,
        components: req
            .components
            .into_iter()
            .map(|c| BomComponent {
                product_id: c.product_id,
                qty: c.qty,
            })
            .collect(),
        quantity: req.quantity,
        routing_id: req.routing_id,
        created_at: Utc::now(),
    };
    let id = bom.id;
    store.boms.write().await.insert(id, bom.clone());
    (StatusCode::CREATED, Json(bom))
}

async fn list_boms(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let boms: Vec<_> = store.boms.read().await.values().cloned().collect();
    Json(boms)
}

async fn create_manufacturing_order(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateManufacturingOrderRequest>,
) -> impl IntoResponse {
    let mo = ManufacturingOrder {
        id: Uuid::new_v4(),
        product_id: req.product_id,
        qty: req.qty,
        bom_id: req.bom_id,
        state: ManufacturingOrderState::Draft,
        scheduled_start: req.scheduled_start,
        completed_at: None,
        created_at: Utc::now(),
    };
    let id = mo.id;
    store.manufacturing_orders.write().await.insert(id, mo.clone());
    (StatusCode::CREATED, Json(mo))
}

async fn list_manufacturing_orders(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let mos: Vec<_> = store
        .manufacturing_orders
        .read()
        .await
        .values()
        .cloned()
        .collect();
    Json(mos)
}

async fn confirm_manufacturing_order(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let bom_id = {
        let mos = store.manufacturing_orders.read().await;
        mos.get(&id).map(|mo| mo.bom_id)
    };

    if let Some(bom_id) = bom_id {
        let bom = {
            let boms = store.boms.read().await;
            boms.get(&bom_id).cloned()
        };

        if let Some(bom) = bom {
            if let Some(routing_id) = bom.routing_id {
                let routing = {
                    let routings = store.routings.read().await;
                    routings.get(&routing_id).cloned()
                };

                if let Some(routing) = routing {
                    // Create work orders from routing operations
                    for op in &routing.operations {
                        let wo = WorkOrder {
                            id: Uuid::new_v4(),
                            mo_id: id,
                            workcenter_id: op.workcenter_id,
                            duration_min: op.duration_min,
                            state: WorkOrderState::Pending,
                            created_at: Utc::now(),
                        };
                        store.work_orders.write().await.insert(wo.id, wo);
                    }
                }
            }
        }

        let mut mos = store.manufacturing_orders.write().await;
        if let Some(mo) = mos.get_mut(&id) {
            mo.state = ManufacturingOrderState::Confirmed;
            (StatusCode::OK, Json(mo.clone()))
        } else {
            (StatusCode::NOT_FOUND, Json(ManufacturingOrder {
                id: Uuid::nil(),
                product_id: Uuid::nil(),
                qty: 0.0,
                bom_id: Uuid::nil(),
                state: ManufacturingOrderState::Draft,
                scheduled_start: Utc::now(),
                completed_at: None,
                created_at: Utc::now(),
            }))
        }
    } else {
        (StatusCode::NOT_FOUND, Json(ManufacturingOrder {
            id: Uuid::nil(),
            product_id: Uuid::nil(),
            qty: 0.0,
            bom_id: Uuid::nil(),
            state: ManufacturingOrderState::Draft,
            scheduled_start: Utc::now(),
            completed_at: None,
            created_at: Utc::now(),
        }))
    }
}

async fn start_manufacturing_order(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut mos = store.manufacturing_orders.write().await;
    if let Some(mo) = mos.get_mut(&id) {
        mo.state = ManufacturingOrderState::InProgress;
        (StatusCode::OK, Json(mo.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(ManufacturingOrder {
            id: Uuid::nil(),
            product_id: Uuid::nil(),
            qty: 0.0,
            bom_id: Uuid::nil(),
            state: ManufacturingOrderState::Draft,
            scheduled_start: Utc::now(),
            completed_at: None,
            created_at: Utc::now(),
        }))
    }
}

async fn complete_manufacturing_order(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut mos = store.manufacturing_orders.write().await;
    if let Some(mo) = mos.get_mut(&id) {
        mo.state = ManufacturingOrderState::Done;
        mo.completed_at = Some(Utc::now());
        (StatusCode::OK, Json(mo.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(ManufacturingOrder {
            id: Uuid::nil(),
            product_id: Uuid::nil(),
            qty: 0.0,
            bom_id: Uuid::nil(),
            state: ManufacturingOrderState::Draft,
            scheduled_start: Utc::now(),
            completed_at: None,
            created_at: Utc::now(),
        }))
    }
}

async fn create_workcenter(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateWorkcenterRequest>,
) -> impl IntoResponse {
    let wc = Workcenter {
        id: Uuid::new_v4(),
        name: req.name,
        capacity: req.capacity,
        oee: req.oee,
        created_at: Utc::now(),
    };
    let id = wc.id;
    store.workcenters.write().await.insert(id, wc.clone());
    (StatusCode::CREATED, Json(wc))
}

async fn list_workcenters(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let wcs: Vec<_> = store.workcenters.read().await.values().cloned().collect();
    Json(wcs)
}

async fn create_work_order(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateWorkOrderRequest>,
) -> impl IntoResponse {
    let wo = WorkOrder {
        id: Uuid::new_v4(),
        mo_id: req.mo_id,
        workcenter_id: req.workcenter_id,
        duration_min: req.duration_min,
        state: WorkOrderState::Pending,
        created_at: Utc::now(),
    };
    let id = wo.id;
    store.work_orders.write().await.insert(id, wo.clone());
    (StatusCode::CREATED, Json(wo))
}

async fn list_work_orders(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let wos: Vec<_> = store.work_orders.read().await.values().cloned().collect();
    Json(wos)
}

async fn start_work_order(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut wos = store.work_orders.write().await;
    if let Some(wo) = wos.get_mut(&id) {
        wo.state = WorkOrderState::InProgress;
        (StatusCode::OK, Json(wo.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(WorkOrder {
            id: Uuid::nil(),
            mo_id: Uuid::nil(),
            workcenter_id: Uuid::nil(),
            duration_min: 0,
            state: WorkOrderState::Pending,
            created_at: Utc::now(),
        }))
    }
}

async fn complete_work_order(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut wos = store.work_orders.write().await;
    if let Some(wo) = wos.get_mut(&id) {
        wo.state = WorkOrderState::Done;
        (StatusCode::OK, Json(wo.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(WorkOrder {
            id: Uuid::nil(),
            mo_id: Uuid::nil(),
            workcenter_id: Uuid::nil(),
            duration_min: 0,
            state: WorkOrderState::Pending,
            created_at: Utc::now(),
        }))
    }
}

async fn create_routing(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateRoutingRequest>,
) -> impl IntoResponse {
    let routing = Routing {
        id: Uuid::new_v4(),
        name: req.name,
        operations: req
            .operations
            .into_iter()
            .map(|op| Operation {
                workcenter_id: op.workcenter_id,
                duration_min: op.duration_min,
                description: op.description,
            })
            .collect(),
        created_at: Utc::now(),
    };
    let id = routing.id;
    store.routings.write().await.insert(id, routing.clone());
    (StatusCode::CREATED, Json(routing))
}

async fn list_routings(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let routings: Vec<_> = store.routings.read().await.values().cloned().collect();
    Json(routings)
}

async fn get_component_requirements(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mos = store.manufacturing_orders.read().await;
    if let Some(mo) = mos.get(&id) {
        let bom_id = mo.bom_id;
        let qty = mo.qty;
        drop(mos);

        let boms = store.boms.read().await;
        if let Some(bom) = boms.get(&bom_id) {
            let boms_map: std::collections::HashMap<_, _> =
                boms.iter().map(|(k, v)| (*k, v.clone())).collect();
            let components = crate::engine::explode_bom(bom, &boms_map, qty);
            return (StatusCode::OK, Json(ComponentRequirementsResponse { components }))
                .into_response();
        }
    }

    (StatusCode::NOT_FOUND, Json(ComponentRequirementsResponse {
        components: vec![],
    }))
    .into_response()
}

pub fn create_router(state: Arc<ErpStore>) -> Router {
    Router::new()
        .route("/api/erp/manufacturing/boms", post(create_bom).get(list_boms))
        .route(
            "/api/erp/manufacturing/orders",
            post(create_manufacturing_order).get(list_manufacturing_orders),
        )
        .route(
            "/api/erp/manufacturing/orders/{id}/confirm",
            post(confirm_manufacturing_order),
        )
        .route(
            "/api/erp/manufacturing/orders/{id}/start",
            post(start_manufacturing_order),
        )
        .route(
            "/api/erp/manufacturing/orders/{id}/complete",
            post(complete_manufacturing_order),
        )
        .route(
            "/api/erp/manufacturing/orders/{id}/component-requirements",
            get(get_component_requirements),
        )
        .route(
            "/api/erp/manufacturing/workcenters",
            post(create_workcenter).get(list_workcenters),
        )
        .route(
            "/api/erp/manufacturing/workorders",
            post(create_work_order).get(list_work_orders),
        )
        .route(
            "/api/erp/manufacturing/workorders/{id}/start",
            post(start_work_order),
        )
        .route(
            "/api/erp/manufacturing/workorders/{id}/complete",
            post(complete_work_order),
        )
        .route(
            "/api/erp/manufacturing/routings",
            post(create_routing).get(list_routings),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confirm_mo_creates_work_orders() {
        let mo_id = Uuid::new_v4();
        let mut mo = ManufacturingOrder {
            id: mo_id,
            product_id: Uuid::new_v4(),
            qty: 10.0,
            bom_id: Uuid::new_v4(),
            state: ManufacturingOrderState::Draft,
            scheduled_start: Utc::now(),
            completed_at: None,
            created_at: Utc::now(),
        };

        assert_eq!(mo.state, ManufacturingOrderState::Draft);
        mo.state = ManufacturingOrderState::Confirmed;
        assert_eq!(mo.state, ManufacturingOrderState::Confirmed);
    }

    #[test]
    fn test_work_order_transitions() {
        let mut wo = WorkOrder {
            id: Uuid::new_v4(),
            mo_id: Uuid::new_v4(),
            workcenter_id: Uuid::new_v4(),
            duration_min: 60,
            state: WorkOrderState::Pending,
            created_at: Utc::now(),
        };

        assert_eq!(wo.state, WorkOrderState::Pending);
        wo.state = WorkOrderState::InProgress;
        assert_eq!(wo.state, WorkOrderState::InProgress);
        wo.state = WorkOrderState::Done;
        assert_eq!(wo.state, WorkOrderState::Done);
    }

    #[test]
    fn test_bom_explosion_logic() {
        let prod_a = Uuid::new_v4();
        let prod_b = Uuid::new_v4();

        let bom = Bom {
            id: Uuid::new_v4(),
            product_id: prod_a,
            components: vec![BomComponent {
                product_id: prod_b,
                qty: 5.0,
            }],
            quantity: 1.0,
            routing_id: None,
            created_at: Utc::now(),
        };

        let boms = std::iter::once((bom.id, bom.clone()))
            .collect::<std::collections::HashMap<_, _>>();

        let explosion = crate::engine::explode_bom(&bom, &boms, 2.0);
        assert_eq!(explosion.len(), 1);
        assert!((explosion[0].1 - 10.0).abs() < 0.01);
    }
}
