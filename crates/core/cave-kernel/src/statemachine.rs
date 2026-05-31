// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Generic finite state-machine primitive.
//!
//! A first-party building block for any cave-* module that models a
//! workflow as discrete states and events: cave-workflows (Argo-style
//! step phases), cave-rollouts (canary/blue-green progression),
//! cave-deploy (sync lifecycle), and cave-pipelines (job stages) all
//! reinvent the same "given current state + an event, look up the next
//! state, maybe gated by a guard, and record what happened" loop.
//!
//! Shape:
//!   - [`TransitionTable<S, E>`] — declares legal `(from, event) -> to`
//!     edges, with an optional guard closure that can veto an edge at
//!     fire time (e.g. "only advance if quota remains").
//!   - [`StateMachine<S, E>`] — holds a current state, fires events
//!     against the table via [`StateMachine::fire`], and records an
//!     ordered [`history`](StateMachine::history) of `(from, event, to)`
//!     triples for audit / debugging / replay.
//!   - [`WorkflowStore`] — trait for persisting a machine's current
//!     state by id, with an in-memory [`MemStore`] implementation. The
//!     trait lets adopters swap in etcd / RDBMS / docdb backends later
//!     without touching caller code.
//!
//! `S` and `E` are required to be `Clone + Eq + Hash` so they can key the
//! transition table and be cheaply snapshotted into history. They are
//! typically small `#[derive(Clone, PartialEq, Eq, Hash)]` enums.

use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// A guard closure: given the current state and the event being fired,
/// return `true` to allow the transition or `false` to veto it. Guards
/// must be pure side-effect-free predicates — they may be invoked
/// speculatively and should not mutate external state.
type Guard<S, E> = Arc<dyn Fn(&S, &E) -> bool + Send + Sync>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransitionError<S, E>
where
    S: Debug,
    E: Debug,
{
    /// No `(from, event)` edge is declared in the table.
    #[error("no transition from {from:?} on event {event:?}")]
    NoTransition { from: S, event: E },
    /// An edge exists but its guard returned `false`.
    #[error("transition from {from:?} on event {event:?} blocked by guard")]
    GuardBlocked { from: S, event: E },
}

struct Edge<S, E> {
    to: S,
    guard: Option<Guard<S, E>>,
}

/// Declarative table of legal transitions. Build it once, then hand it
/// to one or more [`StateMachine`]s. Cheap to clone — guards are shared
/// behind `Arc`.
pub struct TransitionTable<S, E>
where
    S: Clone + Eq + Hash,
    E: Clone + Eq + Hash,
{
    edges: HashMap<(S, E), Edge<S, E>>,
}

impl<S, E> Default for TransitionTable<S, E>
where
    S: Clone + Eq + Hash,
    E: Clone + Eq + Hash,
{
    fn default() -> Self {
        Self {
            edges: HashMap::new(),
        }
    }
}

impl<S, E> Clone for TransitionTable<S, E>
where
    S: Clone + Eq + Hash,
    E: Clone + Eq + Hash,
{
    fn clone(&self) -> Self {
        let edges = self
            .edges
            .iter()
            .map(|((from, ev), edge)| {
                (
                    (from.clone(), ev.clone()),
                    Edge {
                        to: edge.to.clone(),
                        guard: edge.guard.clone(),
                    },
                )
            })
            .collect();
        Self { edges }
    }
}

impl<S, E> TransitionTable<S, E>
where
    S: Clone + Eq + Hash,
    E: Clone + Eq + Hash,
{
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare an unguarded edge `from --event--> to`. A later `add` /
    /// `add_guarded` for the same `(from, event)` key overwrites the
    /// earlier one (last-writer-wins), so a table is deterministic.
    pub fn add(mut self, from: S, on_event: E, to: S) -> Self {
        self.edges
            .insert((from, on_event), Edge { to, guard: None });
        self
    }

    /// Declare a guarded edge. The transition only fires when `guard`
    /// returns `true` for the current state + event; otherwise
    /// [`StateMachine::fire`] returns [`TransitionError::GuardBlocked`]
    /// and the current state is left unchanged.
    pub fn add_guarded<G>(mut self, from: S, on_event: E, to: S, guard: G) -> Self
    where
        G: Fn(&S, &E) -> bool + Send + Sync + 'static,
    {
        self.edges.insert(
            (from, on_event),
            Edge {
                to,
                guard: Some(Arc::new(guard)),
            },
        );
        self
    }

    /// Number of declared edges.
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }
}

/// A running state machine: a current state plus the table that governs
/// its transitions and the history of everything that has fired.
pub struct StateMachine<S, E>
where
    S: Clone + Eq + Hash,
    E: Clone + Eq + Hash,
{
    current: S,
    table: TransitionTable<S, E>,
    history: Vec<(S, E, S)>,
}

impl<S, E> StateMachine<S, E>
where
    S: Clone + Eq + Hash,
    E: Clone + Eq + Hash,
{
    /// Construct a machine starting in `initial`, governed by `table`.
    pub fn new(initial: S, table: TransitionTable<S, E>) -> Self {
        Self {
            current: initial,
            table,
            history: Vec::new(),
        }
    }

    /// The current state.
    pub fn state(&self) -> &S {
        &self.current
    }

    /// Ordered log of `(from, event, to)` for every successful fire.
    pub fn history(&self) -> &[(S, E, S)] {
        &self.history
    }

    /// Whether `(current, event)` has a declared edge whose guard (if
    /// any) currently permits it. A `true` result means [`fire`] with
    /// the same event would succeed.
    ///
    /// [`fire`]: StateMachine::fire
    pub fn can_fire(&self, event: &E) -> bool {
        match self.table.edges.get(&(self.current.clone(), event.clone())) {
            Some(edge) => edge
                .guard
                .as_ref()
                .map(|g| g(&self.current, event))
                .unwrap_or(true),
            None => false,
        }
    }

    /// Advance the machine by firing `event`. On success the current
    /// state moves to the edge's target, the `(from, event, to)` triple
    /// is appended to [`history`](StateMachine::history), and the new
    /// state is returned. On failure the current state and history are
    /// left untouched.
    pub fn fire(&mut self, event: E) -> Result<S, TransitionError<S, E>>
    where
        S: Debug,
        E: Debug,
    {
        let key = (self.current.clone(), event.clone());
        let edge = match self.table.edges.get(&key) {
            Some(edge) => edge,
            None => {
                return Err(TransitionError::NoTransition {
                    from: self.current.clone(),
                    event,
                });
            }
        };
        if let Some(guard) = &edge.guard {
            if !guard(&self.current, &event) {
                return Err(TransitionError::GuardBlocked {
                    from: self.current.clone(),
                    event,
                });
            }
        }
        let to = edge.to.clone();
        let from = std::mem::replace(&mut self.current, to.clone());
        self.history.push((from, event, to.clone()));
        Ok(to)
    }
}

/// Persistence backend for state-machine state, keyed by an opaque id.
/// Adopters implement this over their storage of choice; the in-memory
/// [`MemStore`] is provided for single-node use and tests.
pub trait WorkflowStore<S> {
    /// Persist `state` for `id`, overwriting any prior value.
    fn save(&self, id: &str, state: S);
    /// Load the last-saved state for `id`, or `None` if unknown.
    fn load(&self, id: &str) -> Option<S>;
}

/// In-memory [`WorkflowStore`]. Cheap to clone — backed by
/// `Arc<RwLock<HashMap>>`, so clones share one map.
pub struct MemStore<S> {
    inner: Arc<RwLock<HashMap<String, S>>>,
}

impl<S> Default for MemStore<S> {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl<S> Clone for MemStore<S> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S> MemStore<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of persisted ids.
    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().unwrap().is_empty()
    }
}

impl<S: Clone> WorkflowStore<S> for MemStore<S> {
    fn save(&self, id: &str, state: S) {
        self.inner.write().unwrap().insert(id.to_string(), state);
    }

    fn load(&self, id: &str) -> Option<S> {
        self.inner.read().unwrap().get(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    enum St {
        Pending,
        Running,
        Succeeded,
        Failed,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    enum Ev {
        Start,
        Complete,
        Fail,
    }

    fn table() -> TransitionTable<St, Ev> {
        TransitionTable::new()
            .add(St::Pending, Ev::Start, St::Running)
            .add(St::Running, Ev::Complete, St::Succeeded)
            .add(St::Running, Ev::Fail, St::Failed)
    }

    #[test]
    fn valid_transition_advances_state() {
        let mut sm = StateMachine::new(St::Pending, table());
        let next = sm.fire(Ev::Start).unwrap();
        assert_eq!(next, St::Running);
        assert_eq!(sm.state(), &St::Running);
    }

    #[test]
    fn valid_transition_chain_reaches_terminal() {
        let mut sm = StateMachine::new(St::Pending, table());
        sm.fire(Ev::Start).unwrap();
        let last = sm.fire(Ev::Complete).unwrap();
        assert_eq!(last, St::Succeeded);
        assert_eq!(sm.state(), &St::Succeeded);
    }

    #[test]
    fn invalid_transition_returns_no_transition_error() {
        // Cannot Complete while still Pending.
        let mut sm = StateMachine::new(St::Pending, table());
        let err = sm.fire(Ev::Complete).unwrap_err();
        assert_eq!(
            err,
            TransitionError::NoTransition {
                from: St::Pending,
                event: Ev::Complete,
            }
        );
        // State is unchanged after a failed fire.
        assert_eq!(sm.state(), &St::Pending);
    }

    #[test]
    fn invalid_transition_does_not_record_history() {
        let mut sm = StateMachine::new(St::Pending, table());
        let _ = sm.fire(Ev::Fail);
        assert!(sm.history().is_empty());
    }

    #[test]
    fn guard_blocks_transition_when_predicate_false() {
        // Guard vetoes Start unless some external condition holds; here
        // it always returns false to prove the veto path.
        let t = TransitionTable::new().add_guarded(
            St::Pending,
            Ev::Start,
            St::Running,
            |_from, _ev| false,
        );
        let mut sm = StateMachine::new(St::Pending, t);
        let err = sm.fire(Ev::Start).unwrap_err();
        assert_eq!(
            err,
            TransitionError::GuardBlocked {
                from: St::Pending,
                event: Ev::Start,
            }
        );
        assert_eq!(sm.state(), &St::Pending);
        assert!(sm.history().is_empty());
    }

    #[test]
    fn guard_allows_transition_when_predicate_true() {
        let t = TransitionTable::new().add_guarded(
            St::Pending,
            Ev::Start,
            St::Running,
            |from, ev| *from == St::Pending && *ev == Ev::Start,
        );
        let mut sm = StateMachine::new(St::Pending, t);
        assert!(sm.can_fire(&Ev::Start));
        assert_eq!(sm.fire(Ev::Start).unwrap(), St::Running);
    }

    #[test]
    fn can_fire_reflects_table_and_guards() {
        let t = TransitionTable::new()
            .add(St::Pending, Ev::Start, St::Running)
            .add_guarded(St::Running, Ev::Complete, St::Succeeded, |_, _| false);
        let mut sm = StateMachine::new(St::Pending, t);
        assert!(sm.can_fire(&Ev::Start));
        assert!(!sm.can_fire(&Ev::Complete)); // no edge from Pending
        sm.fire(Ev::Start).unwrap();
        assert!(!sm.can_fire(&Ev::Complete)); // edge exists but guard vetoes
    }

    #[test]
    fn history_records_each_successful_transition_in_order() {
        let mut sm = StateMachine::new(St::Pending, table());
        sm.fire(Ev::Start).unwrap();
        sm.fire(Ev::Fail).unwrap();
        assert_eq!(
            sm.history(),
            &[
                (St::Pending, Ev::Start, St::Running),
                (St::Running, Ev::Fail, St::Failed),
            ]
        );
    }

    #[test]
    fn store_roundtrip_save_then_load() {
        let store: MemStore<St> = MemStore::new();
        assert!(store.load("wf-1").is_none());
        store.save("wf-1", St::Running);
        assert_eq!(store.load("wf-1"), Some(St::Running));
        // Overwrite semantics.
        store.save("wf-1", St::Succeeded);
        assert_eq!(store.load("wf-1"), Some(St::Succeeded));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn store_clone_shares_backing_map() {
        let store: MemStore<St> = MemStore::new();
        let store2 = store.clone();
        store.save("wf-9", St::Failed);
        assert_eq!(store2.load("wf-9"), Some(St::Failed));
    }

    #[test]
    fn store_persists_machine_state_end_to_end() {
        // Drive a machine, then persist + reload its current state.
        let store: MemStore<St> = MemStore::new();
        let mut sm = StateMachine::new(St::Pending, table());
        sm.fire(Ev::Start).unwrap();
        store.save("run-42", sm.state().clone());

        let reloaded = store.load("run-42").unwrap();
        assert_eq!(reloaded, St::Running);

        // Resume a fresh machine from persisted state and continue.
        let mut resumed = StateMachine::new(reloaded, table());
        assert_eq!(resumed.fire(Ev::Complete).unwrap(), St::Succeeded);
    }

    #[test]
    fn last_writer_wins_on_duplicate_edge() {
        let t = TransitionTable::new()
            .add(St::Pending, Ev::Start, St::Failed)
            .add(St::Pending, Ev::Start, St::Running); // overwrites
        assert_eq!(t.len(), 1);
        let mut sm = StateMachine::new(St::Pending, t);
        assert_eq!(sm.fire(Ev::Start).unwrap(), St::Running);
    }

    #[test]
    fn table_is_reusable_across_machines() {
        let t = table();
        let mut a = StateMachine::new(St::Pending, t.clone());
        let mut b = StateMachine::new(St::Pending, t);
        a.fire(Ev::Start).unwrap();
        b.fire(Ev::Start).unwrap();
        assert_eq!(a.state(), &St::Running);
        assert_eq!(b.state(), &St::Running);
    }
}