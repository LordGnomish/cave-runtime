//! CAVE Upstream — Continuous open source project tracking.
//!
//! Monitors GitHub releases, changelogs, and RFCs from all tracked projects.
//! AI-assisted triage classifies changes as ADOPT / WATCH / SKIP.
//! ADOPT items auto-create GitHub Issues in cave-runtime repo.

pub mod tracker;
pub mod projects;

/// All upstream projects we track.
pub use projects::TRACKED_PROJECTS;
