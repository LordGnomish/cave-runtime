// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD cycle — process discovery + exec/exit watcher (userspace
// model of grafana/beyla pkg/internal/discover). Beyla finds processes to
// instrument by matching executable path / open-port discovery criteria,
// watches /proc for exec & exit, and derives a service name per process.

use cave_ebpf_common::process::{
    Criteria, PortRange, ProcessInfo, ProcessWatcher,
};

fn proc(pid: u32, exe: &str, ports: &[u16]) -> ProcessInfo {
    ProcessInfo {
        pid,
        ppid: 1,
        exe_path: exe.into(),
        comm: exe.rsplit('/').next().unwrap_or(exe).into(),
        open_ports: ports.to_vec(),
    }
}

#[test]
fn test_port_range_parse_and_contains() {
    let r = PortRange::parse("8080-8089").unwrap();
    assert_eq!((r.lo, r.hi), (8080, 8089));
    assert!(r.contains(8085));
    assert!(!r.contains(8090));

    let single = PortRange::parse("9000").unwrap();
    assert_eq!((single.lo, single.hi), (9000, 9000));
    assert!(single.contains(9000));

    assert!(PortRange::parse("not-a-port").is_none());
    assert!(PortRange::parse("90-10").is_none()); // inverted
}

#[test]
fn test_exe_substring_criteria_matches() {
    let crit = Criteria {
        exe_substring: Some("/usr/bin/myapp".into()),
        ports: None,
    };
    assert!(crit.matches(&proc(10, "/usr/bin/myapp", &[])));
    assert!(!crit.matches(&proc(11, "/usr/bin/other", &[])));
}

#[test]
fn test_port_criteria_matches_open_port() {
    let crit = Criteria {
        exe_substring: None,
        ports: Some(PortRange::parse("8080-8089").unwrap()),
    };
    assert!(crit.matches(&proc(10, "/x", &[3000, 8085])));
    assert!(!crit.matches(&proc(11, "/x", &[3000, 9090])));
}

#[test]
fn test_both_selectors_are_anded() {
    let crit = Criteria {
        exe_substring: Some("myapp".into()),
        ports: Some(PortRange::parse("8080").unwrap()),
    };
    assert!(crit.matches(&proc(10, "/opt/myapp", &[8080])));
    assert!(!crit.matches(&proc(11, "/opt/myapp", &[9999]))); // exe ok, port no
    assert!(!crit.matches(&proc(12, "/opt/other", &[8080]))); // port ok, exe no
}

#[test]
fn test_empty_criteria_matches_nothing() {
    let crit = Criteria {
        exe_substring: None,
        ports: None,
    };
    assert!(!crit.matches(&proc(10, "/anything", &[80])));
}

#[test]
fn test_service_name_defaults_to_exe_basename() {
    let p = proc(10, "/usr/local/bin/checkout-service", &[]);
    assert_eq!(p.service_name(None), "checkout-service");
    assert_eq!(p.service_name(Some("override")), "override");
}

#[test]
fn test_watcher_tracks_matching_exec_only() {
    let mut w = ProcessWatcher::new(vec![Criteria {
        exe_substring: Some("myapp".into()),
        ports: None,
    }]);
    assert!(w.on_exec(proc(100, "/opt/myapp", &[])).is_some());
    assert!(w.on_exec(proc(101, "/opt/nope", &[])).is_none());
    assert_eq!(w.tracked_count(), 1);
    assert!(w.is_tracked(100));
    assert!(!w.is_tracked(101));
}

#[test]
fn test_watcher_exit_untracks() {
    let mut w = ProcessWatcher::new(vec![Criteria {
        exe_substring: Some("myapp".into()),
        ports: None,
    }]);
    w.on_exec(proc(100, "/opt/myapp", &[]));
    assert!(w.on_exit(100));
    assert!(!w.is_tracked(100));
    assert!(!w.on_exit(100)); // already gone
}

#[test]
fn test_watcher_matches_any_criteria() {
    let mut w = ProcessWatcher::new(vec![
        Criteria {
            exe_substring: Some("frontend".into()),
            ports: None,
        },
        Criteria {
            exe_substring: None,
            ports: Some(PortRange::parse("5432").unwrap()),
        },
    ]);
    assert!(w.on_exec(proc(1, "/bin/frontend", &[])).is_some());
    assert!(w.on_exec(proc(2, "/bin/postgres", &[5432])).is_some());
    assert!(w.on_exec(proc(3, "/bin/random", &[22])).is_none());
    assert_eq!(w.tracked_count(), 2);
}
