// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-upstream-watchd — release watch daemon.
//!
//! Polls GitHub's `/releases/latest` for every tracked upstream, diffs
//! the result against the local `parity.manifest.toml::[upstream] version`
//! pin, parses the release notes into structured Added / Changed /
//! Deprecated / Breaking entries, and emits a `GAP_OPENED` event when
//! the upstream has moved past us.
//!
//! ## Architecture
//!
//! ```text
//!  tracked.rs               persistence.rs (state.json)
//!     │                            │
//!     ▼                            │
//!  poller.rs ──── HTTP/ETag ─── (cached or 304) ─── github releases
//!     │
//!     ▼
//!  diff.rs ──── semver compare ──── parity.manifest pin
//!     │
//!     ▼
//!  changelog.rs ──── parse release body ──── structured entries
//!     │
//!     ▼
//!  event.rs ──── append GAP_OPENED ──── events.jsonl
//! ```
//!
//! ## Auto-port loop (BACKLOG, NOT in this scaffold)
//!
//! The `GAP_OPENED` event is consumed by a future dispatcher that
//! turns the structured changelog into a prompt for the Qwen / Opus
//! port loop (Charter v2 gate). That dispatcher is explicitly out of
//! scope for the 2026-05-13 Fix-C scaffold; this crate is responsible
//! only for **detecting** and **publishing** gaps.

pub mod changelog;
pub mod diff;
pub mod event;
pub mod persistence;
pub mod poller;
pub mod tracked;

/// 2026-05-13 auto-port batch: Charter "self-improving" closer —
/// turns `GAP_OPENED` events into queued port tasks, verifies the
/// result against the charter-v2 gate, and records the outcome to
/// the audit JSONL.
pub mod auto_port;
pub mod auto_port_gate;
pub mod prompt;
pub mod task_queue;

/// 2026-05-13 TDD-strict mode: Charter §1 ("line-by-line TDD upstream
/// parity") turned into a verifiable signal. Walks branch history and
/// flags impl-without-prior-test / `#[ignore]` / mixed commits. Embedded
/// in [`auto_port_gate::CharterV2Gate`] when a [`tdd::GitInspector`] is
/// supplied, and exposed standalone via the `cave-tdd-check` binary.
pub mod tdd;

pub use changelog::{parse_release_body, Changelog, ChangelogEntry, ChangeKind};
pub use diff::{compare_pin_against_latest, Severity, VersionDiff};
pub use event::{emit, GapEvent, GapEventSink, JsonlSink};
pub use persistence::{WatchState, WatchStateEntry};
pub use poller::{fetch_latest, GitHubClient, PollOutcome};
pub use tracked::{load_from_workspace, TrackedProject};

pub use auto_port::{
    AutoPortDispatcher, AutoPortError, AutoPortStatus, DispatchedRecord, DispatchSummary,
    DispatcherConfig, VerifySummary,
};
pub use auto_port_gate::{CharterBaseline, CharterGate, CharterV2Gate, VerifyResult};
pub use prompt::{build_prompt, PortContext};
pub use task_queue::{
    DryRunTaskQueue, OpusTaskQueue, PumpTaskQueue, TaskId, TaskOutput, TaskQueue, TaskQueueError,
    TaskStatus,
};
pub use tdd::{
    analyze_tdd_compliance, scan_stubs, ClassifiedCommit, CommitKind, FileChange, FileChangeKind,
    FileKind, GitError, GitInspector, ShellGitInspector, TddAnalyzer, TddCompliance, TddDetails,
    TddError, TddFinding,
};

#[cfg(test)]
pub(crate) fn fixture_workspace() -> tempfile::TempDir {
    let d = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(d.path().join("crates")).unwrap();
    d
}
