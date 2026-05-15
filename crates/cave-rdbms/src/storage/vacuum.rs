// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: postgres/postgres@c372fbbd8e911f2412b80a8c39d7079366565d67 src/backend/commands/vacuum.c
//! `VACUUM` / `VACUUM FULL` / `VACUUM ANALYZE` bookkeeping.
//!
//! Port of `src/backend/commands/vacuum.c` (vacuum entry point) +
//! `src/backend/access/heap/vacuumlazy.c` (live/dead/frozen tuple
//! accounting). We don't run a real MVCC heap, but we keep per-table
//! `VacuumStats` matching what `pg_stat_user_tables` exposes:
//!
//!   * `n_live_tup` / `n_dead_tup`
//!   * `last_vacuum` / `last_autovacuum`
//!   * `vacuum_count` / `autovacuum_count`
//!   * `wraparound_age` (xmin distance to `vacuum_freeze_max_age`)
//!
//! The autovacuum-trigger formula is the standard PG one:
//!   `threshold = autovacuum_vacuum_threshold + autovacuum_vacuum_scale_factor * n_live_tup`.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

/// Per-table accounting block — one row of `pg_stat_user_tables`.
#[derive(Debug, Clone, PartialEq)]
pub struct VacuumStats {
    pub schema: String,
    pub table: String,
    pub n_live_tup: u64,
    pub n_dead_tup: u64,
    /// Last time `VACUUM` (manual) ran. `None` until first vacuum.
    pub last_vacuum: Option<SystemTime>,
    /// Last time autovacuum ran. `None` until first run.
    pub last_autovacuum: Option<SystemTime>,
    pub vacuum_count: u64,
    pub autovacuum_count: u64,
    /// Distance from the oldest tuple's xmin to the wraparound limit
    /// (`vacuum_freeze_max_age`, default 200M). Anti-wraparound vacuum
    /// fires at 95% (manual §25.1.5). We model this as a single counter.
    pub wraparound_age: u64,
}

impl VacuumStats {
    pub fn new(schema: impl Into<String>, table: impl Into<String>) -> Self {
        Self {
            schema: schema.into(),
            table: table.into(),
            n_live_tup: 0,
            n_dead_tup: 0,
            last_vacuum: None,
            last_autovacuum: None,
            vacuum_count: 0,
            autovacuum_count: 0,
            wraparound_age: 0,
        }
    }

    /// Count a successful INSERT.
    pub fn record_insert(&mut self) {
        self.n_live_tup += 1;
    }

    /// Count a DELETE — the tuple becomes dead (heap-only).
    pub fn record_delete(&mut self) {
        if self.n_live_tup > 0 {
            self.n_live_tup -= 1;
        }
        self.n_dead_tup += 1;
    }

    /// Count an UPDATE — old tuple dead, new tuple live (HOT may avoid
    /// this but we don't model HOT separately).
    pub fn record_update(&mut self) {
        self.n_dead_tup += 1;
        // n_live_tup unchanged: -1 then +1.
    }

    /// Trigger formula. Returns true when dead tuples exceed the
    /// threshold = base + scale_factor * live.
    pub fn needs_autovacuum(&self, settings: &AutovacuumSettings) -> bool {
        let threshold = settings.vacuum_threshold as f64
            + settings.vacuum_scale_factor * self.n_live_tup as f64;
        (self.n_dead_tup as f64) > threshold
    }

    /// Anti-wraparound trigger. PG fires VACUUM (forced) when
    /// `wraparound_age > autovacuum_freeze_max_age * 0.95`.
    pub fn needs_anti_wraparound(&self, settings: &AutovacuumSettings) -> bool {
        let limit = (settings.freeze_max_age as f64 * 0.95) as u64;
        self.wraparound_age > limit
    }

    /// Apply a manual `VACUUM`. Clears dead tuples + resets wraparound age.
    pub fn apply_manual_vacuum(&mut self, now: SystemTime) {
        self.n_dead_tup = 0;
        self.wraparound_age = 0;
        self.last_vacuum = Some(now);
        self.vacuum_count += 1;
    }

    /// Apply an autovacuum cycle.
    pub fn apply_autovacuum(&mut self, now: SystemTime) {
        self.n_dead_tup = 0;
        self.wraparound_age = 0;
        self.last_autovacuum = Some(now);
        self.autovacuum_count += 1;
    }
}

/// Cluster-wide autovacuum knobs (`postgresql.conf`):
/// `autovacuum_vacuum_threshold`, `_scale_factor`, `_freeze_max_age`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutovacuumSettings {
    pub vacuum_threshold: u64,
    pub vacuum_scale_factor: f64,
    pub freeze_max_age: u64,
}

impl AutovacuumSettings {
    /// PG defaults (`postgresql.conf.sample`).
    pub const fn defaults() -> Self {
        Self {
            vacuum_threshold: 50,
            vacuum_scale_factor: 0.2,
            freeze_max_age: 200_000_000,
        }
    }
}

impl Default for AutovacuumSettings {
    fn default() -> Self {
        Self::defaults()
    }
}

/// Per-cluster vacuum registry.
#[derive(Default)]
pub struct VacuumRegistry {
    inner: Mutex<HashMap<(String, String), VacuumStats>>,
    settings: Mutex<AutovacuumSettings>,
}

impl VacuumRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<(String, String), VacuumStats>> {
        self.inner.lock().expect("VacuumRegistry mutex poisoned")
    }

    pub fn settings(&self) -> AutovacuumSettings {
        *self.settings.lock().unwrap()
    }

    pub fn set_settings(&self, s: AutovacuumSettings) {
        *self.settings.lock().unwrap() = s;
    }

    pub fn ensure(&self, schema: &str, table: &str) {
        let mut map = self.lock();
        map.entry((schema.into(), table.into()))
            .or_insert_with(|| VacuumStats::new(schema, table));
    }

    pub fn record_insert(&self, schema: &str, table: &str) {
        let mut map = self.lock();
        map.entry((schema.into(), table.into()))
            .or_insert_with(|| VacuumStats::new(schema, table))
            .record_insert();
    }

    pub fn record_delete(&self, schema: &str, table: &str) {
        let mut map = self.lock();
        map.entry((schema.into(), table.into()))
            .or_insert_with(|| VacuumStats::new(schema, table))
            .record_delete();
    }

    pub fn record_update(&self, schema: &str, table: &str) {
        let mut map = self.lock();
        map.entry((schema.into(), table.into()))
            .or_insert_with(|| VacuumStats::new(schema, table))
            .record_update();
    }

    /// Manually advance the wraparound age — emulates xmin moving on.
    /// Real PG ages by xid count; we age by tick count.
    pub fn age(&self, schema: &str, table: &str, delta: u64) {
        let mut map = self.lock();
        if let Some(s) = map.get_mut(&(schema.into(), table.into())) {
            s.wraparound_age = s.wraparound_age.saturating_add(delta);
        }
    }

    pub fn stats(&self, schema: &str, table: &str) -> Option<VacuumStats> {
        self.lock().get(&(schema.into(), table.into())).cloned()
    }

    /// Tables in (schema, table) order — stable for rendering.
    pub fn all(&self) -> Vec<VacuumStats> {
        let mut v: Vec<_> = self.lock().values().cloned().collect();
        v.sort_by(|a, b| (a.schema.as_str(), a.table.as_str()).cmp(&(b.schema.as_str(), b.table.as_str())));
        v
    }

    /// Run a manual `VACUUM <table>`. PG raises an error on unknown table;
    /// we mirror that.
    pub fn vacuum(&self, schema: &str, table: &str) -> Result<(), String> {
        let mut map = self.lock();
        let entry = map
            .get_mut(&(schema.into(), table.into()))
            .ok_or_else(|| format!("relation \"{schema}.{table}\" does not exist"))?;
        entry.apply_manual_vacuum(SystemTime::now());
        Ok(())
    }

    /// Run an autovacuum sweep. Returns the set of tables actually
    /// vacuumed this cycle. Mirrors `do_autovacuum` in `autovacuum.c`.
    pub fn autovacuum_cycle(&self) -> Vec<(String, String)> {
        let settings = *self.settings.lock().unwrap();
        let mut map = self.lock();
        let now = SystemTime::now();
        let mut touched = Vec::new();
        for ((sch, tbl), stats) in map.iter_mut() {
            if stats.needs_autovacuum(&settings) || stats.needs_anti_wraparound(&settings) {
                stats.apply_autovacuum(now);
                touched.push((sch.clone(), tbl.clone()));
            }
        }
        touched.sort();
        touched
    }

    /// `pg_stat_user_tables` materialisation — what the portal renders.
    pub fn user_tables_view(&self) -> Vec<VacuumStats> {
        self.all()
            .into_iter()
            .filter(|s| s.schema != "pg_catalog" && s.schema != "information_schema")
            .collect()
    }
}

/// Convenience: how long ago was the last vacuum? `Duration::MAX` if never.
pub fn time_since(opt: Option<SystemTime>, now: SystemTime) -> Duration {
    match opt {
        None => Duration::MAX,
        Some(t) => now.duration_since(t).unwrap_or(Duration::ZERO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vacuum_stats_record_insert_increments_live() {
        let mut s = VacuumStats::new("public", "users");
        s.record_insert();
        s.record_insert();
        assert_eq!(s.n_live_tup, 2);
        assert_eq!(s.n_dead_tup, 0);
    }

    #[test]
    fn test_vacuum_stats_record_delete_moves_live_to_dead() {
        let mut s = VacuumStats::new("public", "users");
        s.record_insert();
        s.record_delete();
        assert_eq!(s.n_live_tup, 0);
        assert_eq!(s.n_dead_tup, 1);
    }

    #[test]
    fn test_vacuum_stats_record_update_increments_dead_only() {
        let mut s = VacuumStats::new("public", "users");
        s.record_insert();
        s.record_update();
        assert_eq!(s.n_live_tup, 1);
        assert_eq!(s.n_dead_tup, 1);
    }

    #[test]
    fn test_needs_autovacuum_uses_threshold_plus_scale_factor() {
        // PG defaults: 50 + 0.2 * live. With live=100 → 70 dead trigger.
        let mut s = VacuumStats::new("public", "t");
        s.n_live_tup = 100;
        s.n_dead_tup = 69;
        assert!(!s.needs_autovacuum(&AutovacuumSettings::defaults()));
        s.n_dead_tup = 71;
        assert!(s.needs_autovacuum(&AutovacuumSettings::defaults()));
    }

    #[test]
    fn test_needs_anti_wraparound_at_95_percent() {
        let settings = AutovacuumSettings::defaults();
        let mut s = VacuumStats::new("public", "t");
        s.wraparound_age = (settings.freeze_max_age as f64 * 0.94) as u64;
        assert!(!s.needs_anti_wraparound(&settings));
        s.wraparound_age = (settings.freeze_max_age as f64 * 0.96) as u64;
        assert!(s.needs_anti_wraparound(&settings));
    }

    #[test]
    fn test_apply_manual_vacuum_clears_dead_and_age() {
        let mut s = VacuumStats::new("public", "t");
        s.n_dead_tup = 5_000;
        s.wraparound_age = 100_000_000;
        s.apply_manual_vacuum(SystemTime::UNIX_EPOCH);
        assert_eq!(s.n_dead_tup, 0);
        assert_eq!(s.wraparound_age, 0);
        assert_eq!(s.vacuum_count, 1);
        assert!(s.last_vacuum.is_some());
    }

    #[test]
    fn test_apply_autovacuum_clears_state_and_increments_count() {
        let mut s = VacuumStats::new("public", "t");
        s.n_dead_tup = 100;
        s.apply_autovacuum(SystemTime::UNIX_EPOCH);
        assert_eq!(s.n_dead_tup, 0);
        assert_eq!(s.autovacuum_count, 1);
        assert!(s.last_autovacuum.is_some());
    }

    #[test]
    fn test_registry_vacuum_unknown_relation_errors() {
        let r = VacuumRegistry::new();
        let err = r.vacuum("public", "missing").unwrap_err();
        assert!(err.contains("does not exist"));
    }

    #[test]
    fn test_registry_vacuum_clears_dead_tuples() {
        let r = VacuumRegistry::new();
        r.record_insert("public", "users");
        r.record_delete("public", "users");
        r.vacuum("public", "users").unwrap();
        let s = r.stats("public", "users").unwrap();
        assert_eq!(s.n_dead_tup, 0);
        assert_eq!(s.vacuum_count, 1);
    }

    #[test]
    fn test_autovacuum_cycle_vacuums_only_qualifying_tables() {
        let r = VacuumRegistry::new();
        // 'hot' has many dead tuples → should fire.
        for _ in 0..200 {
            r.record_insert("public", "hot");
        }
        for _ in 0..150 {
            r.record_delete("public", "hot");
        }
        // 'cold' has none → should not fire.
        r.record_insert("public", "cold");
        let touched = r.autovacuum_cycle();
        assert_eq!(touched, vec![("public".into(), "hot".into())]);
    }

    #[test]
    fn test_autovacuum_cycle_fires_on_anti_wraparound_even_when_few_dead() {
        let r = VacuumRegistry::new();
        r.record_insert("public", "frozen");
        let settings = AutovacuumSettings::defaults();
        r.age("public", "frozen", (settings.freeze_max_age as f64 * 0.96) as u64);
        let touched = r.autovacuum_cycle();
        assert_eq!(touched, vec![("public".into(), "frozen".into())]);
    }

    #[test]
    fn test_user_tables_view_excludes_pg_catalog() {
        let r = VacuumRegistry::new();
        r.record_insert("public", "users");
        r.record_insert("pg_catalog", "pg_class");
        r.record_insert("information_schema", "tables");
        let v = r.user_tables_view();
        let names: Vec<&str> = v.iter().map(|s| s.schema.as_str()).collect();
        assert_eq!(names, vec!["public"]);
    }

    #[test]
    fn test_settings_defaults_match_postgresql_conf_sample() {
        let s = AutovacuumSettings::defaults();
        assert_eq!(s.vacuum_threshold, 50);
        assert!((s.vacuum_scale_factor - 0.2).abs() < f64::EPSILON);
        assert_eq!(s.freeze_max_age, 200_000_000);
    }

    #[test]
    fn test_time_since_returns_max_for_never_vacuumed() {
        assert_eq!(time_since(None, SystemTime::UNIX_EPOCH), Duration::MAX);
    }

    #[test]
    fn test_registry_age_saturates_and_does_not_panic() {
        let r = VacuumRegistry::new();
        r.record_insert("public", "t");
        r.age("public", "t", u64::MAX);
        r.age("public", "t", u64::MAX);
        let s = r.stats("public", "t").unwrap();
        assert_eq!(s.wraparound_age, u64::MAX);
    }

    #[test]
    fn test_registry_all_sorted_by_schema_then_table() {
        let r = VacuumRegistry::new();
        r.record_insert("z", "b");
        r.record_insert("a", "z");
        r.record_insert("a", "a");
        let all = r.all();
        let keys: Vec<(String, String)> =
            all.iter().map(|s| (s.schema.clone(), s.table.clone())).collect();
        assert_eq!(
            keys,
            vec![
                ("a".into(), "a".into()),
                ("a".into(), "z".into()),
                ("z".into(), "b".into()),
            ]
        );
    }
}
