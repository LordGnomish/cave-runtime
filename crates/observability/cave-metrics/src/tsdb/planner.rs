// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Leveled-compaction planner — which on-disk blocks to merge next.
//!
//! Direct port of prometheus/prometheus `tsdb/compact.go` (v3.12.0):
//!   * `ExponentialBlockRanges` → [`exponential_block_ranges`]
//!   * `LeveledCompactor.plan`  → [`Planner::plan`]
//!   * `selectDirs` / `selectOverlappingDirs` / `splitByRange`
//!
//! The planner is metadata-only: it reasons about block time ranges and
//! tombstone statistics, never about sample data. Selection priority mirrors
//! upstream exactly:
//!   1. consecutive *overlapping* blocks (always merged first),
//!   2. a range-aligned group that fills a full compaction range (or sits
//!      entirely before the most-recent block),
//!   3. a "big enough" block with >5% tombstones, or any block that is
//!      entirely deleted.

/// `ExponentialBlockRanges(minSize, steps, stepSize)` — the exponential
/// sequence of compaction range widths: `ranges[i] = min_size * step_size^i`.
pub fn exponential_block_ranges(min_size: i64, steps: usize, step_size: i64) -> Vec<i64> {
    let mut ranges = Vec::with_capacity(steps);
    let mut cur = min_size;
    for _ in 0..steps {
        ranges.push(cur);
        cur *= step_size;
    }
    ranges
}

/// Per-block tombstone/series statistics (`tsdb.BlockStats` subset).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockStats {
    pub num_series: u64,
    pub num_tombstones: u64,
}

/// Block metadata the planner reasons about (`tsdb.dirMeta` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockMeta {
    pub dir: String,
    pub min_time: i64,
    pub max_time: i64,
    pub stats: BlockStats,
    /// `Compaction.Failed` — failed blocks are never folded into a range group.
    pub compaction_failed: bool,
}

/// Leveled compaction planner parameterised by its range widths.
#[derive(Debug, Clone)]
pub struct Planner {
    ranges: Vec<i64>,
    enable_overlapping_compaction: bool,
}

impl Planner {
    /// New planner over the given (ascending) compaction range widths.
    /// Overlapping-block compaction is enabled by default, matching the
    /// upstream `Options{EnableOverlappingCompaction: true}` head path.
    pub fn new(ranges: Vec<i64>) -> Self {
        Self { ranges, enable_overlapping_compaction: true }
    }

    /// Toggle the overlapping-block priority (upstream
    /// `enableOverlappingCompaction`).
    pub fn with_overlapping_compaction(mut self, enabled: bool) -> Self {
        self.enable_overlapping_compaction = enabled;
        self
    }

    /// `plan(dms)` — return the block directories to compact next, in
    /// ascending `min_time` order, or empty when nothing should be compacted.
    pub fn plan(&self, mut dms: Vec<BlockMeta>) -> Vec<String> {
        // Sort blocks by MinTime.
        dms.sort_by(|a, b| a.min_time.cmp(&b.min_time));

        // Priority 1: overlapping blocks.
        let overlapping = self.select_overlapping_dirs(&dms);
        if !overlapping.is_empty() {
            return overlapping;
        }

        if dms.is_empty() {
            return Vec::new();
        }

        // Priority 2: range-aligned group (excluding the newest block).
        let without_newest = &dms[..dms.len() - 1];
        let res: Vec<String> = self
            .select_dirs(without_newest)
            .iter()
            .map(|dm| dm.dir.clone())
            .collect();
        if !res.is_empty() {
            return res;
        }

        // Priority 3: tombstone rewrite. Only blocks "big enough"
        // (span >= ranges[len/2]) are rewritten on the >5% rule; smaller
        // blocks are only rewritten when entirely deleted.
        if self.ranges.is_empty() {
            return Vec::new();
        }
        let half = self.ranges[self.ranges.len() / 2];
        for dm in without_newest.iter().rev() {
            let meta = &dm.stats;
            if dm.max_time - dm.min_time < half {
                // Entirely deleted? Then size doesn't matter.
                if meta.num_tombstones > 0 && meta.num_tombstones >= meta.num_series {
                    return vec![dm.dir.clone()];
                }
                break;
            }
            if meta.num_tombstones as f64 / (meta.num_series as f64 + 1.0) > 0.05 {
                return vec![dm.dir.clone()];
            }
        }

        Vec::new()
    }

    /// `selectDirs` — the first range-aligned group that either fills a full
    /// compaction range or sits entirely before the most-recent block.
    fn select_dirs<'a>(&self, ds: &'a [BlockMeta]) -> &'a [BlockMeta] {
        if self.ranges.len() < 2 || ds.is_empty() {
            return &[];
        }
        let high_time = ds[ds.len() - 1].min_time;

        for &iv in &self.ranges[1..] {
            for (start, end) in split_by_range(ds, iv) {
                let p = &ds[start..end];
                // Skip groups containing a failed compaction.
                if p.iter().any(|dm| dm.compaction_failed) {
                    continue;
                }
                let mint = p[0].min_time;
                let maxt = p[p.len() - 1].max_time;
                if (maxt - mint == iv || maxt <= high_time) && p.len() > 1 {
                    return p;
                }
            }
        }
        &[]
    }

    /// `selectOverlappingDirs` — the longest leading run of consecutive blocks
    /// whose time ranges overlap (input must be sorted by `min_time`).
    fn select_overlapping_dirs(&self, ds: &[BlockMeta]) -> Vec<String> {
        if !self.enable_overlapping_compaction || ds.len() < 2 {
            return Vec::new();
        }
        let mut overlapping: Vec<String> = Vec::new();
        let mut global_maxt = ds[0].max_time;

        for i in 1..ds.len() {
            let d = &ds[i];
            if d.min_time < global_maxt {
                if overlapping.is_empty() {
                    overlapping.push(ds[i - 1].dir.clone());
                }
                overlapping.push(d.dir.clone());
            } else if !overlapping.is_empty() {
                break;
            }
            if d.max_time > global_maxt {
                global_maxt = d.max_time;
            }
        }
        overlapping
    }
}

/// `splitByRange` — partition blocks into groups aligned to multiples of `tr`.
/// Returns half-open `[start, end)` index ranges into `ds` so callers can slice
/// without cloning. A block whose `max_time` exceeds its aligned window
/// `[t0, t0+tr)` is skipped (it belongs to a coarser range).
fn split_by_range(ds: &[BlockMeta], tr: i64) -> Vec<(usize, usize)> {
    let mut splits = Vec::new();
    let mut i = 0usize;
    while i < ds.len() {
        let m = &ds[i];
        // Aligned range start [t0, t0+tr).
        let t0 = if m.min_time >= 0 {
            tr * (m.min_time / tr)
        } else {
            tr * ((m.min_time - tr + 1) / tr)
        };
        // Block too wide for this range — advance and retry at the next block.
        if m.max_time > t0 + tr {
            i += 1;
            continue;
        }
        let start = i;
        while i < ds.len() && ds[i].max_time <= t0 + tr {
            i += 1;
        }
        if i > start {
            splits.push((start, i));
        }
    }
    splits
}
