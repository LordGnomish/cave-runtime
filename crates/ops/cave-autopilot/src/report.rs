// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Daily report generation.
//!
//! At 23:55 each day the daemon renders a markdown report to
//! `docs/audit/autopilot-daily-<YYYY-MM-DD>.md` inside the instance's repo. It
//! captures the subsystem-progress delta, the failed-task list, Claude cost
//! tracking, and the next day's priority queue — the honest record of what the
//! 7/24 loop actually did. Rendering is pure; [`DailyReport::write`] is the
//! only I/O.

use crate::error::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One task's outcome as it appears in the report.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskLine {
    pub subsystem: String,
    pub completion_before: f64,
    pub completion_after: f64,
    /// Top tier that touched the task (`l2_coder`, `l3_claude`, …).
    pub tier: String,
    pub note: String,
}

impl TaskLine {
    pub fn delta(&self) -> f64 {
        self.completion_after - self.completion_before
    }
}

/// The full daily report model.
#[derive(Debug, Clone, PartialEq)]
pub struct DailyReport {
    pub date: String,
    pub instance: String,
    pub completed: Vec<TaskLine>,
    pub failed: Vec<TaskLine>,
    pub escalated_human: Vec<TaskLine>,
    pub claude_calls: u64,
    pub claude_tokens: u64,
    pub llm_calls: BTreeMap<String, u64>,
    pub mean_completion_start: f64,
    pub mean_completion_end: f64,
    pub next_queue: Vec<String>,
}

impl DailyReport {
    /// File name for a given date — `autopilot-daily-<date>.md`. Instances live
    /// in separate repos so the bare name never collides.
    pub fn file_name(date: &str) -> String {
        format!("autopilot-daily-{date}.md")
    }

    /// Full path inside a report directory.
    pub fn path_in(&self, report_dir: &Path) -> PathBuf {
        report_dir.join(Self::file_name(&self.date))
    }

    /// Total day-over-day mean-completion delta.
    pub fn mean_delta(&self) -> f64 {
        self.mean_completion_end - self.mean_completion_start
    }

    /// Render the markdown body.
    pub fn render_markdown(&self) -> String {
        let mut s = String::new();
        s.push_str("<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->\n");
        s.push_str(&format!("# Autopilot Daily — {} ({})\n\n", self.instance, self.date));

        s.push_str("## Summary\n\n");
        s.push_str(&format!("- **Completed:** {}\n", self.completed.len()));
        s.push_str(&format!("- **Failed:** {}\n", self.failed.len()));
        s.push_str(&format!("- **Escalated to human:** {}\n", self.escalated_human.len()));
        s.push_str(&format!(
            "- **Mean completion:** {:.4} → {:.4} (Δ {:+.4})\n",
            self.mean_completion_start,
            self.mean_completion_end,
            self.mean_delta()
        ));
        s.push_str(&format!(
            "- **Claude:** {} call(s), {} token(s)\n\n",
            self.claude_calls, self.claude_tokens
        ));

        s.push_str("## Completed tasks\n\n");
        if self.completed.is_empty() {
            s.push_str("_none_\n\n");
        } else {
            s.push_str("| Subsystem | Before | After | Δ | Tier | Note |\n");
            s.push_str("|---|---|---|---|---|---|\n");
            for t in &self.completed {
                s.push_str(&format!(
                    "| {} | {:.3} | {:.3} | {:+.3} | {} | {} |\n",
                    t.subsystem, t.completion_before, t.completion_after, t.delta(), t.tier, t.note
                ));
            }
            s.push('\n');
        }

        s.push_str("## Failed tasks\n\n");
        if self.failed.is_empty() {
            s.push_str("_none_\n\n");
        } else {
            for t in &self.failed {
                s.push_str(&format!("- `{}` — {} (tier {})\n", t.subsystem, t.note, t.tier));
            }
            s.push('\n');
        }

        s.push_str("## LLM call counts\n\n");
        for (tier, n) in &self.llm_calls {
            s.push_str(&format!("- `{tier}`: {n}\n"));
        }
        s.push('\n');

        s.push_str("## Next-day priority queue\n\n");
        if self.next_queue.is_empty() {
            s.push_str("_idle — all subsystems at threshold_\n");
        } else {
            for (i, q) in self.next_queue.iter().enumerate() {
                s.push_str(&format!("{}. `{}`\n", i + 1, q));
            }
        }
        s.push('\n');
        s
    }

    /// Write the report into `report_dir`, creating it if needed. Returns the
    /// path written.
    pub fn write(&self, report_dir: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(report_dir)?;
        let path = self.path_in(report_dir);
        std::fs::write(&path, self.render_markdown())?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DailyReport {
        let mut llm = BTreeMap::new();
        llm.insert("l2_coder".to_string(), 4u64);
        llm.insert("l3_claude".to_string(), 1u64);
        DailyReport {
            date: "2026-06-07".into(),
            instance: "cave-runtime".into(),
            completed: vec![TaskLine {
                subsystem: "cave-etcd".into(),
                completion_before: 0.52,
                completion_after: 0.58,
                tier: "l2_coder".into(),
                note: "ported raft step".into(),
            }],
            failed: vec![TaskLine {
                subsystem: "cave-policy".into(),
                completion_before: 0.65,
                completion_after: 0.65,
                tier: "l3_claude".into(),
                note: "tests still red after escalation".into(),
            }],
            escalated_human: vec![],
            claude_calls: 1,
            claude_tokens: 15000,
            llm_calls: llm,
            mean_completion_start: 0.70,
            mean_completion_end: 0.71,
            next_queue: vec!["port-cave-policy".into(), "port-cave-deploy".into()],
        }
    }

    #[test]
    fn file_name_is_dated() {
        assert_eq!(DailyReport::file_name("2026-06-07"), "autopilot-daily-2026-06-07.md");
    }

    #[test]
    fn markdown_contains_key_sections() {
        let md = sample().render_markdown();
        assert!(md.contains("# Autopilot Daily — cave-runtime (2026-06-07)"));
        assert!(md.contains("## Completed tasks"));
        assert!(md.contains("cave-etcd"));
        assert!(md.contains("+0.060")); // delta formatting
        assert!(md.contains("## Failed tasks"));
        assert!(md.contains("cave-policy"));
        assert!(md.contains("l3_claude`: 1"));
        assert!(md.contains("1. `port-cave-policy`"));
    }

    #[test]
    fn mean_delta_signed() {
        assert!((sample().mean_delta() - 0.01).abs() < 1e-9);
    }

    #[test]
    fn empty_queue_renders_idle() {
        let mut r = sample();
        r.next_queue.clear();
        assert!(r.render_markdown().contains("idle — all subsystems at threshold"));
    }

    #[test]
    fn write_emits_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = sample().write(dir.path()).unwrap();
        assert!(p.exists());
        assert!(p.ends_with("autopilot-daily-2026-06-07.md"));
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(body.contains("Autopilot Daily"));
    }
}
