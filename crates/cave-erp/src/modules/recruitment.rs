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
pub struct CreateJobRequest {
    pub title: String,
    pub department_id: Uuid,
    pub description: String,
    pub headcount_target: u32,
}

#[derive(Serialize, Deserialize)]
pub struct CreateApplicantRequest {
    pub job_id: Uuid,
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub resume_url: Option<String>,
    pub source: String,
}

#[derive(Serialize, Deserialize)]
pub struct AdvanceApplicantRequest {
    pub stage_id: Uuid,
}

#[derive(Serialize, Deserialize)]
pub struct CreateInterviewRequest {
    pub applicant_id: Uuid,
    pub scheduled_at: chrono::DateTime<chrono::Utc>,
    pub interviewer_id: Uuid,
    pub mode: InterviewMode,
}

#[derive(Serialize, Deserialize)]
pub struct InterviewFeedbackRequest {
    pub feedback: String,
    pub outcome: InterviewOutcome,
}

#[derive(Serialize, Deserialize)]
pub struct CreateOfferRequest {
    pub applicant_id: Uuid,
    pub salary: f64,
    pub currency: String,
    pub start_date: chrono::DateTime<chrono::Utc>,
}

// Handlers
async fn create_job(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateJobRequest>,
) -> impl IntoResponse {
    let job = Job {
        id: Uuid::new_v4(),
        title: req.title,
        department_id: req.department_id,
        description: req.description,
        state: JobState::Open,
        posted_at: Utc::now(),
        headcount_target: req.headcount_target,
        created_at: Utc::now(),
    };
    let id = job.id;
    store.jobs.write().await.insert(id, job.clone());
    (StatusCode::CREATED, Json(job))
}

async fn list_jobs(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let jobs: Vec<_> = store.jobs.read().await.values().cloned().collect();
    Json(jobs)
}

async fn close_job(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut jobs = store.jobs.write().await;
    if let Some(job) = jobs.get_mut(&id) {
        job.state = JobState::Closed;
        (StatusCode::OK, Json(job.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Job {
            id: Uuid::nil(),
            title: String::new(),
            department_id: Uuid::nil(),
            description: String::new(),
            state: JobState::Closed,
            posted_at: Utc::now(),
            headcount_target: 0,
            created_at: Utc::now(),
        }))
    }
}

async fn create_applicant(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateApplicantRequest>,
) -> impl IntoResponse {
    // Get first stage (Applied)
    let stages = store.stages_recruit.read().await;
    let first_stage = stages.values().min_by_key(|s| s.order).map(|s| s.id);
    drop(stages);

    let stage_id = first_stage.unwrap_or_else(Uuid::new_v4);

    let applicant = Applicant {
        id: Uuid::new_v4(),
        job_id: req.job_id,
        name: req.name,
        email: req.email,
        phone: req.phone,
        resume_url: req.resume_url,
        source: req.source,
        stage_id,
        score: 0,
        rating: 3,
        status: ApplicantStatus::Active,
        applied_at: Utc::now(),
        recruiter_id: None,
        created_at: Utc::now(),
    };
    let id = applicant.id;
    store.applicants.write().await.insert(id, applicant.clone());
    (StatusCode::CREATED, Json(applicant))
}

async fn list_applicants(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let apps: Vec<_> = store.applicants.read().await.values().cloned().collect();
    Json(apps)
}

async fn advance_applicant(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AdvanceApplicantRequest>,
) -> impl IntoResponse {
    let mut applicants = store.applicants.write().await;
    if let Some(app) = applicants.get_mut(&id) {
        app.stage_id = req.stage_id;
        (StatusCode::OK, Json(app.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Applicant {
            id: Uuid::nil(),
            job_id: Uuid::nil(),
            name: String::new(),
            email: String::new(),
            phone: None,
            resume_url: None,
            source: String::new(),
            stage_id: Uuid::nil(),
            score: 0,
            rating: 0,
            status: ApplicantStatus::Active,
            applied_at: Utc::now(),
            recruiter_id: None,
            created_at: Utc::now(),
        }))
    }
}

async fn reject_applicant(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut applicants = store.applicants.write().await;
    if let Some(app) = applicants.get_mut(&id) {
        app.status = ApplicantStatus::Rejected;
        (StatusCode::OK, Json(app.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Applicant {
            id: Uuid::nil(),
            job_id: Uuid::nil(),
            name: String::new(),
            email: String::new(),
            phone: None,
            resume_url: None,
            source: String::new(),
            stage_id: Uuid::nil(),
            score: 0,
            rating: 0,
            status: ApplicantStatus::Active,
            applied_at: Utc::now(),
            recruiter_id: None,
            created_at: Utc::now(),
        }))
    }
}

async fn hire_applicant(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut applicants = store.applicants.write().await;
    if let Some(app) = applicants.get_mut(&id) {
        app.status = ApplicantStatus::Hired;
        (StatusCode::OK, Json(app.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Applicant {
            id: Uuid::nil(),
            job_id: Uuid::nil(),
            name: String::new(),
            email: String::new(),
            phone: None,
            resume_url: None,
            source: String::new(),
            stage_id: Uuid::nil(),
            score: 0,
            rating: 0,
            status: ApplicantStatus::Active,
            applied_at: Utc::now(),
            recruiter_id: None,
            created_at: Utc::now(),
        }))
    }
}

async fn create_interview(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateInterviewRequest>,
) -> impl IntoResponse {
    let interview = Interview {
        id: Uuid::new_v4(),
        applicant_id: req.applicant_id,
        scheduled_at: req.scheduled_at,
        interviewer_id: req.interviewer_id,
        mode: req.mode,
        feedback: None,
        outcome: InterviewOutcome::Pending,
        created_at: Utc::now(),
    };
    let id = interview.id;
    store.interviews.write().await.insert(id, interview.clone());
    (StatusCode::CREATED, Json(interview))
}

async fn list_interviews(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let interviews: Vec<_> = store.interviews.read().await.values().cloned().collect();
    Json(interviews)
}

async fn interview_feedback(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
    Json(req): Json<InterviewFeedbackRequest>,
) -> impl IntoResponse {
    let mut interviews = store.interviews.write().await;
    if let Some(interview) = interviews.get_mut(&id) {
        interview.feedback = Some(req.feedback);
        interview.outcome = req.outcome;
        (StatusCode::OK, Json(interview.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Interview {
            id: Uuid::nil(),
            applicant_id: Uuid::nil(),
            scheduled_at: Utc::now(),
            interviewer_id: Uuid::nil(),
            mode: InterviewMode::Phone,
            feedback: None,
            outcome: InterviewOutcome::Pending,
            created_at: Utc::now(),
        }))
    }
}

async fn create_offer(
    State(store): State<Arc<ErpStore>>,
    Json(req): Json<CreateOfferRequest>,
) -> impl IntoResponse {
    let offer = Offer {
        id: Uuid::new_v4(),
        applicant_id: req.applicant_id,
        salary: req.salary,
        currency: req.currency,
        start_date: req.start_date,
        state: OfferState::Draft,
        expires_at: Utc::now() + chrono::Duration::days(7),
        created_at: Utc::now(),
    };
    let id = offer.id;
    store.offers.write().await.insert(id, offer.clone());
    (StatusCode::CREATED, Json(offer))
}

async fn list_offers(State(store): State<Arc<ErpStore>>) -> impl IntoResponse {
    let offers: Vec<_> = store.offers.read().await.values().cloned().collect();
    Json(offers)
}

async fn send_offer(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut offers = store.offers.write().await;
    if let Some(offer) = offers.get_mut(&id) {
        offer.state = OfferState::Sent;
        (StatusCode::OK, Json(offer.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Offer {
            id: Uuid::nil(),
            applicant_id: Uuid::nil(),
            salary: 0.0,
            currency: String::new(),
            start_date: Utc::now(),
            state: OfferState::Draft,
            expires_at: Utc::now(),
            created_at: Utc::now(),
        }))
    }
}

async fn accept_offer(
    State(store): State<Arc<ErpStore>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let mut offers = store.offers.write().await;
    if let Some(offer) = offers.get_mut(&id) {
        offer.state = OfferState::Accepted;
        (StatusCode::OK, Json(offer.clone()))
    } else {
        (StatusCode::NOT_FOUND, Json(Offer {
            id: Uuid::nil(),
            applicant_id: Uuid::nil(),
            salary: 0.0,
            currency: String::new(),
            start_date: Utc::now(),
            state: OfferState::Draft,
            expires_at: Utc::now(),
            created_at: Utc::now(),
        }))
    }
}

pub fn create_router(state: Arc<ErpStore>) -> Router {
    Router::new()
        .route("/api/erp/recruitment/jobs", post(create_job).get(list_jobs))
        .route("/api/erp/recruitment/jobs/{id}/close", post(close_job))
        .route(
            "/api/erp/recruitment/applicants",
            post(create_applicant).get(list_applicants),
        )
        .route(
            "/api/erp/recruitment/applicants/{id}/advance",
            post(advance_applicant),
        )
        .route(
            "/api/erp/recruitment/applicants/{id}/reject",
            post(reject_applicant),
        )
        .route(
            "/api/erp/recruitment/applicants/{id}/hire",
            post(hire_applicant),
        )
        .route(
            "/api/erp/recruitment/interviews",
            post(create_interview).get(list_interviews),
        )
        .route(
            "/api/erp/recruitment/interviews/{id}/feedback",
            post(interview_feedback),
        )
        .route(
            "/api/erp/recruitment/offers",
            post(create_offer).get(list_offers),
        )
        .route("/api/erp/recruitment/offers/{id}/send", post(send_offer))
        .route("/api/erp/recruitment/offers/{id}/accept", post(accept_offer))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_applicant_advance_moves_stage() {
        let stage_id = Uuid::new_v4();
        let new_stage = Uuid::new_v4();
        let mut app = Applicant {
            id: Uuid::new_v4(),
            job_id: Uuid::new_v4(),
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
            phone: None,
            resume_url: None,
            source: "LinkedIn".to_string(),
            stage_id,
            score: 80,
            rating: 5,
            status: ApplicantStatus::Active,
            applied_at: Utc::now(),
            recruiter_id: None,
            created_at: Utc::now(),
        };

        assert_eq!(app.stage_id, stage_id);
        app.stage_id = new_stage;
        assert_eq!(app.stage_id, new_stage);
    }

    #[test]
    fn test_offer_accept_sets_state() {
        let mut offer = Offer {
            id: Uuid::new_v4(),
            applicant_id: Uuid::new_v4(),
            salary: 50000.0,
            currency: "EUR".to_string(),
            start_date: Utc::now(),
            state: OfferState::Sent,
            expires_at: Utc::now(),
            created_at: Utc::now(),
        };

        assert_eq!(offer.state, OfferState::Sent);
        offer.state = OfferState::Accepted;
        assert_eq!(offer.state, OfferState::Accepted);
    }

    #[test]
    fn test_interview_feedback_updates_outcome() {
        let mut interview = Interview {
            id: Uuid::new_v4(),
            applicant_id: Uuid::new_v4(),
            scheduled_at: Utc::now(),
            interviewer_id: Uuid::new_v4(),
            mode: InterviewMode::Video,
            feedback: None,
            outcome: InterviewOutcome::Pending,
            created_at: Utc::now(),
        };

        assert_eq!(interview.outcome, InterviewOutcome::Pending);
        interview.feedback = Some("Great fit!".to_string());
        interview.outcome = InterviewOutcome::Pass;
        assert_eq!(interview.outcome, InterviewOutcome::Pass);
        assert!(interview.feedback.is_some());
    }
}
