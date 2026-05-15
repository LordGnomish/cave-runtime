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
pub struct CreateProjectRequest {
    pub name: String,
    pub code: String,
    pub customer_id: Option<Uuid>,
    pub manager_id: Uuid,
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: Option<chrono::DateTime<chrono::Utc>>,
    pub budget: f64,
    pub currency: String,
}

#[derive(Serialize, Deserialize)]
pub struct CreateTaskRequest {
    pub project_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub assignee_id: Option<Uuid>,
    pub priority: TaskPriority,
    pub estimated_hours: f64,
    pub deadline: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize, Deserialize)]
pub struct MoveTaskStateRequest {
    pub state: TaskState,
}

#[derive(Serialize, Deserialize)]
pub struct CreateMilestoneRequest {
    pub project_id: Uuid,
    pub name: String,
    pub due_date: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateTimeEntryRequest {
    pub project_id: Uuid,
    pub task_id: Option<Uuid>,
    pub user_id: Uuid,
    pub date: chrono::DateTime<chrono::Utc>,
    pub hours: f64,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ProjectSummary {
    pub total_hours: f64,
    pub spent_hours: f64,
    pub todo_count: usize,
    pub in_progress_count: usize,
    pub review_count: usize,
    pub done_count: usize,
    pub milestones_count: usize,
}

async fn create_project(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    let project = Project {
        id: Uuid::new_v4(),
        name: req.name,
        code: req.code,
        customer_id: req.customer_id,
        manager_id: req.manager_id,
        state: ProjectState::Active,
        start: req.start,
        end: req.end,
        budget: req.budget,
        currency: req.currency,
        created_at: Utc::now(),
    };
    let id = project.id;
    store.projects.write().await.insert(id, project.clone());
    (StatusCode::CREATED, Json(project))
}

async fn list_projects(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let projects: Vec<_> = store.projects.read().await.values().cloned().collect();
    Json(projects)
}

async fn complete_project(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut projects = store.projects.write().await;
    if let Some(proj) = projects.get_mut(&id) {
        proj.state = ProjectState::Completed;
        (StatusCode::OK, Json(proj.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Project {
            id: Uuid::nil(),
            name: String::new(),
            code: String::new(),
            customer_id: None,
            manager_id: Uuid::nil(),
            state: ProjectState::Active,
            start: Utc::now(),
            end: None,
            budget: 0.0,
            currency: String::new(),
            created_at: Utc::now(),
        }))
    }
}

async fn create_task(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    let task = Task {
        id: Uuid::new_v4(),
        project_id: req.project_id,
        title: req.title,
        description: req.description,
        assignee_id: req.assignee_id,
        priority: req.priority,
        state: TaskState::Todo,
        estimated_hours: req.estimated_hours,
        spent_hours: 0.0,
        deadline: req.deadline,
        created_at: Utc::now(),
    };
    let id = task.id;
    store.tasks.write().await.insert(id, task.clone());
    (StatusCode::CREATED, Json(task))
}

async fn list_tasks(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let tasks: Vec<_> = store.tasks.read().await.values().cloned().collect();
    Json(tasks)
}

async fn move_task_state(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<MoveTaskStateRequest>,
) -> impl IntoResponse {
    let mut tasks = store.tasks.write().await;
    if let Some(task) = tasks.get_mut(&id) {
        task.state = req.state;
        (StatusCode::OK, Json(task.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Task {
            id: Uuid::nil(),
            project_id: Uuid::nil(),
            title: String::new(),
            description: None,
            assignee_id: None,
            priority: TaskPriority::Medium,
            state: TaskState::Todo,
            estimated_hours: 0.0,
            spent_hours: 0.0,
            deadline: None,
            created_at: Utc::now(),
        }))
    }
}

async fn create_milestone(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateMilestoneRequest>,
) -> impl IntoResponse {
    let milestone = Milestone {
        id: Uuid::new_v4(),
        project_id: req.project_id,
        name: req.name,
        due_date: req.due_date,
        state: MilestoneState::Upcoming,
        created_at: Utc::now(),
    };
    let id = milestone.id;
    store.milestones.write().await.insert(id, milestone.clone());
    (StatusCode::CREATED, Json(milestone))
}

async fn list_milestones(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let milestones: Vec<_> = store.milestones.read().await.values().cloned().collect();
    Json(milestones)
}

async fn achieve_milestone(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut milestones = store.milestones.write().await;
    if let Some(ms) = milestones.get_mut(&id) {
        ms.state = MilestoneState::Achieved;
        (StatusCode::OK, Json(ms.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Milestone {
            id: Uuid::nil(),
            project_id: Uuid::nil(),
            name: String::new(),
            due_date: Utc::now(),
            state: MilestoneState::Upcoming,
            created_at: Utc::now(),
        }))
    }
}

async fn create_time_entry(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateTimeEntryRequest>,
) -> impl IntoResponse {
    let entry = TimeEntry {
        id: Uuid::new_v4(),
        project_id: req.project_id,
        task_id: req.task_id,
        user_id: req.user_id,
        date: req.date,
        hours: req.hours,
        description: req.description,
        created_at: Utc::now(),
    };

    let task_id = entry.task_id;
    let entry_hours = entry.hours;
    let entry_id = entry.id;

    store.time_entries.write().await.insert(entry_id, entry.clone());

    // Update task spent_hours if task_id exists
    if let Some(task_id) = task_id {
        let mut tasks = store.tasks.write().await;
        if let Some(task) = tasks.get_mut(&task_id) {
            task.spent_hours += entry_hours;
        }
    }

    (StatusCode::CREATED, Json(entry))
}

async fn list_time_entries(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let entries: Vec<_> = store.time_entries.read().await.values().cloned().collect();
    Json(entries)
}

async fn get_project_summary(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tasks = store.tasks.read().await;
    let project_tasks: Vec<_> = tasks
        .values()
        .filter(|t| t.project_id == id)
        .cloned()
        .collect();
    drop(tasks);

    let mut summary = ProjectSummary {
        total_hours: 0.0,
        spent_hours: 0.0,
        todo_count: 0,
        in_progress_count: 0,
        review_count: 0,
        done_count: 0,
        milestones_count: 0,
    };

    for task in &project_tasks {
        summary.total_hours += task.estimated_hours;
        summary.spent_hours += task.spent_hours;

        match task.state {
            TaskState::Todo => summary.todo_count += 1,
            TaskState::InProgress => summary.in_progress_count += 1,
            TaskState::Review => summary.review_count += 1,
            TaskState::Done => summary.done_count += 1,
        }
    }

    let milestones = store.milestones.read().await;
    summary.milestones_count = milestones
        .values()
        .filter(|m| m.project_id == id)
        .count();

    Json(summary)
}

pub fn create_router(state: Arc<ErpStore>) -> Router {
    Router::new()
        .route(
            "/api/erp/projects",
            post(create_project).get(list_projects),
        )
        .route("/api/erp/projects/{id}/complete", post(complete_project))
        .route("/api/erp/projects/{id}/summary", get(get_project_summary))
        .route("/api/erp/tasks", post(create_task).get(list_tasks))
        .route(
            "/api/erp/tasks/{id}/move-state",
            post(move_task_state),
        )
        .route(
            "/api/erp/milestones",
            post(create_milestone).get(list_milestones),
        )
        .route("/api/erp/milestones/{id}/achieve", post(achieve_milestone))
        .route(
            "/api/erp/time-entries",
            post(create_time_entry).get(list_time_entries),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_move_task_state_transitions() {
        let mut task = Task {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            title: "Implement feature".to_string(),
            description: None,
            assignee_id: None,
            priority: TaskPriority::High,
            state: TaskState::Todo,
            estimated_hours: 8.0,
            spent_hours: 0.0,
            deadline: None,
            created_at: Utc::now(),
        };

        assert_eq!(task.state, TaskState::Todo);
        task.state = TaskState::InProgress;
        assert_eq!(task.state, TaskState::InProgress);
        task.state = TaskState::Review;
        assert_eq!(task.state, TaskState::Review);
        task.state = TaskState::Done;
        assert_eq!(task.state, TaskState::Done);
    }

    #[test]
    fn test_achieve_milestone_sets_state() {
        let mut milestone = Milestone {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            name: "Beta Release".to_string(),
            due_date: Utc::now(),
            state: MilestoneState::Upcoming,
            created_at: Utc::now(),
        };

        assert_eq!(milestone.state, MilestoneState::Upcoming);
        milestone.state = MilestoneState::Achieved;
        assert_eq!(milestone.state, MilestoneState::Achieved);
    }

    #[test]
    fn test_project_summary_aggregates_correctly() {
        let project_id = Uuid::new_v4();

        let tasks = vec![
            Task {
                id: Uuid::new_v4(),
                project_id,
                title: "Task 1".to_string(),
                description: None,
                assignee_id: None,
                priority: TaskPriority::Medium,
                state: TaskState::Done,
                estimated_hours: 10.0,
                spent_hours: 10.0,
                deadline: None,
                created_at: Utc::now(),
            },
            Task {
                id: Uuid::new_v4(),
                project_id,
                title: "Task 2".to_string(),
                description: None,
                assignee_id: None,
                priority: TaskPriority::Medium,
                state: TaskState::InProgress,
                estimated_hours: 5.0,
                spent_hours: 3.0,
                deadline: None,
                created_at: Utc::now(),
            },
        ];

        let total_est: f64 = tasks.iter().map(|t| t.estimated_hours).sum();
        let total_spent: f64 = tasks.iter().map(|t| t.spent_hours).sum();
        let done_count = tasks.iter().filter(|t| t.state == TaskState::Done).count();

        assert!((total_est - 15.0).abs() < 0.01);
        assert!((total_spent - 13.0).abs() < 0.01);
        assert_eq!(done_count, 1);
    }
}
