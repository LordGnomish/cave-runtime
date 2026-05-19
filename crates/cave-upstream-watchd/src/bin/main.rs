// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-upstream-watchd` — upstream release watch daemon.
//!
//! Usage:
//!   cave-upstream-watchd poll          one tick + write events, exit
//!   cave-upstream-watchd list          print tracked projects + pinned versions
//!   cave-upstream-watchd dump-events   print events.jsonl as JSON array
//!
//! Configuration:
//!   GitHub PAT is resolved by [`cave_upstream_watchd::keychain::resolve_github_token`]:
//!     1. macOS keychain — service `cave-upstream-watchd`, account `$USER`
//!     2. `GITHUB_TOKEN` env (DEPRECATED — emits a warn log)
//!     3. anonymous (60 req/h)
//!   CAVE_WATCHD_WORKSPACE     workspace root (default: walk up to Cargo.lock)
//!   CAVE_WATCHD_STATE         state.json path
//!   CAVE_WATCHD_EVENTS        events.jsonl path
//!   CAVE_WATCHD_MAX_PROJECTS  cap on projects polled in one tick (default: all)

use chrono::Utc;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{info, warn};

use cave_upstream_watchd::{
    changelog::parse_release_body,
    diff::compare_pin_against_latest,
    event::{read_events, GapEvent, GapEventSink, JsonlSink},
    persistence::WatchState,
    poller::{GitHubClient, PollOutcome},
    tracked::{load_from_workspace, TrackedProject},
};

#[derive(Parser, Debug)]
#[command(
    name = "cave-upstream-watchd",
    about = "Upstream release watch daemon — Charter self-healing/self-improving foundation"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// One-shot tick: poll every tracked upstream once + emit
    /// GAP_OPENED events for any that have moved past our pin.
    Poll {
        /// Override workspace root.
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Override state.json path.
        #[arg(long)]
        state: Option<PathBuf>,
        /// Override events.jsonl path.
        #[arg(long)]
        events: Option<PathBuf>,
    },
    /// Print the tracked-project list and exit.
    List {
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Print the events.jsonl contents as a JSON array.
    DumpEvents {
        #[arg(long)]
        events: Option<PathBuf>,
    },
    /// Auto-port dispatcher: one-shot scan over events.jsonl,
    /// submit any unprocessed GAP to the configured TaskQueue, then
    /// verify completed tasks against the charter-v2 gate. Honours
    /// `CAVE_AUTOPORT_DISABLE=1` kill switch.
    Dispatch {
        /// Override workspace root.
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Override events.jsonl path.
        #[arg(long)]
        events: Option<PathBuf>,
        /// Override dispatched.jsonl path.
        #[arg(long)]
        state: Option<PathBuf>,
        /// Override audit.jsonl path.
        #[arg(long)]
        audit: Option<PathBuf>,
        /// Backend: `dryrun` | `pump` | `claude-cli` | `hermes` | `opus`.
        /// Default: dryrun (safe — never side-effects until the operator
        /// opts in). `claude-cli` was the production default until
        /// 2026-05-19 when the local Claude Code session started
        /// returning HTTP 401; `hermes` is the local-LLM fallback that
        /// routes through the operator's configured Hermes provider
        /// (Ollama tier-1 by default — no API key, no Anthropic
        /// session required). `opus` keeps the Anthropic-API path for
        /// headless hosts.
        #[arg(long, default_value = "dryrun")]
        backend: String,
        /// Skip the verify_completed pass (only run scan_and_dispatch).
        #[arg(long)]
        scan_only: bool,
    },
    /// Print every dispatched record as a JSON array — diagnostic.
    DumpDispatched {
        #[arg(long)]
        state: Option<PathBuf>,
    },
}

fn workspace_root() -> PathBuf {
    if let Ok(p) = std::env::var("CAVE_WATCHD_WORKSPACE") {
        return PathBuf::from(p);
    }
    let mut cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if cur.join("Cargo.lock").is_file() {
            return cur;
        }
        if !cur.pop() {
            break;
        }
    }
    PathBuf::from(".")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Poll {
            workspace,
            state,
            events,
        } => {
            let root = workspace.unwrap_or_else(workspace_root);
            let state_path = state.unwrap_or_else(WatchState::default_path);
            let events_path = events.unwrap_or_else(JsonlSink::default_path);
            run_poll(&root, &state_path, &events_path).await
        }
        Cmd::List { workspace } => {
            let root = workspace.unwrap_or_else(workspace_root);
            let projects = load_from_workspace(&root)?;
            for p in &projects {
                println!(
                    "{:<25} {:<35} pin={:<12} priority={:?}",
                    p.cave_module,
                    p.github_repo,
                    p.current_pin.as_deref().unwrap_or("-"),
                    p.priority,
                );
            }
            println!("\n{} projects total", projects.len());
            Ok(())
        }
        Cmd::DumpEvents { events } => {
            let p = events.unwrap_or_else(JsonlSink::default_path);
            let evts = read_events(&p)?;
            println!("{}", serde_json::to_string_pretty(&evts)?);
            Ok(())
        }
        Cmd::Dispatch {
            workspace,
            events,
            state,
            audit,
            backend,
            scan_only,
        } => {
            let root = workspace.unwrap_or_else(workspace_root);
            let (default_events, default_state, default_audit) =
                cave_upstream_watchd::AutoPortDispatcher::default_paths();
            let events_path = events.unwrap_or(default_events);
            let state_path = state.unwrap_or(default_state);
            let audit_path = audit.unwrap_or(default_audit);
            run_dispatch(&root, &events_path, &state_path, &audit_path, &backend, scan_only).await
        }
        Cmd::DumpDispatched { state } => {
            let p = state.unwrap_or_else(|| {
                cave_upstream_watchd::AutoPortDispatcher::default_paths().1
            });
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let records: Vec<cave_upstream_watchd::DispatchedRecord> = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            println!("{}", serde_json::to_string_pretty(&records)?);
            Ok(())
        }
    }
}

async fn run_dispatch(
    workspace: &std::path::Path,
    events_path: &std::path::Path,
    state_path: &std::path::Path,
    audit_path: &std::path::Path,
    backend_str: &str,
    scan_only: bool,
) -> anyhow::Result<()> {
    use cave_upstream_watchd::{
        auto_port::{AutoPortDispatcher, DispatcherConfig, WorkspaceContextResolver},
        auto_port_gate::CharterV2Gate,
        task_queue::{
            ClaudeCliTaskQueue, DryRunTaskQueue, HermesTaskQueue, OpusTaskQueue,
            PumpTaskQueue, TaskQueue,
        },
    };
    use std::sync::Arc;

    let gate = Arc::new(CharterV2Gate::new(workspace.to_path_buf()));
    let stub_gate = gate.clone();
    let ratio_gate = gate.clone();
    let resolver = Arc::new(WorkspaceContextResolver {
        workspace_root: workspace.to_path_buf(),
        gate: gate.clone(),
        stub_counter: Arc::new(move || stub_gate.count_workspace_stubs().unwrap_or(0)),
        ratio_reader: Arc::new(move |crate_name: &str| {
            ratio_gate.read_fill_ratio(crate_name).ok().flatten()
        }),
    });

    let queue: Arc<dyn TaskQueue> = match backend_str {
        "dryrun" => {
            let log = audit_path.with_file_name("dryrun-audit.jsonl");
            tracing::info!(backend = "dryrun", log = %log.display(), "auto-port dispatcher up");
            Arc::new(DryRunTaskQueue::new(log))
        }
        "pump" => {
            let q = PumpTaskQueue::default_paths();
            tracing::info!(
                backend = "pump",
                queue = %q.queue_dir.display(),
                completed = %q.completed_dir.display(),
                "auto-port dispatcher up"
            );
            Arc::new(q)
        }
        "opus" => {
            let state_dir = state_path.with_file_name("opus");
            let q = OpusTaskQueue::from_env(state_dir).ok_or_else(|| {
                anyhow::anyhow!(
                    "ANTHROPIC_API_KEY not set — refusing to construct OpusTaskQueue. \
                     Set the env var or use --backend dryrun/pump/claude-cli."
                )
            })?;
            tracing::info!(backend = "opus", "auto-port dispatcher up");
            Arc::new(q)
        }
        "claude-cli" => {
            let log_dir = state_path.with_file_name("claude-cli");
            let q = ClaudeCliTaskQueue::from_env(log_dir).map_err(|e| {
                anyhow::anyhow!(
                    "could not initialise ClaudeCliTaskQueue: {e} \
                     (install Claude Code: https://claude.com/code; or use --backend hermes/pump)"
                )
            })?;
            tracing::info!(backend = "claude-cli", "auto-port dispatcher up");
            Arc::new(q)
        }
        "hermes" => {
            let log_dir = state_path.with_file_name("hermes");
            let q = HermesTaskQueue::from_env(log_dir).map_err(|e| {
                anyhow::anyhow!(
                    "could not initialise HermesTaskQueue: {e} \
                     (install Hermes Agent: `pipx install hermes-agent`; \
                     or use --backend pump/claude-cli)"
                )
            })?;
            tracing::info!(backend = "hermes", "auto-port dispatcher up");
            Arc::new(q)
        }
        other => {
            anyhow::bail!(
                "unknown backend '{other}' — pick dryrun | pump | opus | claude-cli | hermes"
            );
        }
    };

    let dispatcher = AutoPortDispatcher::new(
        events_path.to_path_buf(),
        state_path.to_path_buf(),
        audit_path.to_path_buf(),
        queue,
        gate,
        resolver,
        DispatcherConfig::default(),
    );
    dispatcher.boot().await?;

    let scan = dispatcher.scan_and_dispatch().await?;
    tracing::info!(?scan, "scan_and_dispatch complete");
    println!(
        "dispatch: considered={} dispatched={} already={} skipped_disabled={} skipped_cooldown={} skipped_rate={} errors={}",
        scan.considered,
        scan.dispatched,
        scan.already_dispatched,
        scan.skipped_disabled,
        scan.skipped_cooldown,
        scan.skipped_rate_limit,
        scan.errors,
    );
    if !scan_only {
        let verify = dispatcher.verify_completed().await?;
        tracing::info!(?verify, "verify_completed complete");
        println!(
            "verify:   considered={} still_running={} merged={} charter_failed={} backend_failed={}",
            verify.considered,
            verify.still_running,
            verify.merged,
            verify.charter_failed,
            verify.backend_failed,
        );
    }
    Ok(())
}

async fn run_poll(
    workspace: &std::path::Path,
    state_path: &std::path::Path,
    events_path: &std::path::Path,
) -> anyhow::Result<()> {
    let projects = load_from_workspace(workspace)?;
    info!(count = projects.len(), workspace = %workspace.display(), "loaded tracked projects");

    let max_projects: usize = std::env::var("CAVE_WATCHD_MAX_PROJECTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(projects.len());

    let (token, source) = cave_upstream_watchd::keychain::resolve_github_token(None);
    if token.is_none() {
        warn!(
            "no GitHub PAT found in keychain (service `cave-upstream-watchd`) \
             nor in $GITHUB_TOKEN — anonymous limit is 60 req/h, watchd \
             will throttle quickly. Run `security add-generic-password -U \
             -s cave-upstream-watchd -a \"$USER\" -w <PAT>` to enable."
        );
    } else {
        info!(source = source, "github PAT resolved");
    }
    let client = GitHubClient::new(token);
    let mut state = WatchState::load(state_path)?;
    let sink = JsonlSink::new(events_path.to_path_buf());

    let mut new_releases = 0;
    let mut no_release = 0;
    let mut rate_limited = 0;
    let mut not_modified = 0;
    let mut errors = 0;

    for project in projects.iter().take(max_projects) {
        match poll_one(&client, &sink, &mut state, project, Utc::now()).await {
            Ok(PollOutcome::NewRelease { .. }) => new_releases += 1,
            Ok(PollOutcome::NotModified { .. }) => not_modified += 1,
            Ok(PollOutcome::NoRelease) => no_release += 1,
            Ok(PollOutcome::RateLimited { .. }) => {
                rate_limited += 1;
                warn!(repo = %project.github_repo, "rate limited — stopping tick");
                break;
            }
            Err(e) => {
                errors += 1;
                warn!(repo = %project.github_repo, error = %e, "poll error");
            }
        }
    }

    state.last_tick_at = Some(Utc::now());
    state.save(state_path)?;

    info!(
        new_releases,
        not_modified,
        no_release,
        rate_limited,
        errors,
        "tick complete"
    );
    println!(
        "watchd tick: new={} not_modified={} no_release={} rate_limited={} errors={}",
        new_releases, not_modified, no_release, rate_limited, errors,
    );
    Ok(())
}

/// Poll one project. Returns the underlying outcome so the caller
/// can tally aggregates. Side-effects: appends an event if there's a
/// new release that's actually beyond our pin, and writes the state
/// entry regardless of outcome.
pub async fn poll_one(
    client: &GitHubClient,
    sink: &dyn GapEventSink,
    state: &mut WatchState,
    project: &TrackedProject,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<PollOutcome, anyhow::Error> {
    let prev = state.get(&project.github_repo).cloned().unwrap_or_default();
    let outcome = client
        .fetch_latest(
            &project.github_repo,
            prev.etag.as_deref(),
            prev.last_modified.as_deref(),
        )
        .await?;

    match &outcome {
        PollOutcome::NewRelease {
            release,
            etag,
            last_modified,
            ..
        } => {
            let diff = compare_pin_against_latest(
                project.current_pin.as_deref(),
                &release.tag_name,
            );
            state.upsert(&project.github_repo, |e| {
                e.last_poll = Some(now);
                e.last_known_tag = Some(release.tag_name.clone());
                e.etag = etag.clone();
                e.last_modified = last_modified.clone();
                e.consecutive_errors = 0;
            });
            if diff.outdated {
                let cl = parse_release_body(release.body.as_deref().unwrap_or(""));
                let gap_age = release.published_at.map(|t| (now - t).num_seconds());
                let event = GapEvent::new(
                    project.cave_module.clone(),
                    project.github_repo.clone(),
                    project.current_pin.clone(),
                    release.tag_name.clone(),
                    diff.severity,
                    gap_age,
                    None, // parity_ratio injection deferred — daemon would read parity-index.json
                    cl,
                    now,
                );
                sink.emit(&event)?;
            }
        }
        PollOutcome::NotModified { .. } => {
            state.upsert(&project.github_repo, |e| {
                e.last_poll = Some(now);
                e.consecutive_errors = 0;
            });
        }
        PollOutcome::NoRelease => {
            state.upsert(&project.github_repo, |e| {
                e.last_poll = Some(now);
                e.consecutive_errors = 0;
            });
        }
        PollOutcome::RateLimited { .. } => {
            // Do not advance state — leave etag/last_known_tag intact
            // so the next tick retries with the same cache.
        }
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cave_upstream_watchd::tracked::Priority;
    use httpmock::{Method::GET, MockServer};

    fn project(name: &str, repo: &str, pin: &str) -> TrackedProject {
        TrackedProject {
            cave_module: name.into(),
            github_repo: repo.into(),
            current_pin: Some(pin.into()),
            priority: Priority::Normal,
        }
    }

    fn release_json(tag: &str, body: &str) -> String {
        serde_json::to_string(&serde_json::json!({
            "tag_name": tag,
            "name": tag,
            "body": body,
            "published_at": "2026-05-13T10:00:00Z",
            "html_url": format!("https://github.com/x/y/releases/{tag}"),
            "prerelease": false,
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn poll_one_emits_gap_event_when_upstream_ahead() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/repos/foo/bar/releases/latest");
            then.status(200)
                .header("etag", "\"e1\"")
                .header("content-type", "application/json")
                .body(release_json("v1.2.0", "## Added\n- new feature\n"));
        });
        let client = GitHubClient::with_base_url(None, server.base_url());
        let dir = tempfile::TempDir::new().unwrap();
        let events_path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(events_path.clone());
        let mut state = WatchState::default();
        let now = chrono::Utc::now();
        let outcome = poll_one(
            &client,
            &sink,
            &mut state,
            &project("cave-foo", "foo/bar", "v1.0.0"),
            now,
        )
        .await
        .unwrap();
        assert!(matches!(outcome, PollOutcome::NewRelease { .. }));
        let events = read_events(&events_path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].cave_module, "cave-foo");
        assert_eq!(events[0].latest_tag, "v1.2.0");
        assert_eq!(events[0].changelog.entries.len(), 1);
    }

    #[tokio::test]
    async fn poll_one_no_event_when_pin_matches_latest() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/repos/foo/bar/releases/latest");
            then.status(200)
                .header("content-type", "application/json")
                .body(release_json("v1.0.0", ""));
        });
        let client = GitHubClient::with_base_url(None, server.base_url());
        let dir = tempfile::TempDir::new().unwrap();
        let events_path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(events_path.clone());
        let mut state = WatchState::default();
        let outcome = poll_one(
            &client,
            &sink,
            &mut state,
            &project("cave-foo", "foo/bar", "v1.0.0"),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, PollOutcome::NewRelease { .. }));
        let events = read_events(&events_path).unwrap();
        assert!(events.is_empty(), "no gap expected when pin == latest");
    }

    #[tokio::test]
    async fn poll_one_updates_state_etag_after_new_release() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/repos/foo/bar/releases/latest");
            then.status(200)
                .header("etag", "\"new-etag\"")
                .header("last-modified", "Wed, 13 May 2026 12:00:00 GMT")
                .header("content-type", "application/json")
                .body(release_json("v2.0.0", ""));
        });
        let client = GitHubClient::with_base_url(None, server.base_url());
        let dir = tempfile::TempDir::new().unwrap();
        let mut state = WatchState::default();
        let sink = JsonlSink::new(dir.path().join("events.jsonl"));
        let _ = poll_one(
            &client,
            &sink,
            &mut state,
            &project("cave-foo", "foo/bar", "v1.0.0"),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
        let entry = state.get("foo/bar").unwrap();
        assert_eq!(entry.etag.as_deref(), Some("\"new-etag\""));
        assert_eq!(entry.last_known_tag.as_deref(), Some("v2.0.0"));
        assert_eq!(entry.consecutive_errors, 0);
    }

    #[tokio::test]
    async fn poll_one_no_release_does_not_emit_event() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/repos/foo/bar/releases/latest");
            then.status(404);
        });
        let client = GitHubClient::with_base_url(None, server.base_url());
        let dir = tempfile::TempDir::new().unwrap();
        let events_path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(events_path.clone());
        let mut state = WatchState::default();
        let outcome = poll_one(
            &client,
            &sink,
            &mut state,
            &project("cave-foo", "foo/bar", "v1.0.0"),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, PollOutcome::NoRelease));
        assert!(read_events(&events_path).unwrap().is_empty());
    }
}
