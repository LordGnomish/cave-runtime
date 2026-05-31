// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

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