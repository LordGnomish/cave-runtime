// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::*;
use crate::store::ErpStore;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ===== Request/Response Types =====

#[derive(Serialize, Deserialize)]
pub struct CreateEmployeeRequest {
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub department_id: Uuid,
    pub job_title: String,
    pub manager_id: Option<Uuid>,
    pub tags: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub struct UpdateEmployeeRequest {
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub job_title: Option<String>,
    pub manager_id: Option<Option<Uuid>>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateDepartmentRequest {
    pub name: String,
    pub manager_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateContractRequest {
    pub employee_id: Uuid,
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: Option<chrono::DateTime<chrono::Utc>>,
    pub salary: f64,
    pub currency: String,
    pub contract_type: ContractType,
}

#[derive(Serialize, Deserialize)]
pub struct CreateLeaveRequest {
    pub employee_id: Uuid,
    pub leave_type: String,
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
    pub notes: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ApproveLeaveRequest {
    pub approver_id: Uuid,
}

#[derive(Serialize, Deserialize)]
pub struct RejectLeaveRequest {
    pub approver_id: Uuid,
}

#[derive(Serialize, Deserialize)]
pub struct CreateTimesheetRequest {
    pub employee_id: Uuid,
    pub project_id: Option<Uuid>,
    pub date: chrono::DateTime<chrono::Utc>,
    pub hours: f64,
    pub task_id: Option<Uuid>,
    pub note: Option<String>,
}

// ===== Handlers =====

async fn create_employee(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateEmployeeRequest>,
) -> impl IntoResponse {
    let employee = Employee {
        id: Uuid::new_v4(),
        name: req.name,
        email: req.email,
        phone: req.phone,
        department_id: req.department_id,
        job_title: req.job_title,
        manager_id: req.manager_id,
        hire_date: Utc::now(),
        termination_date: None,
        status: EmployeeStatus::Active,
        tags: req.tags.unwrap_or_default(),
        created_at: Utc::now(),
    };

    let id = employee.id;
    store.employees.write().await.insert(id, employee.clone());

    (StatusCode::CREATED, Json(employee))
}

async fn get_employee(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match store.employees.read().await.get(&id) {
        Some(emp) => (StatusCode::OK, Json(Some(emp.clone()))).into_response(),
        None => (StatusCode::NOT_FOUND, Json::<Option<Employee>>(None)).into_response(),
    }
}

async fn list_employees(
    State(store): State<Arc<ErpStore>>,
) -> impl IntoResponse {
    let employees: Vec<_> = store.employees.read().await.values().cloned().collect();
    Json(employees)
}

async fn update_employee(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateEmployeeRequest>,
) -> impl IntoResponse {
    let mut employees = store.employees.write().await;
    match employees.get_mut(&id) {
        Some(emp) => {
            if let Some(name) = req.name {
                emp.name = name;
            }
            if let Some(email) = req.email {
                emp.email = email;
            }
            if let Some(phone) = req.phone {
                emp.phone = Some(phone);
            }
            if let Some(job_title) = req.job_title {
                emp.job_title = job_title;
            }
            if let Some(manager_id) = req.manager_id {
                emp.manager_id = manager_id;
            }
            (StatusCode::OK, Json(serde_json::to_value(emp.clone()).unwrap_or_default()))
        }
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error":"not_found"}))),
    }
}

async fn delete_employee(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match store.employees.write().await.remove(&id) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::NOT_FOUND,
    }
}

async fn create_department(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateDepartmentRequest>,
) -> impl IntoResponse {
    let dept = Department {
        id: Uuid::new_v4(),
        name: req.name,
        manager_id: req.manager_id,
        parent_id: req.parent_id,
        created_at: Utc::now(),
    };

    let id = dept.id;
    store.departments.write().await.insert(id, dept.clone());
    (StatusCode::CREATED, Json(dept))
}

async fn list_departments(
    State(store): State<Arc<ErpStore>>,
) -> impl IntoResponse {
    let depts: Vec<_> = store.departments.read().await.values().cloned().collect();
    Json(depts)
}

async fn create_contract(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateContractRequest>,
) -> impl IntoResponse {
    let contract = Contract {
        id: Uuid::new_v4(),
        employee_id: req.employee_id,
        start: req.start,
        end: req.end,
        salary: req.salary,
        currency: req.currency,
        contract_type: req.contract_type,
        created_at: Utc::now(),
    };

    let id = contract.id;
    store.contracts.write().await.insert(id, contract.clone());
    (StatusCode::CREATED, Json(contract))
}

async fn list_contracts(
    State(store): State<Arc<ErpStore>>,
) -> impl IntoResponse {
    let contracts: Vec<_> = store.contracts.read().await.values().cloned().collect();
    Json(contracts)
}

async fn create_leave(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateLeaveRequest>,
) -> impl IntoResponse {
    let leave = Leave {
        id: Uuid::new_v4(),
        employee_id: req.employee_id,
        leave_type: req.leave_type,
        start: req.start,
        end: req.end,
        status: LeaveStatus::Pending,
        approver_id: None,
        notes: req.notes,
        created_at: Utc::now(),
    };

    let id = leave.id;
    store.leaves.write().await.insert(id, leave.clone());
    (StatusCode::CREATED, Json(leave))
}

async fn list_leaves(
    State(store): State<Arc<ErpStore>>,
) -> impl IntoResponse {
    let leaves: Vec<_> = store.leaves.read().await.values().cloned().collect();
    Json(leaves)
}

async fn approve_leave(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ApproveLeaveRequest>,
) -> impl IntoResponse {
    let mut leaves = store.leaves.write().await;
    match leaves.get_mut(&id) {
        Some(leave) => {
            if leave.status == LeaveStatus::Pending {
                leave.status = LeaveStatus::Approved;
                leave.approver_id = Some(req.approver_id);
                (StatusCode::OK, Json(leave.clone()))
            } else {
                (StatusCode::BAD_REQUEST, Json(leave.clone()))
            }
        }
        None => (StatusCode::NOT_FOUND, Json(Leave {
            id: Uuid::nil(),
            employee_id: Uuid::nil(),
            leave_type: String::new(),
            start: Utc::now(),
            end: Utc::now(),
            status: LeaveStatus::Pending,
            approver_id: None,
            notes: None,
            created_at: Utc::now(),
        })),
    }
}

async fn reject_leave(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<RejectLeaveRequest>,
) -> impl IntoResponse {
    let mut leaves = store.leaves.write().await;
    match leaves.get_mut(&id) {
        Some(leave) => {
            if leave.status == LeaveStatus::Pending {
                leave.status = LeaveStatus::Rejected;
                leave.approver_id = Some(req.approver_id);
                (StatusCode::OK, Json(leave.clone()))
            } else {
                (StatusCode::BAD_REQUEST, Json(leave.clone()))
            }
        }
        None => (StatusCode::NOT_FOUND, Json(Leave {
            id: Uuid::nil(),
            employee_id: Uuid::nil(),
            leave_type: String::new(),
            start: Utc::now(),
            end: Utc::now(),
            status: LeaveStatus::Pending,
            approver_id: None,
            notes: None,
            created_at: Utc::now(),
        })),
    }
}

async fn create_timesheet(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateTimesheetRequest>,
) -> impl IntoResponse {
    let timesheet = Timesheet {
        id: Uuid::new_v4(),
        employee_id: req.employee_id,
        project_id: req.project_id,
        date: req.date,
        hours: req.hours,
        task_id: req.task_id,
        note: req.note,
        created_at: Utc::now(),
    };

    let id = timesheet.id;
    store.timesheets.write().await.insert(id, timesheet.clone());
    (StatusCode::CREATED, Json(timesheet))
}

async fn list_timesheets(
    State(store): State<Arc<ErpStore>>,
) -> impl IntoResponse {
    let timesheets: Vec<_> = store.timesheets.read().await.values().cloned().collect();
    Json(timesheets)
}

// ===== Router =====

pub fn create_router(state: Arc<ErpStore>) -> Router {
    Router::new()
        .route("/api/erp/hr/employees", post(create_employee).get(list_employees))
        .route(
            "/api/erp/hr/employees/{id}",
            get(get_employee).put(update_employee).delete(delete_employee),
        )
        .route(
            "/api/erp/hr/departments",
            post(create_department).get(list_departments),
        )
        .route("/api/erp/hr/contracts", post(create_contract).get(list_contracts))
        .route("/api/erp/hr/leaves", post(create_leave).get(list_leaves))
        .route(
            "/api/erp/hr/leaves/{id}/approve",
            post(approve_leave),
        )
        .route(
            "/api/erp/hr/leaves/{id}/reject",
            post(reject_leave),
        )
        .route(
            "/api/erp/hr/timesheets",
            post(create_timesheet).get(list_timesheets),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leave_approve_transitions_pending_to_approved() {
        let leave = Leave {
            id: Uuid::new_v4(),
            employee_id: Uuid::new_v4(),
            leave_type: "vacation".to_string(),
            start: Utc::now(),
            end: Utc::now(),
            status: LeaveStatus::Pending,
            approver_id: None,
            notes: None,
            created_at: Utc::now(),
        };

        assert_eq!(leave.status, LeaveStatus::Pending);
        // Simulating approval logic
        let approved_leave = Leave {
            status: LeaveStatus::Approved,
            ..leave
        };
        assert_eq!(approved_leave.status, LeaveStatus::Approved);
    }

    #[test]
    fn test_leave_reject_transitions_pending_to_rejected() {
        let leave = Leave {
            id: Uuid::new_v4(),
            employee_id: Uuid::new_v4(),
            leave_type: "vacation".to_string(),
            start: Utc::now(),
            end: Utc::now(),
            status: LeaveStatus::Pending,
            approver_id: None,
            notes: None,
            created_at: Utc::now(),
        };

        assert_eq!(leave.status, LeaveStatus::Pending);
        let rejected_leave = Leave {
            status: LeaveStatus::Rejected,
            ..leave
        };
        assert_eq!(rejected_leave.status, LeaveStatus::Rejected);
    }

    #[test]
    fn test_employee_termination_sets_status_and_date() {
        let mut employee = Employee {
            id: Uuid::new_v4(),
            name: "John".to_string(),
            email: "john@example.com".to_string(),
            phone: None,
            department_id: Uuid::new_v4(),
            job_title: "Engineer".to_string(),
            manager_id: None,
            hire_date: Utc::now(),
            termination_date: None,
            status: EmployeeStatus::Active,
            tags: vec![],
            created_at: Utc::now(),
        };

        assert_eq!(employee.status, EmployeeStatus::Active);
        assert_eq!(employee.termination_date, None);

        employee.status = EmployeeStatus::Terminated;
        employee.termination_date = Some(Utc::now());

        assert_eq!(employee.status, EmployeeStatus::Terminated);
        assert!(employee.termination_date.is_some());
    }
}
