// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: postgres/postgres@c372fbbd8e911f2412b80a8c39d7079366565d67 src/backend/commands/explain.c
//! `EXPLAIN [ANALYZE]` plan formatting.
//!
//! Port of the text-format branch of `src/backend/commands/explain.c`
//! (`ExplainOnePlan`, `ExplainNode`). Cave-rdbms currently runs a
//! single-pass executor so cost estimates are placeholders; the
//! `ExplainNode`-style indentation, "Plan" header, "Filter:" / "Sort
//! Key:" lines, and the `ANALYZE`-only `actual time` / `rows` columns
//! all match PG's renderer byte-for-byte where possible.

use std::time::{Duration, Instant};

/// One row in the rendered plan tree. PG calls these "plan nodes"
/// (`Plan` struct in `nodes/plannodes.h`).
#[derive(Debug, Clone, PartialEq)]
pub struct ExplainRow {
    /// Indent depth — 0 = root, +1 per "->" arrow.
    pub depth: usize,
    /// e.g. `"Seq Scan"`, `"Sort"`, `"Limit"`.
    pub node_type: String,
    /// Optional secondary line(s) — `"Sort Key: ..."`, `"Filter: ..."`.
    pub details: Vec<String>,
    /// Estimated cost — PG renders `(cost=0.00..15.00 rows=10 width=4)`.
    pub estimated_rows: u64,
    /// Wall-clock timing for this node — only populated by `EXPLAIN ANALYZE`.
    pub actual: Option<NodeTiming>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodeTiming {
    pub elapsed: Duration,
    pub rows: u64,
    pub loops: u64,
}

/// A full plan tree. Construction is left to the planner / executor;
/// this module is responsible for *rendering* it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExplainPlan {
    pub rows: Vec<ExplainRow>,
    /// Total elapsed for the root — `Execution Time: X.Y ms` footer.
    pub total_elapsed: Option<Duration>,
    /// Planning time — `Planning Time: X.Y ms`.
    pub planning_elapsed: Option<Duration>,
}

impl ExplainPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, row: ExplainRow) {
        self.rows.push(row);
    }

    /// Text-format renderer — mirrors `ExplainText` (`explain.c`).
    /// Indent is two spaces per depth level after the root, with a
    /// "->" arrow at the start of every non-root node.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for row in &self.rows {
            self.render_row(row, &mut out);
        }
        if let Some(p) = self.planning_elapsed {
            out.push_str(&format!("Planning Time: {:.3} ms\n", duration_ms(p)));
        }
        if let Some(t) = self.total_elapsed {
            out.push_str(&format!("Execution Time: {:.3} ms\n", duration_ms(t)));
        }
        out
    }

    fn render_row(&self, row: &ExplainRow, out: &mut String) {
        // Indent + arrow. PG: root has no leading "->", children do.
        if row.depth == 0 {
            // no prefix
        } else {
            for _ in 0..(row.depth - 1) {
                out.push_str("  ");
            }
            out.push_str("->  ");
        }
        out.push_str(&row.node_type);
        // Cost block. With no estimator we emit `cost=0.00..0.00`.
        out.push_str(&format!(
            "  (cost=0.00..0.00 rows={} width=0)",
            row.estimated_rows
        ));
        if let Some(a) = &row.actual {
            out.push_str(&format!(
                " (actual time={:.3}..{:.3} rows={} loops={})",
                duration_ms(a.elapsed),
                duration_ms(a.elapsed),
                a.rows,
                a.loops
            ));
        }
        out.push('\n');
        for d in &row.details {
            for _ in 0..(row.depth) {
                out.push_str("  ");
            }
            out.push_str("  ");
            out.push_str(d);
            out.push('\n');
        }
    }
}

fn duration_ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// `EXPLAIN ANALYZE` collector. Pin a `Stopwatch` at the entry of each
/// executor node, stop it on emit. Mirrors PG's `InstrStartNode` /
/// `InstrStopNode` instrumentation in `executor/instrument.c`.
pub struct Stopwatch {
    start: Instant,
    rows: u64,
    loops: u64,
}

impl Stopwatch {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
            rows: 0,
            loops: 0,
        }
    }

    pub fn add_row(&mut self) {
        self.rows += 1;
    }

    pub fn add_loop(&mut self) {
        self.loops += 1;
    }

    pub fn finish(self) -> NodeTiming {
        NodeTiming {
            elapsed: self.start.elapsed(),
            rows: self.rows,
            loops: self.loops.max(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn row(depth: usize, ty: &str, rows: u64) -> ExplainRow {
        ExplainRow {
            depth,
            node_type: ty.into(),
            details: vec![],
            estimated_rows: rows,
            actual: None,
        }
    }

    #[test]
    fn test_explain_empty_plan_renders_empty() {
        let p = ExplainPlan::new();
        assert_eq!(p.render(), "");
    }

    #[test]
    fn test_explain_single_node_emits_cost_block() {
        let mut p = ExplainPlan::new();
        p.add(row(0, "Seq Scan on users", 100));
        let txt = p.render();
        assert!(txt.contains("Seq Scan on users  (cost=0.00..0.00 rows=100 width=0)"));
    }

    #[test]
    fn test_explain_child_node_has_arrow_and_indent() {
        let mut p = ExplainPlan::new();
        p.add(row(0, "Limit", 10));
        p.add(row(1, "Seq Scan on users", 100));
        let txt = p.render();
        // Second line starts with "->  "
        let lines: Vec<&str> = txt.lines().collect();
        assert!(lines[1].starts_with("->  Seq Scan on users"));
    }

    #[test]
    fn test_explain_deep_nesting_indents_two_spaces_per_level() {
        let mut p = ExplainPlan::new();
        p.add(row(0, "Limit", 10));
        p.add(row(1, "Sort", 100));
        p.add(row(2, "Seq Scan", 1000));
        let txt = p.render();
        let lines: Vec<&str> = txt.lines().collect();
        // depth=2 → 1 level of indent before "->  "
        assert!(lines[2].starts_with("  ->  Seq Scan"));
    }

    #[test]
    fn test_explain_detail_lines_indented_under_node() {
        let mut p = ExplainPlan::new();
        let mut sort = row(0, "Sort", 100);
        sort.details.push("Sort Key: users.id".into());
        sort.details.push("Sort Method: quicksort".into());
        p.add(sort);
        let txt = p.render();
        assert!(txt.contains("\n  Sort Key: users.id\n"));
        assert!(txt.contains("\n  Sort Method: quicksort\n"));
    }

    #[test]
    fn test_explain_analyze_appends_actual_time_block() {
        let mut p = ExplainPlan::new();
        let mut r = row(0, "Seq Scan on t", 10);
        r.actual = Some(NodeTiming {
            elapsed: Duration::from_micros(2_500),
            rows: 10,
            loops: 1,
        });
        p.add(r);
        let txt = p.render();
        assert!(txt.contains("actual time=2.500..2.500 rows=10 loops=1"));
    }

    #[test]
    fn test_explain_planning_and_execution_time_footers() {
        let mut p = ExplainPlan::new();
        p.add(row(0, "Result", 1));
        p.planning_elapsed = Some(Duration::from_micros(150));
        p.total_elapsed = Some(Duration::from_micros(900));
        let txt = p.render();
        assert!(txt.ends_with("Planning Time: 0.150 ms\nExecution Time: 0.900 ms\n"));
    }

    #[test]
    fn test_stopwatch_finish_reports_elapsed_and_counts() {
        let mut sw = Stopwatch::start();
        sw.add_row();
        sw.add_row();
        sw.add_loop();
        sleep(Duration::from_millis(1));
        let t = sw.finish();
        assert_eq!(t.rows, 2);
        assert_eq!(t.loops, 1);
        assert!(t.elapsed >= Duration::from_millis(1));
    }

    #[test]
    fn test_stopwatch_defaults_to_one_loop() {
        // PG instrumentation always reports `loops >= 1` so that the
        // per-loop division never goes through 0.
        let sw = Stopwatch::start();
        let t = sw.finish();
        assert_eq!(t.loops, 1);
        assert_eq!(t.rows, 0);
    }
}
