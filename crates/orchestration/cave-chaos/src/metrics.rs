// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus metrics exporter for chaos experiments.
//!
//! Renders experiment counters in the Prometheus text exposition format, served
//! at `GET /api/chaos/metrics`. cave-metrics owns cluster-wide Prometheus
//! scraping, but a module exposing its own `/metrics` is standard and portable —
//! this is the in-crate exporter that a Prometheus server (or cave-metrics)
//! scrapes.

use crate::engine::is_high_risk;
use crate::models::{ChaosExperiment, ExperimentStatus};

/// Render the chaos experiment metrics in Prometheus text exposition format.
pub fn render_prometheus(experiments: &[ChaosExperiment]) -> String {
    let total = experiments.len();
    let by = |s: ExperimentStatus| experiments.iter().filter(|e| e.status == s).count();
    let draft = by(ExperimentStatus::Draft);
    let running = by(ExperimentStatus::Running);
    let completed = by(ExperimentStatus::Completed);
    let aborted = by(ExperimentStatus::Aborted);
    let failed = by(ExperimentStatus::Failed);
    let high_risk = experiments.iter().filter(|e| is_high_risk(e)).count();

    format!(
        "# HELP chaos_experiments_total Total chaos experiments tracked.\n\
         # TYPE chaos_experiments_total gauge\n\
         chaos_experiments_total {total}\n\
         # HELP chaos_experiments_by_status Chaos experiments by lifecycle status.\n\
         # TYPE chaos_experiments_by_status gauge\n\
         chaos_experiments_by_status{{status=\"draft\"}} {draft}\n\
         chaos_experiments_by_status{{status=\"running\"}} {running}\n\
         chaos_experiments_by_status{{status=\"completed\"}} {completed}\n\
         chaos_experiments_by_status{{status=\"aborted\"}} {aborted}\n\
         chaos_experiments_by_status{{status=\"failed\"}} {failed}\n\
         # HELP chaos_experiments_active Chaos experiments currently running.\n\
         # TYPE chaos_experiments_active gauge\n\
         chaos_experiments_active {running}\n\
         # HELP chaos_experiments_high_risk Chaos experiments targeting production namespaces.\n\
         # TYPE chaos_experiments_high_risk gauge\n\
         chaos_experiments_high_risk {high_risk}\n"
    )
}
