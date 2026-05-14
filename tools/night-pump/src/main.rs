// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

use axum::{
    extract::{Json as ExtractJson, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Parser, Debug, Clone)]
#[command(about = "cave-night-pump — coordinates worker batches against a YAML queue")]
struct Args {
    #[arg(long, default_value = "queue.yaml")]
    queue: PathBuf,
    #[arg(long, default_value = "state.json")]
    state: PathBuf,
    #[arg(long, default_value = "contributions.jsonl")]
    contributions: PathBuf,
    #[arg(long, default_value = "log/heartbeat.log")]
    heartbeat_log: PathBuf,
    #[arg(long, default_value_t = 9090)]
    port: u16,
    #[arg(long, default_value_t = 8)]
    max_parallel: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Batch {
    pub id: String,
    pub scope: String,
    pub target_tests: u32,
    pub upstream: String,
    #[serde(default)]
    pub pin_check: bool,
    #[serde(default)]
    pub dependency: Vec<String>,
    pub priority: u32,
    #[serde(default = "default_retry_max")]
    pub retry_max: u32,
}

fn default_retry_max() -> u32 {
    3
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Queue {
    #[serde(default)]
    pub backend_v01: Vec<Batch>,
    #[serde(default)]
    pub ux_v01: Vec<Batch>,
    #[serde(default)]
    pub obs_v01: Vec<Batch>,
    #[serde(default)]
    pub adr_review_runtime_overrides: Vec<Batch>,
}

impl Queue {
    pub fn all(&self) -> impl Iterator<Item = &Batch> {
        self.backend_v01
            .iter()
            .chain(self.ux_v01.iter())
            .chain(self.obs_v01.iter())
            .chain(self.adr_review_runtime_overrides.iter())
    }

    pub fn count(&self) -> usize {
        self.backend_v01.len()
            + self.ux_v01.len()
            + self.obs_v01.len()
            + self.adr_review_runtime_overrides.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Queued,
    Dispatched,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchState {
    pub status: BatchStatus,
    pub dispatched_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub retry_count: u32,
    pub worker_id: Option<String>,
    pub last_error: Option<String>,
}

impl Default for BatchState {
    fn default() -> Self {
        Self {
            status: BatchStatus::Queued,
            dispatched_at: None,
            completed_at: None,
            retry_count: 0,
            worker_id: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateFile {
    #[serde(default)]
    pub batches: HashMap<String, BatchState>,
    #[serde(default)]
    pub last_heartbeat: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contribution {
    pub worker_id: String,
    pub batch_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub status: String,
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub test_delta: i32,
    #[serde(default)]
    pub lines_added: u32,
    #[serde(default)]
    pub lines_removed: u32,
    pub error: Option<String>,
}

pub struct AppState {
    pub queue: Queue,
    pub state: RwLock<StateFile>,
    pub state_path: PathBuf,
    pub contributions_path: PathBuf,
    pub max_parallel: usize,
    pub dispatch_paused: RwLock<bool>,
}

impl AppState {
    pub fn new(
        queue: Queue,
        state: StateFile,
        state_path: PathBuf,
        contributions_path: PathBuf,
        max_parallel: usize,
    ) -> Self {
        Self {
            queue,
            state: RwLock::new(state),
            state_path,
            contributions_path,
            max_parallel,
            dispatch_paused: RwLock::new(false),
        }
    }
}

pub async fn load_queue(path: &PathBuf) -> anyhow::Result<Queue> {
    let bytes = fs::read(path).await?;
    let q: Queue = serde_yaml::from_slice(&bytes)?;
    Ok(q)
}

pub async fn load_state(path: &PathBuf) -> anyhow::Result<StateFile> {
    if !path.exists() {
        return Ok(StateFile::default());
    }
    let bytes = fs::read(path).await?;
    if bytes.is_empty() {
        return Ok(StateFile::default());
    }
    let s: StateFile = serde_json::from_slice(&bytes)?;
    Ok(s)
}

pub async fn persist_state(path: &PathBuf, state: &StateFile) -> anyhow::Result<()> {
    let json = serde_json::to_vec_pretty(state)?;
    let tmp = path.with_extension("json.tmp");
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).await.ok();
        }
    }
    fs::write(&tmp, &json).await?;
    fs::rename(&tmp, path).await?;
    Ok(())
}

pub async fn append_contribution(path: &PathBuf, c: &Contribution) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).await.ok();
        }
    }
    let line = serde_json::to_string(c)? + "\n";
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    f.write_all(line.as_bytes()).await?;
    Ok(())
}

pub fn deps_satisfied(batch: &Batch, state: &StateFile) -> bool {
    batch.dependency.iter().all(|dep| {
        state
            .batches
            .get(dep)
            .map(|s| s.status == BatchStatus::Completed)
            .unwrap_or(false)
    })
}

pub fn count_in_flight(state: &StateFile) -> usize {
    state
        .batches
        .values()
        .filter(|s| matches!(s.status, BatchStatus::Dispatched | BatchStatus::InProgress))
        .count()
}

pub fn is_dispatchable(batch: &Batch, state: &StateFile) -> bool {
    let status_ok = match state.batches.get(&batch.id) {
        None => true,
        Some(s) => match s.status {
            BatchStatus::Queued => true,
            BatchStatus::Failed => s.retry_count < batch.retry_max,
            _ => false,
        },
    };
    status_ok && deps_satisfied(batch, state)
}

#[derive(Deserialize)]
pub struct NextBatchQuery {
    pub worker_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct HeartbeatStats {
    pub queued: usize,
    pub dispatched: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub failed: usize,
    pub total: usize,
    pub max_parallel: usize,
    pub dispatch_paused: bool,
    pub last_heartbeat: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct BatchCompleteRequest {
    pub worker_id: String,
    pub batch_id: String,
    pub status: String,
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub test_delta: i32,
    #[serde(default)]
    pub lines_added: u32,
    #[serde(default)]
    pub lines_removed: u32,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct ContributionsQuery {
    pub since: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContribAggregate {
    pub batches: u32,
    pub completed: u32,
    pub failed: u32,
    pub test_delta: i32,
    pub lines_added: u32,
    pub lines_removed: u32,
}

pub async fn next_batch_handler(
    State(app): State<Arc<AppState>>,
    Query(q): Query<NextBatchQuery>,
) -> Result<Json<Option<Batch>>, (StatusCode, String)> {
    if *app.dispatch_paused.read().await {
        return Ok(Json(None));
    }
    let mut state = app.state.write().await;
    if count_in_flight(&state) >= app.max_parallel {
        return Ok(Json(None));
    }
    let mut candidates: Vec<&Batch> = app.queue.all().filter(|b| is_dispatchable(b, &state)).collect();
    candidates.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
    if let Some(picked) = candidates.first() {
        let pid = picked.id.clone();
        let pclone = (*picked).clone();
        let entry = state.batches.entry(pid).or_default();
        entry.status = BatchStatus::Dispatched;
        entry.dispatched_at = Some(Utc::now());
        entry.worker_id = Some(q.worker_id.clone());
        entry.last_error = None;
        let snapshot = state.clone();
        drop(state);
        persist_state(&app.state_path, &snapshot)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok(Json(Some(pclone)))
    } else {
        Ok(Json(None))
    }
}

pub async fn batch_complete_handler(
    State(app): State<Arc<AppState>>,
    ExtractJson(req): ExtractJson<BatchCompleteRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let now = Utc::now();
    let started_at;
    {
        let mut state = app.state.write().await;
        let entry = state.batches.entry(req.batch_id.clone()).or_default();
        started_at = entry.dispatched_at.unwrap_or(now);
        match req.status.as_str() {
            "completed" => {
                entry.status = BatchStatus::Completed;
                entry.completed_at = Some(now);
                entry.last_error = None;
            }
            "failed" => {
                entry.status = BatchStatus::Failed;
                entry.completed_at = Some(now);
                entry.retry_count = entry.retry_count.saturating_add(1);
                entry.last_error = req.error.clone();
            }
            other => {
                return Err((StatusCode::BAD_REQUEST, format!("invalid status: {}", other)));
            }
        }
        let snapshot = state.clone();
        drop(state);
        persist_state(&app.state_path, &snapshot)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    let contrib = Contribution {
        worker_id: req.worker_id.clone(),
        batch_id: req.batch_id.clone(),
        started_at,
        completed_at: now,
        status: req.status.clone(),
        commit_sha: req.commit_sha.clone(),
        test_delta: req.test_delta,
        lines_added: req.lines_added,
        lines_removed: req.lines_removed,
        error: req.error.clone(),
    };
    append_contribution(&app.contributions_path, &contrib)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({"ok": true, "batch_id": req.batch_id, "status": req.status})))
}

pub async fn heartbeat_handler(State(app): State<Arc<AppState>>) -> Json<HeartbeatStats> {
    let state = app.state.read().await;
    let paused = *app.dispatch_paused.read().await;
    let mut stats = HeartbeatStats {
        queued: 0,
        dispatched: 0,
        in_progress: 0,
        completed: 0,
        failed: 0,
        total: app.queue.count(),
        max_parallel: app.max_parallel,
        dispatch_paused: paused,
        last_heartbeat: state.last_heartbeat,
    };
    for b in app.queue.all() {
        match state.batches.get(&b.id) {
            None => stats.queued += 1,
            Some(s) => match s.status {
                BatchStatus::Queued => stats.queued += 1,
                BatchStatus::Dispatched => stats.dispatched += 1,
                BatchStatus::InProgress => stats.in_progress += 1,
                BatchStatus::Completed => stats.completed += 1,
                BatchStatus::Failed => stats.failed += 1,
            },
        }
    }
    Json(stats)
}

pub async fn contributions_handler(
    State(app): State<Arc<AppState>>,
    Query(q): Query<ContributionsQuery>,
) -> Result<Json<HashMap<String, ContribAggregate>>, (StatusCode, String)> {
    let raw = match fs::read_to_string(&app.contributions_path).await {
        Ok(s) => s,
        Err(_) => String::new(),
    };
    let since: Option<DateTime<Utc>> = q
        .since
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc)));
    let mut agg: HashMap<String, ContribAggregate> = HashMap::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let c: Contribution = match serde_json::from_str(line) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if let Some(ref since_d) = since {
            if c.completed_at < *since_d {
                continue;
            }
        }
        let entry = agg.entry(c.worker_id.clone()).or_default();
        entry.batches += 1;
        match c.status.as_str() {
            "completed" => entry.completed += 1,
            "failed" => entry.failed += 1,
            _ => {}
        }
        entry.test_delta += c.test_delta;
        entry.lines_added += c.lines_added;
        entry.lines_removed += c.lines_removed;
    }
    Ok(Json(agg))
}

pub fn build_router(app: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/next-batch", get(next_batch_handler))
        .route("/api/batch-complete", post(batch_complete_handler))
        .route("/api/heartbeat", get(heartbeat_handler))
        .route("/api/contributions", get(contributions_handler))
        .with_state(app)
}

async fn tick_task(app: Arc<AppState>, heartbeat_log: PathBuf) {
    use sysinfo::{Disks, System};
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.tick().await; // discard immediate first tick
    loop {
        interval.tick().await;
        let now = Utc::now();
        let mut sys = System::new();
        sys.refresh_memory();
        let avail_gb = sys.available_memory() as f64 / 1e9;
        let disks = Disks::new_with_refreshed_list();
        let disk_free_gb: u64 = disks
            .iter()
            .map(|d| d.available_space() / 1_000_000_000)
            .min()
            .unwrap_or(u64::MAX);

        let snapshot = {
            let mut state = app.state.write().await;
            state.last_heartbeat = Some(now);
            state.clone()
        };
        if let Err(e) = persist_state(&app.state_path, &snapshot).await {
            warn!("persist_state failed: {}", e);
        }
        let completed = snapshot
            .batches
            .values()
            .filter(|s| s.status == BatchStatus::Completed)
            .count();
        let in_flight = count_in_flight(&snapshot);

        if let Some(parent) = heartbeat_log.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).await.ok();
            }
        }
        let line = format!(
            "{} disk_free_gb={} mem_avail_gb={:.1} completed={} in_flight={}\n",
            now.to_rfc3339(),
            disk_free_gb,
            avail_gb,
            completed,
            in_flight
        );
        if let Ok(mut f) = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&heartbeat_log)
            .await
        {
            let _ = f.write_all(line.as_bytes()).await;
        }

        let pause = disk_free_gb < 30 || avail_gb < 4.0;
        if disk_free_gb < 30 {
            warn!("disk free {}GB < 30GB", disk_free_gb);
        }
        if avail_gb < 4.0 {
            warn!("memory available {:.1}GB < 4GB", avail_gb);
        }
        let mut p = app.dispatch_paused.write().await;
        if *p != pause {
            info!("dispatch_paused: {} → {}", *p, pause);
        }
        *p = pause;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();
    info!("loading queue: {}", args.queue.display());
    let queue = load_queue(&args.queue).await?;
    info!("queue: {} batches", queue.count());
    let state = load_state(&args.state).await?;
    info!("state: {} known batches", state.batches.len());
    let app = Arc::new(AppState::new(
        queue,
        state,
        args.state.clone(),
        args.contributions.clone(),
        args.max_parallel,
    ));
    let router = build_router(Arc::clone(&app));
    let tick_app = Arc::clone(&app);
    let tick_log = args.heartbeat_log.clone();
    tokio::spawn(async move {
        tick_task(tick_app, tick_log).await;
    });
    let addr = format!("0.0.0.0:{}", args.port);
    info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn fixture_queue() -> Queue {
        let mut q = Queue::default();
        q.backend_v01.push(Batch {
            id: "a".into(),
            scope: "backend".into(),
            target_tests: 10,
            upstream: "x".into(),
            pin_check: true,
            dependency: vec![],
            priority: 9,
            retry_max: 3,
        });
        q.backend_v01.push(Batch {
            id: "b".into(),
            scope: "backend".into(),
            target_tests: 10,
            upstream: "y".into(),
            pin_check: true,
            dependency: vec!["a".into()],
            priority: 8,
            retry_max: 3,
        });
        q.ux_v01.push(Batch {
            id: "c".into(),
            scope: "ux".into(),
            target_tests: 5,
            upstream: "z".into(),
            pin_check: false,
            dependency: vec![],
            priority: 4,
            retry_max: 3,
        });
        q
    }

    fn temp_paths() -> (PathBuf, PathBuf) {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let pid = std::process::id();
        // Add a static counter to disambiguate when tests run in parallel within
        // the same nanosecond.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("nightpump-test-{}-{}-{}", nanos, pid, n));
        std::fs::create_dir_all(&dir).unwrap();
        (dir.join("state.json"), dir.join("contrib.jsonl"))
    }

    async fn read_body_to_vec(body: Body) -> Vec<u8> {
        body.collect().await.unwrap().to_bytes().to_vec()
    }

    #[test]
    fn batch_state_default() {
        let s = BatchState::default();
        assert_eq!(s.status, BatchStatus::Queued);
        assert_eq!(s.retry_count, 0);
    }

    #[test]
    fn deps_satisfied_works() {
        let mut state = StateFile::default();
        let b = Batch {
            id: "x".into(),
            scope: "s".into(),
            target_tests: 0,
            upstream: "u".into(),
            pin_check: false,
            dependency: vec!["d1".into()],
            priority: 1,
            retry_max: 3,
        };
        assert!(!deps_satisfied(&b, &state));
        state.batches.insert(
            "d1".into(),
            BatchState {
                status: BatchStatus::Completed,
                ..Default::default()
            },
        );
        assert!(deps_satisfied(&b, &state));
    }

    #[test]
    fn is_dispatchable_state_machine() {
        let b = Batch {
            id: "x".into(),
            scope: "s".into(),
            target_tests: 0,
            upstream: "u".into(),
            pin_check: false,
            dependency: vec![],
            priority: 1,
            retry_max: 3,
        };
        let mut state = StateFile::default();
        assert!(is_dispatchable(&b, &state)); // unknown → queued, dispatchable

        state.batches.insert(
            "x".into(),
            BatchState {
                status: BatchStatus::Dispatched,
                ..Default::default()
            },
        );
        assert!(!is_dispatchable(&b, &state)); // dispatched → not again

        state.batches.insert(
            "x".into(),
            BatchState {
                status: BatchStatus::Completed,
                ..Default::default()
            },
        );
        assert!(!is_dispatchable(&b, &state)); // completed → done

        state.batches.insert(
            "x".into(),
            BatchState {
                status: BatchStatus::Failed,
                retry_count: 1,
                ..Default::default()
            },
        );
        assert!(is_dispatchable(&b, &state)); // failed under cap → retryable

        state.batches.insert(
            "x".into(),
            BatchState {
                status: BatchStatus::Failed,
                retry_count: 3,
                ..Default::default()
            },
        );
        assert!(!is_dispatchable(&b, &state)); // failed at max → done
    }

    #[tokio::test]
    async fn http_next_batch_returns_highest_priority() {
        let q = fixture_queue();
        let (sp, cp) = temp_paths();
        let app = Arc::new(AppState::new(q, StateFile::default(), sp, cp, 8));
        let router = build_router(Arc::clone(&app));
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/next-batch?worker_id=test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = read_body_to_vec(resp.into_body()).await;
        let v: Option<Batch> = serde_json::from_slice(&body).unwrap();
        assert_eq!(v.expect("expected a batch").id, "a"); // priority 9
    }

    #[tokio::test]
    async fn http_dispatch_respects_dependency() {
        let q = fixture_queue();
        let (sp, cp) = temp_paths();
        let app = Arc::new(AppState::new(q, StateFile::default(), sp, cp, 8));
        let router = build_router(Arc::clone(&app));

        let r1 = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/next-batch?worker_id=w1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v1: Option<Batch> = serde_json::from_slice(&read_body_to_vec(r1.into_body()).await).unwrap();
        assert_eq!(v1.unwrap().id, "a");

        // Next call: 'b' depends on 'a' (still dispatched), so must skip to 'c'.
        let r2 = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/next-batch?worker_id=w2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v2: Option<Batch> = serde_json::from_slice(&read_body_to_vec(r2.into_body()).await).unwrap();
        assert_eq!(v2.unwrap().id, "c");
    }

    #[tokio::test]
    async fn http_concurrency_cap() {
        let q = fixture_queue();
        let (sp, cp) = temp_paths();
        let app = Arc::new(AppState::new(q, StateFile::default(), sp, cp, 1)); // cap=1
        let router = build_router(Arc::clone(&app));
        let r1 = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/next-batch?worker_id=w1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v1: Option<Batch> = serde_json::from_slice(&read_body_to_vec(r1.into_body()).await).unwrap();
        assert!(v1.is_some());
        // cap reached → null
        let r2 = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/next-batch?worker_id=w2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v2: Option<Batch> = serde_json::from_slice(&read_body_to_vec(r2.into_body()).await).unwrap();
        assert!(v2.is_none());
    }

    #[tokio::test]
    async fn http_batch_complete_advances_state_and_unblocks_dependents() {
        let q = fixture_queue();
        let (sp, cp) = temp_paths();
        let app = Arc::new(AppState::new(q, StateFile::default(), sp, cp, 8));
        let router = build_router(Arc::clone(&app));

        // dispatch 'a'
        let _ = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/next-batch?worker_id=w1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // complete 'a'
        let body = serde_json::json!({
            "worker_id": "w1",
            "batch_id":  "a",
            "status":    "completed",
            "commit_sha":    "abcdef",
            "test_delta":    50,
            "lines_added":   200,
            "lines_removed": 10,
        });
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/batch-complete")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // 'b' should now be dispatchable (its dep 'a' is Completed).
        let r = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/next-batch?worker_id=w2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v: Option<Batch> = serde_json::from_slice(&read_body_to_vec(r.into_body()).await).unwrap();
        assert_eq!(v.unwrap().id, "b");
    }

    #[tokio::test]
    async fn http_batch_complete_failed_increments_retry() {
        let q = fixture_queue();
        let (sp, cp) = temp_paths();
        let app = Arc::new(AppState::new(q, StateFile::default(), sp.clone(), cp, 8));
        let router = build_router(Arc::clone(&app));

        // dispatch 'a', fail it
        let _ = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/next-batch?worker_id=w1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = serde_json::json!({
            "worker_id": "w1", "batch_id": "a", "status": "failed",
            "error": "boom"
        });
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/batch-complete")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        // verify in-memory state
        let st = app.state.read().await;
        let s = st.batches.get("a").unwrap();
        assert_eq!(s.status, BatchStatus::Failed);
        assert_eq!(s.retry_count, 1);
        assert_eq!(s.last_error.as_deref(), Some("boom"));
        drop(st);

        // 'a' should be dispatchable again (under retry cap)
        let r = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/next-batch?worker_id=w2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v: Option<Batch> = serde_json::from_slice(&read_body_to_vec(r.into_body()).await).unwrap();
        assert_eq!(v.unwrap().id, "a");
    }

    #[tokio::test]
    async fn http_heartbeat_returns_stats() {
        let q = fixture_queue();
        let (sp, cp) = temp_paths();
        let app = Arc::new(AppState::new(q, StateFile::default(), sp, cp, 8));
        let router = build_router(Arc::clone(&app));
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/heartbeat")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let stats: HeartbeatStats =
            serde_json::from_slice(&read_body_to_vec(resp.into_body()).await).unwrap();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.queued, 3);
        assert_eq!(stats.completed, 0);
    }

    #[tokio::test]
    async fn http_contributions_aggregates_by_worker() {
        let q = fixture_queue();
        let (sp, cp) = temp_paths();
        let app = Arc::new(AppState::new(q, StateFile::default(), sp, cp, 8));
        let router = build_router(Arc::clone(&app));

        // dispatch + complete 'a' twice (failed then completed)
        for (i, status) in [(1, "failed"), (2, "completed")] {
            let _ = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/next-batch?worker_id=w{}", i))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = serde_json::json!({
                "worker_id": format!("w{}", i),
                "batch_id":  "a",
                "status":    status,
                "lines_added":  100u32,
                "test_delta":   10i32,
            });
            let _ = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/batch-complete")
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
        }

        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/contributions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let agg: HashMap<String, ContribAggregate> =
            serde_json::from_slice(&read_body_to_vec(resp.into_body()).await).unwrap();
        let w1 = agg.get("w1").expect("w1 contribution");
        let w2 = agg.get("w2").expect("w2 contribution");
        assert_eq!(w1.failed, 1);
        assert_eq!(w2.completed, 1);
        assert_eq!(w1.lines_added, 100);
        assert_eq!(w2.test_delta, 10);
    }

    #[test]
    fn shipped_queue_yaml_parses_with_at_least_40_batches() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("queue.yaml");
        let bytes = std::fs::read(&path).expect("queue.yaml missing");
        let q: Queue = serde_yaml::from_slice(&bytes).expect("queue.yaml failed to parse");
        let total = q.count();
        assert!(total >= 40, "expected ≥40 batches, got {}", total);
        // Spot-check some required entries
        assert!(q.all().any(|b| b.id == "cave-etcd-deeper-003"));
        assert!(q.all().any(|b| b.id == "cave-portal-tenant-dashboard"));
        assert!(q.all().any(|b| b.id == "cave-tetragon-scaffold"));
    }
}
