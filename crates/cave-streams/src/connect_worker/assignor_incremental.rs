// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/runtime/src/main/java/org/apache/kafka/connect/runtime/distributed/IncrementalCooperativeAssignor.java
//   connect/runtime/src/main/java/org/apache/kafka/connect/runtime/distributed/ConnectProtocol.java

//! Incremental cooperative assignor for Kafka Connect (KIP-415).
//!
//! Connect's own incremental rebalance is distinct from the consumer
//! `CooperativeStickyAssignor` (KIP-429). Both share the same idea —
//! revoke partitions/tasks once, then assign new ones — but the
//! Connect variant is keyed by `(connector, task_index)` pairs and
//! handles the scheduled-rebalance-delay window during which a
//! worker that briefly disappears keeps its previous tasks.
//!
//! ## Algorithm (cf. upstream `performTaskAssignment`)
//!
//! 1. **Inventory.** Compute the *desired* set of connectors+tasks
//!    across all live workers, and the *previously assigned* set per
//!    worker from the prior generation.
//! 2. **Revocations.** Any task assigned to a worker whose
//!    deterministic owner has changed is revoked. The worker is told
//!    to stop it before the next generation runs.
//! 3. **New assignments.** New tasks (created or revoked) are split
//!    across workers using rendezvous-hash. The "scheduled rebalance
//!    delay" budget (defaults to 5s in upstream, exposed here as a
//!    `delay_ms` knob) deliberately leaves a fraction of these
//!    pending for the next generation — preventing a thundering-herd
//!    move when a worker rejoins shortly after leaving.
//! 4. **Balance pass.** Workers whose load exceeds the mean by more
//!    than 1 trade one task to underloaded workers, until imbalance
//!    is ≤ 1.

use std::collections::{BTreeMap, BTreeSet};

use super::distributed_herder::MemberId;

/// Identifier for a Connect assignment unit. Connectors and tasks
/// both move under the same scheme.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssignmentUnit {
    /// Connector — the worker hosts the Connector class, picking up
    /// task-config publication duty.
    Connector(String),
    /// Task — the worker runs `<connector>:<task>`.
    Task { connector: String, task: u32 },
}

impl AssignmentUnit {
    /// Stable string-form used in the rendezvous-hash + delta
    /// reports.
    pub fn key(&self) -> String {
        match self {
            Self::Connector(c) => format!("connector::{c}"),
            Self::Task { connector, task } => format!("task::{connector}::{task}"),
        }
    }
}

/// One worker's previous-generation assignment — fed in by the
/// herder so the assignor can detect revocations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviousAssignment {
    pub units: BTreeSet<AssignmentUnit>,
}

impl PreviousAssignment {
    pub fn new(units: impl IntoIterator<Item = AssignmentUnit>) -> Self {
        Self {
            units: units.into_iter().collect(),
        }
    }
}

/// Result handed back to each worker after the incremental compute.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IncrementalConnectorAssignment {
    pub revoked: BTreeSet<AssignmentUnit>,
    pub assigned: BTreeSet<AssignmentUnit>,
    /// What the worker should hold *after* the revoke + assign
    /// (= previous − revoked + assigned).
    pub final_set: BTreeSet<AssignmentUnit>,
}

/// Full delta — one entry per worker that participated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectAssignmentDelta {
    pub per_worker: BTreeMap<MemberId, IncrementalConnectorAssignment>,
    /// Units left unassigned this generation (because of the
    /// scheduled-rebalance-delay budget). They land in the *next*
    /// generation if no worker rejoins to claim them.
    pub deferred: BTreeSet<AssignmentUnit>,
    /// Generation we just finished assigning.
    pub generation: u64,
}

pub struct IncrementalConnectAssignor {
    /// 5_000ms default mirrors upstream's
    /// `scheduled.rebalance.max.delay.ms`.
    pub delay_ms: u64,
    /// Generation counter — bumped on each `assign` call.
    generation: u64,
    /// Per-worker assignments from the previous generation.
    previous: BTreeMap<MemberId, PreviousAssignment>,
}

impl Default for IncrementalConnectAssignor {
    fn default() -> Self {
        Self {
            delay_ms: 5_000,
            generation: 0,
            previous: BTreeMap::new(),
        }
    }
}

impl IncrementalConnectAssignor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_delay_ms(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Seed previous-generation assignments — used after a restart
    /// or by tests to inject specific state.
    pub fn seed_previous(
        &mut self,
        previous: BTreeMap<MemberId, PreviousAssignment>,
    ) {
        self.previous = previous;
    }

    /// Run one incremental compute. Pure — does not mutate worker
    /// state, only the internal `previous` snapshot.
    ///
    /// * `members` — alive workers.
    /// * `desired` — every connector + task the cluster should be
    ///   running this generation.
    /// * `scheduled_delay_used_ms` — if a worker just left and we
    ///   are still within `delay_ms`, the units it owned are *not*
    ///   reassigned (held in `deferred`). Caller computes the
    ///   elapsed budget; the assignor honours it.
    pub fn assign(
        &mut self,
        members: &[MemberId],
        desired: &BTreeSet<AssignmentUnit>,
        scheduled_delay_used_ms: u64,
    ) -> ConnectAssignmentDelta {
        self.generation = self.generation.saturating_add(1);

        if members.is_empty() {
            return ConnectAssignmentDelta {
                per_worker: BTreeMap::new(),
                deferred: desired.clone(),
                generation: self.generation,
            };
        }

        let prev_owner: BTreeMap<&AssignmentUnit, &MemberId> = self
            .previous
            .iter()
            .flat_map(|(m, p)| p.units.iter().map(move |u| (u, m)))
            .collect();
        let alive: BTreeSet<&MemberId> = members.iter().collect();

        // Per-worker final + revoked + assigned accumulators.
        let mut final_per: BTreeMap<MemberId, BTreeSet<AssignmentUnit>> = BTreeMap::new();
        let mut revoked_per: BTreeMap<MemberId, BTreeSet<AssignmentUnit>> = BTreeMap::new();
        let mut assigned_per: BTreeMap<MemberId, BTreeSet<AssignmentUnit>> = BTreeMap::new();
        for m in members {
            final_per.entry(m.clone()).or_default();
            revoked_per.entry(m.clone()).or_default();
            assigned_per.entry(m.clone()).or_default();
        }

        // Carry-forward sticky assignments: a unit whose previous
        // owner is still alive AND still rendezvous-picks them
        // stays.
        let mut deferred: BTreeSet<AssignmentUnit> = BTreeSet::new();
        let mut to_place: Vec<AssignmentUnit> = Vec::new();
        for unit in desired {
            let picked = rendezvous_pick(members, unit);
            let p_owner = prev_owner.get(unit).copied();
            match (p_owner, picked) {
                (Some(prev), Some(new)) if prev == &new && alive.contains(prev) => {
                    final_per.entry(prev.clone()).or_default().insert(unit.clone());
                }
                (Some(prev), Some(_new)) if alive.contains(prev) => {
                    // Owner alive but rendezvous moved → revoke from
                    // prev, queue for assignment.
                    revoked_per
                        .entry(prev.clone())
                        .or_default()
                        .insert(unit.clone());
                    to_place.push(unit.clone());
                }
                (Some(prev), _) if !alive.contains(prev) => {
                    // Previous owner is gone. Defer-during-delay
                    // budget: hold for the rest of the window.
                    if scheduled_delay_used_ms < self.delay_ms {
                        deferred.insert(unit.clone());
                    } else {
                        to_place.push(unit.clone());
                    }
                }
                (None, _) => {
                    // New unit — queue.
                    to_place.push(unit.clone());
                }
                _ => {
                    to_place.push(unit.clone());
                }
            }
        }

        // Place every unit in to_place via rendezvous-hash.
        for unit in to_place {
            if let Some(owner) = rendezvous_pick(members, &unit) {
                assigned_per.entry(owner.clone()).or_default().insert(unit.clone());
                final_per.entry(owner).or_default().insert(unit);
            }
        }

        // Balance pass: if any worker's load exceeds the mean by
        // more than 1, donate to the most-underloaded worker.
        // Bounded loops because each donation reduces total
        // imbalance by 2.
        loop {
            let load: BTreeMap<MemberId, usize> = members
                .iter()
                .map(|m| (m.clone(), final_per.get(m).map(|s| s.len()).unwrap_or(0)))
                .collect();
            let max = load.iter().max_by_key(|(_, v)| **v).map(|(m, v)| (m.clone(), *v));
            let min = load.iter().min_by_key(|(_, v)| **v).map(|(m, v)| (m.clone(), *v));
            match (max, min) {
                (Some((hi, h)), Some((lo, l))) if h > l + 1 && hi != lo => {
                    // Donate the alphabetically-first unit.
                    let pick = final_per
                        .get(&hi)
                        .and_then(|s| s.iter().next().cloned());
                    if let Some(u) = pick {
                        final_per.entry(hi.clone()).and_modify(|s| {
                            s.remove(&u);
                        });
                        // If `hi` had freshly received this unit in
                        // the placement pass, the donation undoes
                        // that — remove from assigned_per[hi] rather
                        // than recording it as both assigned+revoked.
                        let was_freshly_assigned = assigned_per
                            .get_mut(&hi)
                            .map(|s| s.remove(&u))
                            .unwrap_or(false);
                        if !was_freshly_assigned {
                            // Otherwise the unit was carried-forward
                            // sticky from the previous generation;
                            // record an honest revocation.
                            revoked_per.entry(hi.clone()).or_default().insert(u.clone());
                        }
                        assigned_per.entry(lo.clone()).or_default().insert(u.clone());
                        final_per.entry(lo).or_default().insert(u);
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }

        // Build per-worker reports + update previous-generation
        // snapshot.
        let mut per_worker = BTreeMap::new();
        let mut new_previous = BTreeMap::new();
        for m in members {
            let revoked = revoked_per.remove(m).unwrap_or_default();
            let assigned = assigned_per.remove(m).unwrap_or_default();
            let final_set = final_per.remove(m).unwrap_or_default();
            new_previous.insert(
                m.clone(),
                PreviousAssignment {
                    units: final_set.clone(),
                },
            );
            per_worker.insert(
                m.clone(),
                IncrementalConnectorAssignment {
                    revoked,
                    assigned,
                    final_set,
                },
            );
        }
        self.previous = new_previous;

        ConnectAssignmentDelta {
            per_worker,
            deferred,
            generation: self.generation,
        }
    }
}

fn rendezvous_pick(members: &[MemberId], unit: &AssignmentUnit) -> Option<MemberId> {
    members
        .iter()
        .map(|m| (score(m, unit), m))
        .max_by_key(|(s, _)| *s)
        .map(|(_, m)| m.clone())
}

fn score(m: &MemberId, unit: &AssignmentUnit) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    m.as_str().hash(&mut h);
    unit.key().hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(c: &str, t: u32) -> AssignmentUnit {
        AssignmentUnit::Task {
            connector: c.into(),
            task: t,
        }
    }
    fn conn(c: &str) -> AssignmentUnit {
        AssignmentUnit::Connector(c.into())
    }
    fn ms(strs: &[&str]) -> Vec<MemberId> {
        strs.iter().map(|s| MemberId::from(*s)).collect()
    }
    fn set(units: Vec<AssignmentUnit>) -> BTreeSet<AssignmentUnit> {
        units.into_iter().collect()
    }

    #[test]
    fn empty_members_defers_everything() {
        let mut a = IncrementalConnectAssignor::new();
        let delta = a.assign(&[], &set(vec![task("c", 0)]), 0);
        assert_eq!(delta.deferred.len(), 1);
        assert!(delta.per_worker.is_empty());
        assert_eq!(delta.generation, 1);
    }

    #[test]
    fn first_assignment_distributes_tasks() {
        let mut a = IncrementalConnectAssignor::new();
        let desired = set(vec![task("c", 0), task("c", 1), task("c", 2), task("c", 3)]);
        let delta = a.assign(&ms(&["w1", "w2"]), &desired, 0);
        let w1 = &delta.per_worker[&MemberId::from("w1")];
        let w2 = &delta.per_worker[&MemberId::from("w2")];
        assert_eq!(w1.final_set.len() + w2.final_set.len(), 4);
        assert!(!w1.final_set.is_empty());
        assert!(!w2.final_set.is_empty());
        // First gen → no revokes (no previous).
        assert!(w1.revoked.is_empty());
        assert!(w2.revoked.is_empty());
    }

    #[test]
    fn sticky_keeps_unmoved_owner() {
        let mut a = IncrementalConnectAssignor::new();
        let desired = set(vec![task("c", 0), task("c", 1), task("c", 2), task("c", 3)]);
        // Round 1.
        let d1 = a.assign(&ms(&["w1", "w2"]), &desired, 0);
        // Round 2 with same desired+members — no revokes.
        let d2 = a.assign(&ms(&["w1", "w2"]), &desired, 0);
        for (_, rep) in &d2.per_worker {
            assert!(rep.revoked.is_empty(), "stable round should not revoke");
            assert!(rep.assigned.is_empty(), "stable round should not assign");
        }
        // Generation counter incremented.
        assert_eq!(d2.generation, d1.generation + 1);
    }

    #[test]
    fn worker_leave_within_delay_window_holds_units_deferred() {
        let mut a = IncrementalConnectAssignor::new().with_delay_ms(5_000);
        let desired = set(vec![task("c", 0), task("c", 1), task("c", 2), task("c", 3)]);
        // Round 1 with w1+w2.
        a.assign(&ms(&["w1", "w2"]), &desired, 0);
        // Round 2: w2 drops, but we're still in the delay window
        // (used=0ms).
        let d2 = a.assign(&ms(&["w1"]), &desired, 0);
        // w1 should keep what it had; w2's tasks should be deferred.
        let w1 = &d2.per_worker[&MemberId::from("w1")];
        // At least some of the desired set should be in deferred.
        assert!(
            !d2.deferred.is_empty(),
            "expected some deferred during delay window; got {:?}",
            d2
        );
        assert!(w1.final_set.len() < 4, "w1 must not yet pick up everything");
    }

    #[test]
    fn worker_leave_after_delay_window_redistributes() {
        let mut a = IncrementalConnectAssignor::new().with_delay_ms(5_000);
        let desired = set(vec![task("c", 0), task("c", 1), task("c", 2), task("c", 3)]);
        a.assign(&ms(&["w1", "w2"]), &desired, 0);
        // Delay window already elapsed.
        let d2 = a.assign(&ms(&["w1"]), &desired, 10_000);
        assert!(d2.deferred.is_empty(), "after delay everything must be reassigned");
        let w1 = &d2.per_worker[&MemberId::from("w1")];
        assert_eq!(w1.final_set.len(), 4);
    }

    #[test]
    fn worker_join_revokes_from_overloaded_owner() {
        let mut a = IncrementalConnectAssignor::new();
        let desired = set(vec![task("c", 0), task("c", 1), task("c", 2), task("c", 3)]);
        a.assign(&ms(&["w1"]), &desired, 0);
        let d2 = a.assign(&ms(&["w1", "w2"]), &desired, 0);
        let w1 = &d2.per_worker[&MemberId::from("w1")];
        let w2 = &d2.per_worker[&MemberId::from("w2")];
        // Some tasks must have moved.
        assert!(!w1.revoked.is_empty() || !w2.assigned.is_empty());
        // Final sets must cover everything together.
        let total = w1.final_set.len() + w2.final_set.len();
        assert_eq!(total, 4);
    }

    #[test]
    fn connector_vs_task_keys_are_distinct() {
        let a = AssignmentUnit::Connector("c".into());
        let b = AssignmentUnit::Task {
            connector: "c".into(),
            task: 0,
        };
        assert_ne!(a.key(), b.key());
    }

    #[test]
    fn generation_advances_per_assign_call() {
        let mut a = IncrementalConnectAssignor::new();
        assert_eq!(a.generation(), 0);
        a.assign(&ms(&["w1"]), &set(vec![]), 0);
        assert_eq!(a.generation(), 1);
        a.assign(&ms(&["w1"]), &set(vec![]), 0);
        assert_eq!(a.generation(), 2);
    }

    #[test]
    fn seed_previous_lets_resume_after_restart() {
        let mut a = IncrementalConnectAssignor::new();
        let mut seed = BTreeMap::new();
        seed.insert(
            MemberId::from("w1"),
            PreviousAssignment::new(vec![task("c", 0)]),
        );
        a.seed_previous(seed);
        // Same task desired again → sticky retention.
        let d = a.assign(&ms(&["w1"]), &set(vec![task("c", 0)]), 0);
        let w1 = &d.per_worker[&MemberId::from("w1")];
        assert!(w1.revoked.is_empty());
        assert!(w1.assigned.is_empty(), "sticky → no fresh assign");
        assert_eq!(w1.final_set.len(), 1);
    }

    #[test]
    fn connector_units_assigned_alongside_tasks() {
        let mut a = IncrementalConnectAssignor::new();
        let desired = set(vec![conn("c"), task("c", 0), task("c", 1)]);
        let d = a.assign(&ms(&["w1", "w2"]), &desired, 0);
        let total: usize = d.per_worker.values().map(|r| r.final_set.len()).sum();
        assert_eq!(total, 3);
    }

    #[test]
    fn balance_pass_keeps_imbalance_within_one() {
        let mut a = IncrementalConnectAssignor::new();
        let desired = set((0..10).map(|i| task("c", i)).collect());
        let d = a.assign(&ms(&["w1", "w2", "w3"]), &desired, 0);
        let counts: Vec<usize> = d
            .per_worker
            .values()
            .map(|r| r.final_set.len())
            .collect();
        let max = *counts.iter().max().unwrap();
        let min = *counts.iter().min().unwrap();
        assert!(max - min <= 1, "expected imbalance ≤1, got {counts:?}");
    }

    #[test]
    fn revoke_unit_does_not_double_count_to_final() {
        let mut a = IncrementalConnectAssignor::new();
        let desired = set(vec![task("c", 0), task("c", 1), task("c", 2)]);
        a.assign(&ms(&["w1"]), &desired, 0);
        let d = a.assign(&ms(&["w1", "w2"]), &desired, 0);
        for (_, rep) in &d.per_worker {
            for u in &rep.revoked {
                assert!(
                    !rep.final_set.contains(u),
                    "revoked unit must not appear in final_set"
                );
            }
        }
    }
}
