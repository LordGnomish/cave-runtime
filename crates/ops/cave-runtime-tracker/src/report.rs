// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Daily report emitters — a human-readable `.md` digest grouped by
//! category and a machine `.json` record with the same data.
//!
//! The JSON layout is stable so a future cave-portal admin page and the
//! daily LaunchAgent can evolve independently.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::TrackerResult;
use crate::poll::PollSummary;
use crate::registry::DriftStatus;

/// One full daily report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyReport {
    pub schema_version: u32,
    pub generated_at_utc: String,
    pub totals: Totals,
    pub poll: PollSummary,
    /// Phase 0 mandate banner — repeated in JSON so downstream tooling
    /// cannot mistake a drift report for an executed upgrade.
    pub phase_0_no_auto_bump: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Totals {
    pub tracked: usize,
    pub in_sync: usize,
    pub behind: usize,
    pub unknown: usize,
}

impl DailyReport {
    pub fn assemble(poll: PollSummary) -> Self {
        let totals = Totals {
            tracked: poll.total(),
            in_sync: poll.count(DriftStatus::InSync),
            behind: poll.count(DriftStatus::Behind),
            unknown: poll.count(DriftStatus::Unknown),
        };
        Self {
            schema_version: 1,
            generated_at_utc: chrono::Utc::now().to_rfc3339(),
            totals,
            poll,
            phase_0_no_auto_bump: true,
        }
    }

    pub fn to_json(&self) -> TrackerResult<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&format!(
            "# cave-runtime-tracker — daily upstream drift ({})\n\n",
            self.generated_at_utc
        ));
        md.push_str(&format!("- tracked subsystems: **{}**\n", self.totals.tracked));
        md.push_str(&format!("- ✅ in-sync: {}\n", self.totals.in_sync));
        md.push_str(&format!("- ⚠️ behind: {}\n", self.totals.behind));
        md.push_str(&format!("- ❔ unknown (unpinned / unresolved): {}\n", self.totals.unknown));
        md.push_str("- phase 0: **report only, no auto-bump**\n\n");

        if !self.poll.unresolved.is_empty() {
            md.push_str(&format!(
                "> {} repo(s) could not be resolved this run (offline / rate-limited): {}\n\n",
                self.poll.unresolved.len(),
                self.poll.unresolved.join(", ")
            ));
        }

        // Group rows by category, categories alphabetised, rows by name.
        let mut by_cat: BTreeMap<&str, Vec<&crate::poll::PollResult>> = BTreeMap::new();
        for r in &self.poll.results {
            by_cat.entry(r.upstream.category.as_str()).or_default().push(r);
        }
        for (cat, mut rows) in by_cat {
            rows.sort_by(|a, b| a.upstream.name.cmp(&b.upstream.name));
            md.push_str(&format!("## {cat}\n\n"));
            md.push_str("| Subsystem | cave module | upstream | pinned | latest | status |\n");
            md.push_str("|-----------|-------------|----------|--------|--------|--------|\n");
            for r in rows {
                md.push_str(&format!(
                    "| {} | `{}` | [{}](https://github.com/{}) | {} | {} | {} |\n",
                    r.upstream.name,
                    r.upstream.cave_module,
                    r.upstream.repo,
                    r.upstream.repo,
                    r.upstream.pinned.as_deref().unwrap_or("—"),
                    r.latest.as_deref().unwrap_or("—"),
                    r.status.badge(),
                ));
            }
            md.push('\n');
        }
        md
    }

    /// A richer human digest than [`to_markdown`](Self::to_markdown): the
    /// drift summary, an explicit **behind** list (where action is owed),
    /// and — when `measurements` are present — a port-depth LOC table.
    /// This is the `daily-progress-<date>.md` artifact.
    pub fn to_progress_markdown(&self, measurements: &[crate::measure::Measurement]) -> String {
        use crate::registry::DriftStatus;
        let mut md = String::new();
        md.push_str(&format!(
            "# cave-runtime-tracker — daily progress ({})\n\n",
            self.generated_at_utc
        ));
        md.push_str(&format!(
            "**{}** subsystems tracked — ✅ {} in-sync · ⚠️ {} behind · ❔ {} unknown. \
             Phase 0: **report only, no auto-bump**.\n\n",
            self.totals.tracked, self.totals.in_sync, self.totals.behind, self.totals.unknown
        ));

        // Behind list — the rows where our port lags the upstream tag.
        let mut behind: Vec<&crate::poll::PollResult> = self
            .poll
            .results
            .iter()
            .filter(|r| r.status == DriftStatus::Behind)
            .collect();
        behind.sort_by(|a, b| a.upstream.name.cmp(&b.upstream.name));
        md.push_str(&format!("## ⚠️ Behind upstream ({})\n\n", behind.len()));
        if behind.is_empty() {
            md.push_str("_Nothing behind — every pinned port matches its latest upstream tag._\n\n");
        } else {
            md.push_str("| Subsystem | cave module | upstream | ported | latest |\n");
            md.push_str("|-----------|-------------|----------|--------|--------|\n");
            for r in behind {
                md.push_str(&format!(
                    "| {} | `{}` | [{}](https://github.com/{}) | {} | {} |\n",
                    r.upstream.name,
                    r.upstream.cave_module,
                    r.upstream.repo,
                    r.upstream.repo,
                    r.upstream.pinned.as_deref().unwrap_or("—"),
                    r.latest.as_deref().unwrap_or("—"),
                ));
            }
            md.push('\n');
        }

        // In-sync list — short roll-call so progress is visible.
        let in_sync: Vec<&str> = self
            .poll
            .results
            .iter()
            .filter(|r| r.status == DriftStatus::InSync)
            .map(|r| r.upstream.name.as_str())
            .collect();
        if !in_sync.is_empty() {
            md.push_str(&format!(
                "## ✅ In-sync ({})\n\n{}\n\n",
                in_sync.len(),
                in_sync.join(", ")
            ));
        }

        // Port-depth LOC table (only when measured this run).
        if !measurements.is_empty() {
            md.push_str("## 📏 Port depth — LOC (tokei)\n\n");
            md.push_str("Ratio = cave-crate code ÷ upstream code. A *focused* re-implementation, not a 1:1 line translation — read as a trend, not a parity score.\n\n");
            md.push_str("| cave module | upstream | upstream LOC | cave LOC | depth |\n");
            md.push_str("|-------------|----------|-------------:|---------:|------:|\n");
            for m in measurements {
                let up = m.upstream.map(|s| s.code.to_string()).unwrap_or_else(|| "—".into());
                let cave = m.cave.map(|s| s.code.to_string()).unwrap_or_else(|| "—".into());
                let depth = m
                    .ratio
                    .map(|r| format!("{:.2}%", r * 100.0))
                    .unwrap_or_else(|| "—".into());
                md.push_str(&format!(
                    "| `{}` | [{}](https://github.com/{}) | {} | {} | {} |\n",
                    m.cave_module, m.upstream_repo, m.upstream_repo, up, cave, depth
                ));
            }
            md.push('\n');
        }

        if !self.poll.unresolved.is_empty() {
            md.push_str(&format!(
                "> {} repo(s) unresolved this run (offline / rate-limited): {}\n",
                self.poll.unresolved.len(),
                self.poll.unresolved.join(", ")
            ));
        }
        md
    }

    /// Write `daily-<stamp>.{md,json}` into `dir`, plus `latest.json`
    /// when `emit_latest`. Returns the (json, md) paths written.
    pub fn write_to_dir(
        &self,
        dir: &Path,
        stamp: &str,
        emit_latest: bool,
    ) -> TrackerResult<(PathBuf, PathBuf)> {
        std::fs::create_dir_all(dir)?;
        let json = self.to_json()?;
        let json_path = dir.join(format!("daily-{stamp}.json"));
        let md_path = dir.join(format!("daily-{stamp}.md"));
        std::fs::write(&json_path, &json)?;
        std::fs::write(&md_path, self.to_markdown())?;
        if emit_latest {
            std::fs::write(dir.join("latest.json"), &json)?;
        }
        Ok((json_path, md_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrackerConfig;

    fn report() -> DailyReport {
        let cfg = TrackerConfig::default_config();
        DailyReport::assemble(PollSummary::from_registry_only(&cfg))
    }

    #[test]
    fn totals_sum_to_tracked() {
        let r = report();
        assert_eq!(
            r.totals.in_sync + r.totals.behind + r.totals.unknown,
            r.totals.tracked
        );
    }

    #[test]
    fn markdown_has_title_totals_and_a_table_header() {
        let md = report().to_markdown();
        assert!(md.contains("# cave-runtime-tracker"));
        assert!(md.contains("tracked subsystems:"));
        assert!(md.contains("| Subsystem | cave module |"));
        assert!(md.contains("no auto-bump"));
    }

    #[test]
    fn json_is_pretty_and_round_trips() {
        let r = report();
        let j = r.to_json().unwrap();
        assert!(j.contains("\n  \"schema_version\""));
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["phase_0_no_auto_bump"], true);
    }

    #[test]
    fn write_to_dir_emits_md_json_and_latest() {
        let dir = tempfile::tempdir().unwrap();
        let r = report();
        let (jp, mp) = r.write_to_dir(dir.path(), "2026-06-07", true).unwrap();
        assert!(jp.exists() && mp.exists());
        assert!(dir.path().join("latest.json").exists());
    }

    #[test]
    fn progress_markdown_lists_behind_and_loc() {
        use crate::measure::{LocStats, Measurement};
        use crate::registry::DriftStatus;
        let cfg = TrackerConfig::default_config();
        let mut summary = PollSummary::from_registry_only(&cfg);
        // Force one row Behind so the behind table renders.
        summary.results[0].upstream.pinned = Some("v1.0.0".to_string());
        summary.results[0].latest = Some("v2.0.0".to_string());
        summary.results[0].status = DriftStatus::Behind;
        let report = DailyReport::assemble(summary);

        let m = Measurement {
            upstream_repo: "cilium/cilium".to_string(),
            cave_module: "cave-net".to_string(),
            upstream: Some(LocStats { code: 400_000, ..Default::default() }),
            cave: Some(LocStats { code: 12_000, ..Default::default() }),
            ratio: Some(0.03),
        };
        let md = report.to_progress_markdown(&[m]);
        assert!(md.contains("# cave-runtime-tracker — daily progress"));
        assert!(md.contains("## ⚠️ Behind upstream (1)"));
        assert!(md.contains("v2.0.0")); // latest tag of the behind row
        assert!(md.contains("## 📏 Port depth — LOC (tokei)"));
        assert!(md.contains("`cave-net`"));
        assert!(md.contains("3.00%")); // ratio rendered as a percentage
    }

    #[test]
    fn progress_markdown_without_measurements_omits_loc_table() {
        let cfg = TrackerConfig::default_config();
        let report = DailyReport::assemble(PollSummary::from_registry_only(&cfg));
        let md = report.to_progress_markdown(&[]);
        assert!(!md.contains("Port depth"));
        // Registry-only → nothing behind.
        assert!(md.contains("Nothing behind"));
    }

    #[test]
    fn write_to_dir_skips_latest_when_disabled() {
        let dir = tempfile::tempdir().unwrap();
        report().write_to_dir(dir.path(), "2026-06-07", false).unwrap();
        assert!(!dir.path().join("latest.json").exists());
    }
}
