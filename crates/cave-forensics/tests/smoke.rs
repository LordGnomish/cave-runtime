// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-forensics smoke tests — exercise the five user-required scenarios:
//!   1. TracingPolicy CRD parse + validate
//!   2. Policy filter end-to-end against a ProcessExec event
//!   3. Enforcer Sigkill decision emission
//!   4. NDJSON + gRPC frame round-trip
//!   5. Case + evidence ingestion with chain-of-custody hash

use cave_forensics::case::CaseStore;
use cave_forensics::enforcer::Enforcer;
use cave_forensics::events::file::{FileEvent, FileOp, OpenFlags};
use cave_forensics::events::network::{ipv4, L4Proto, NetworkEvent, NetworkOp};
use cave_forensics::events::process_exec::ProcessExecEvent;
use cave_forensics::events::KernelEvent;
use cave_forensics::evidence::{verify_chain, CustodyEntry};
use cave_forensics::export::grpc_codec::{decode_events, encode_event, encode_many};
use cave_forensics::export::json_stream::{decode_ndjson, encode_ndjson};
use cave_forensics::filter::{ActionKind, FilterGroup, FilterOp, MatchAction, MatchArg, MatchBinary};
use cave_forensics::models::{EvidenceType, ForensicSeverity};
use cave_forensics::process::{cap, Credentials, Namespaces, Process};
use cave_forensics::store::{matching_groups, PolicyStore};
use cave_forensics::tracing_policy::{
    KProbeSpec, PolicyKind, PolicyMeta, TracingPolicy, TracingPolicySpec,
};
use chrono::{TimeZone, Utc};

fn ts() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn proc(pid: u32, bin: &str) -> Process {
    Process {
        exec_id: format!("e-{pid}"),
        pid,
        pid_in_ns: pid + 1,
        binary: bin.into(),
        arguments: String::new(),
        cwd: "/".into(),
        credentials: Credentials::default(),
        namespaces: Namespaces::default(),
        parent_exec_id: None,
        container_id: Some("docker://abc".into()),
        pod_name: Some("victim".into()),
        pod_namespace: Some("default".into()),
        start_time: ts(),
        end_time: None,
    }
}

#[test]
fn smoke_1_tracing_policy_parse_validate() {
    let p = TracingPolicy {
        api_version: "cilium.io/v1alpha1".into(),
        kind: PolicyKind::TracingPolicyNamespaced,
        metadata: PolicyMeta {
            name: "block-cat-shadow".into(),
            namespace: Some("default".into()),
            ..Default::default()
        },
        spec: TracingPolicySpec {
            kprobes: vec![KProbeSpec {
                call: "sys_open".into(),
                syscall: true,
                return_: false,
                args: vec![],
                selectors: vec![],
            }],
            ..Default::default()
        },
    };
    let json = serde_json::to_string(&p).unwrap();
    let back = TracingPolicy::parse_json(&json).unwrap();
    assert_eq!(back.metadata.name, "block-cat-shadow");
    assert!(back.is_namespaced());
}

#[test]
fn smoke_2_filter_end_to_end() {
    let mut g = FilterGroup::default();
    g.match_binaries.push(MatchBinary {
        operator: FilterOp::Prefix,
        values: vec!["/usr/bin/".into()],
    });
    g.match_args.push(MatchArg {
        index: 0,
        operator: FilterOp::Postfix,
        values: vec!["/etc/shadow".into()],
    });

    let ev = KernelEvent::FileOp(FileEvent {
        op: FileOp::Read,
        path: "/etc/shadow".into(),
        flags: Some(OpenFlags::O_RDONLY.bits()),
        mode: None,
        process: proc(1, "/usr/bin/cat"),
        observed_at: ts(),
    });
    assert!(g.matches(&ev).unwrap(), "matched policy should fire");
}

#[test]
fn smoke_3_enforcer_sigkill_decision() {
    let mut g = FilterGroup::default();
    g.match_binaries.push(MatchBinary {
        operator: FilterOp::Equal,
        values: vec!["/bin/bash".into()],
    });
    g.match_actions.push(MatchAction {
        action: ActionKind::Sigkill,
        arg_error: None,
        arg_sig: Some(9),
        arg_fd: None,
        arg_name: None,
        rate_limit: None,
    });

    let ev = KernelEvent::ProcessExec(ProcessExecEvent {
        process: proc(123, "/bin/bash"),
        ancestors: vec![],
        observed_at: ts(),
    });
    let e = Enforcer::default();
    let decisions = e.decide("kill-bash", &g, &ev).unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].action, ActionKind::Sigkill);
    assert_eq!(decisions[0].target_pid, 123);
}

#[test]
fn smoke_4_export_round_trips() {
    let evs = vec![
        KernelEvent::ProcessExec(ProcessExecEvent {
            process: proc(1, "/bin/sh"),
            ancestors: vec![],
            observed_at: ts(),
        }),
        KernelEvent::Network(NetworkEvent {
            op: NetworkOp::Connect,
            proto: L4Proto::Tcp,
            src_ip: ipv4(10, 0, 0, 1),
            src_port: 33333,
            dst_ip: ipv4(1, 1, 1, 1),
            dst_port: 443,
            bytes: 0,
            process: proc(1, "/bin/curl"),
            observed_at: ts(),
        }),
    ];

    // gRPC framed
    let bytes = encode_many(&evs).unwrap();
    let back = decode_events(&bytes).unwrap();
    assert_eq!(back, evs);

    // NDJSON
    let json = encode_ndjson(&evs).unwrap();
    let back2 = decode_ndjson(&json).unwrap();
    assert_eq!(back2, evs);

    // Single-frame too
    let single = encode_event(&evs[0]).unwrap();
    let back3 = decode_events(&single).unwrap();
    assert_eq!(back3, vec![evs[0].clone()]);
}

#[test]
fn smoke_5_case_ingestion_with_chain_of_custody() {
    let store = CaseStore::new();
    let c = store.open(
        "tetragon-detected shell-in-container",
        "CAP_SYS_ADMIN + /bin/bash inside victim pod",
        ForensicSeverity::Critical,
    );

    let ev = KernelEvent::ProcessExec(ProcessExecEvent {
        process: {
            let mut p = proc(42, "/bin/bash");
            p.credentials.caps_effective = cap::CAP_SYS_ADMIN;
            p
        },
        ancestors: vec!["init".into()],
        observed_at: ts(),
    });
    let updated = store.ingest_event(c.id, &ev, "tetragon-agent").unwrap();
    assert_eq!(updated.evidence.len(), 1);
    let item = &updated.evidence[0];
    assert!(matches!(item.evidence_type, EvidenceType::LogFile));
    assert_eq!(item.hash_sha256.as_ref().unwrap().len(), 64);

    // Chain-of-custody verifier
    let g = CustodyEntry::genesis("tetragon-agent", "collect");
    let f = CustodyEntry::following(&g, "soc-analyst", "review");
    let s = CustodyEntry::following(&f, "ir-lead", "seal");
    verify_chain(&[g, f, s]).expect("chain must verify");
}

#[test]
fn smoke_6_policy_store_dispatches_to_observer() {
    let s = PolicyStore::new();
    let mut p = TracingPolicy {
        api_version: "cilium.io/v1alpha1".into(),
        kind: PolicyKind::TracingPolicy,
        metadata: PolicyMeta {
            name: "block-curl".into(),
            ..Default::default()
        },
        spec: TracingPolicySpec {
            kprobes: vec![KProbeSpec {
                call: "sys_execve".into(),
                syscall: true,
                return_: false,
                args: vec![],
                selectors: vec![],
            }],
            ..Default::default()
        },
    };
    let mut g = FilterGroup::default();
    g.match_binaries.push(MatchBinary {
        operator: FilterOp::Postfix,
        values: vec!["curl".into()],
    });
    g.match_actions.push(MatchAction {
        action: ActionKind::Post,
        arg_error: None,
        arg_sig: None,
        arg_fd: None,
        arg_name: None,
        rate_limit: None,
    });
    p.spec.kprobes[0].selectors.push(g);
    s.install(p).unwrap();

    let ev = KernelEvent::ProcessExec(ProcessExecEvent {
        process: proc(1, "/usr/bin/curl"),
        ancestors: vec![],
        observed_at: ts(),
    });
    let matched = matching_groups(&s, &ev);
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].0, "block-curl");
}
