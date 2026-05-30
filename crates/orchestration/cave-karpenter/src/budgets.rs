// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Disruption-budget `AllowedDisruptions` math — port of the budget helpers
//! in `pkg/apis/v1/nodepool.go` from kubernetes-sigs/karpenter v1.12.1
//! (sha ed490e8).
//!
//! A NodePool's `Disruption.Budgets` cap how many nodes the disruption
//! controller may take down concurrently. Each budget's `nodes` field is an
//! IntOrString (`"3"` or `"20%"`); the effective allowance for a reason is the
//! minimum across the budgets that apply to it, or unbounded (`MaxInt32`) when
//! none constrain it.
//!
//! Ported here: the k8s intstr round-up percentage scaler
//! ([`scaled_value_from_int_or_percent`], mirroring
//! `intstr.GetScaledValueFromIntOrPercent` + `GetIntStrFromValue`),
//! [`budget_allowed_disruptions`] (`Budget.GetAllowedDisruptions`),
//! [`nodepool_allowed_disruptions_by_reason`]
//! (`NodePool.GetAllowedDisruptionsByReason`), and
//! [`must_get_allowed_disruptions`] (`MustGetAllowedDisruptions`).
//!
//! The cron-schedule `IsActive` window (`Schedule` + `Duration`) is evaluated
//! by the clock-threaded [`budget_is_active_at`] / [`budget_allowed_disruptions_at`]
//! (cont3 2026-05-30), faithfully porting `(*Budget).IsActive` over the
//! [`crate::cron`] engine and the [`crate::duration`] parser. The legacy
//! no-clock helpers ([`budget_allowed_disruptions`] etc.) only evaluate
//! always-active (no-schedule) budgets; a scheduled budget on that path still
//! returns [`BudgetError::ScheduleNotPortable`] since it has no clock to
//! evaluate against — callers with a clock should use the `_at` variants.

use crate::cron::parse_standard;
use crate::duration::parse_duration;
use crate::models::Budget;
use std::fmt;

/// Upstream returns `math.MaxInt32` for an unbounded budget allowance.
pub const UNBOUNDED_DISRUPTIONS: i64 = i32::MAX as i64;

/// Errors surfaced while computing budget allowances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetError {
    /// `nodes` was neither a valid integer nor a valid percentage.
    InvalidIntOrPercent(String),
    /// The budget carries a cron `Schedule`/`Duration` but was evaluated on the
    /// legacy no-clock path. Use [`budget_is_active_at`] / the `_at` variants.
    ScheduleNotPortable,
    /// The budget's `Schedule` is not a valid standard cron expression.
    InvalidCron(String),
    /// The budget's `Duration` is not a valid Go duration string.
    InvalidDuration(String),
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BudgetError::InvalidIntOrPercent(s) => {
                write!(f, "invalid int-or-percent budget value: {s:?}")
            }
            BudgetError::ScheduleNotPortable => f.write_str(
                "scheduled budget needs a clock; use budget_is_active_at / the _at variants",
            ),
            BudgetError::InvalidCron(s) => write!(f, "invalid cron schedule: {s:?}"),
            BudgetError::InvalidDuration(s) => write!(f, "invalid budget duration: {s:?}"),
        }
    }
}

impl std::error::Error for BudgetError {}

/// Resolve a budget `nodes` IntOrString against `total`, mirroring
/// `intstr.GetScaledValueFromIntOrPercent` composed with `GetIntStrFromValue`:
///   * a bare integer (`"3"`) passes through unchanged, ignoring `total`;
///   * a percentage (`"20%"`) yields `ceil`/`floor` of `pct * total / 100`
///     depending on `round_up`.
///
/// Negative integers/percentages and malformed strings are rejected (upstream
/// relies on prior nodepool validation; we fail closed rather than panic).
pub fn scaled_value_from_int_or_percent(
    spec: &str,
    total: usize,
    round_up: bool,
) -> Result<usize, BudgetError> {
    let invalid = || BudgetError::InvalidIntOrPercent(spec.to_string());

    if let Some(pct_str) = spec.strip_suffix('%') {
        // percentage branch
        let pct: i64 = pct_str.parse().map_err(|_| invalid())?;
        if pct < 0 {
            return Err(invalid());
        }
        let scaled = pct as f64 * total as f64 / 100.0;
        let v = if round_up {
            scaled.ceil()
        } else {
            scaled.floor()
        };
        Ok(v as usize)
    } else {
        // integer branch (GetIntStrFromValue treats parseable ints as Int)
        let v: i64 = spec.parse().map_err(|_| invalid())?;
        if v < 0 {
            return Err(invalid());
        }
        Ok(v as usize)
    }
}

/// `(*Budget).GetAllowedDisruptions`: the allowance contributed by one budget.
/// Inactive budgets are unbounded (`MaxInt32`); active budgets scale their
/// `nodes` IntOrString against `num_nodes`, rounding up (matching how
/// Kubernetes handles `MaxUnavailable` on PDBs — a disruption may slightly
/// exceed a percentage budget rather than block entirely).
pub fn budget_allowed_disruptions(budget: &Budget, num_nodes: usize) -> Result<i64, BudgetError> {
    if !budget_is_active(budget)? {
        return Ok(UNBOUNDED_DISRUPTIONS);
    }
    let v = scaled_value_from_int_or_percent(&budget.nodes, num_nodes, true)?;
    Ok(v as i64)
}

/// `(*Budget).IsActive` — no-schedule path only. A budget with neither a
/// schedule nor a duration is always active; a scheduled budget returns
/// [`BudgetError::ScheduleNotPortable`] (the no-clock path cannot evaluate it).
fn budget_is_active(budget: &Budget) -> Result<bool, BudgetError> {
    if budget.schedule.is_none() && budget.duration.is_none() {
        Ok(true)
    } else {
        Err(BudgetError::ScheduleNotPortable)
    }
}

/// `(*Budget).IsActive`: faithful clock-threaded port. A budget with neither a
/// schedule nor a duration is always active. Otherwise it walks back the
/// duration from `now_unix` (UTC seconds), asks the cron engine for the next
/// schedule hit at-or-after that checkpoint, and is active when that hit is not
/// after `now` — i.e. when the most recent hit's window still covers `now`.
pub fn budget_is_active_at(budget: &Budget, now_unix: i64) -> Result<bool, BudgetError> {
    if budget.schedule.is_none() && budget.duration.is_none() {
        return Ok(true);
    }
    // lo.FromPtr semantics: a nil schedule parses as the empty string (which
    // the cron engine rejects), a nil duration is zero.
    let schedule_str = budget.schedule.as_deref().unwrap_or("");
    let schedule = parse_standard(&format!("TZ=UTC {schedule_str}"))
        .map_err(|e| BudgetError::InvalidCron(e.to_string()))?;
    let duration_ns = match budget.duration.as_deref() {
        Some(d) => parse_duration(d).map_err(|_| BudgetError::InvalidDuration(d.to_string()))?,
        None => 0,
    };
    let duration_secs = duration_ns / 1_000_000_000;
    let checkpoint = now_unix - duration_secs;
    match schedule.next(checkpoint) {
        Some(next_hit) => Ok(next_hit <= now_unix),
        None => Ok(false),
    }
}

/// `(*Budget).GetAllowedDisruptions` with a clock: inactive budgets are
/// unbounded (`MaxInt32`); active budgets scale their `nodes` IntOrString
/// against `num_nodes`, rounding up — the clock-threaded twin of
/// [`budget_allowed_disruptions`].
pub fn budget_allowed_disruptions_at(
    budget: &Budget,
    num_nodes: usize,
    now_unix: i64,
) -> Result<i64, BudgetError> {
    if !budget_is_active_at(budget, now_unix)? {
        return Ok(UNBOUNDED_DISRUPTIONS);
    }
    let v = scaled_value_from_int_or_percent(&budget.nodes, num_nodes, true)?;
    Ok(v as i64)
}

/// `(*NodePool).GetAllowedDisruptionsByReason` with a clock — the
/// clock-threaded twin of [`nodepool_allowed_disruptions_by_reason`].
pub fn nodepool_allowed_disruptions_by_reason_at(
    budgets: &[Budget],
    num_nodes: usize,
    reason: &str,
    now_unix: i64,
) -> Result<i64, BudgetError> {
    let mut allowed = UNBOUNDED_DISRUPTIONS;
    let mut first_err: Option<BudgetError> = None;
    for budget in budgets {
        match budget_allowed_disruptions_at(budget, num_nodes, now_unix) {
            Ok(val) => {
                if budget.reasons.is_empty() || budget.reasons.iter().any(|r| r == reason) {
                    allowed = allowed.min(val);
                }
            }
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(allowed),
    }
}

/// `(*NodePool).MustGetAllowedDisruptions` with a clock — fails closed to `0`.
pub fn must_get_allowed_disruptions_at(
    budgets: &[Budget],
    num_nodes: usize,
    reason: &str,
    now_unix: i64,
) -> i64 {
    nodepool_allowed_disruptions_by_reason_at(budgets, num_nodes, reason, now_unix).unwrap_or(0)
}

/// `(*NodePool).GetAllowedDisruptionsByReason`: the minimum allowance across
/// every budget that applies to `reason`. A budget with empty `reasons`
/// applies to all reasons. Returns [`UNBOUNDED_DISRUPTIONS`] when no budget
/// constrains the reason. Errors from individual budgets are aggregated:
/// upstream collects them via multierr while still returning the running
/// minimum — we surface the first error encountered.
pub fn nodepool_allowed_disruptions_by_reason(
    budgets: &[Budget],
    num_nodes: usize,
    reason: &str,
) -> Result<i64, BudgetError> {
    let mut allowed = UNBOUNDED_DISRUPTIONS;
    let mut first_err: Option<BudgetError> = None;
    for budget in budgets {
        match budget_allowed_disruptions(budget, num_nodes) {
            Ok(val) => {
                if budget.reasons.is_empty() || budget.reasons.iter().any(|r| r == reason) {
                    allowed = allowed.min(val);
                }
            }
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(allowed),
    }
}

/// `(*NodePool).MustGetAllowedDisruptions`: like
/// [`nodepool_allowed_disruptions_by_reason`] but fails closed to `0` on any
/// error, reducing the state the disruption controller must reconcile.
pub fn must_get_allowed_disruptions(budgets: &[Budget], num_nodes: usize, reason: &str) -> i64 {
    nodepool_allowed_disruptions_by_reason(budgets, num_nodes, reason).unwrap_or(0)
}
