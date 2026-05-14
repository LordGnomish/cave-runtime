// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl chaos run/list/abort` — native chaos engineering verb.

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use super::{HttpVerb, PreparedRequest};

#[derive(Subcommand, Debug, Clone)]
pub enum ChaosCmd {
    /// Run an experiment.
    Run {
        #[arg(long)]
        kind: String,
        #[arg(long)]
        target: String,
        #[arg(long, default_value = "5m")]
        duration: String,
        #[arg(short = 't', long)]
        tenant: Option<String>,
        #[arg(long)]
        id: Option<String>,
    },
    /// List experiments.
    List {
        #[arg(short = 't', long)]
        tenant: Option<String>,
        #[arg(long)]
        status: Option<String>,
    },
    /// Abort a running experiment.
    Abort {
        id: String,
        #[arg(short = 't', long)]
        tenant: Option<String>,
    },
}

const KINDS: &[&str] = &[
    "network", "pod", "io", "time", "kernel", "stress", "process", "dns",
];
const STATUSES: &[&str] = &["pending", "running", "finished", "failed", "aborted"];

pub fn prepare(cmd: &ChaosCmd) -> Result<PreparedRequest> {
    match cmd {
        ChaosCmd::Run {
            kind,
            target,
            duration,
            tenant,
            id,
        } => {
            if !KINDS.contains(&kind.as_str()) {
                bail!("unknown kind `{}`; want one of {:?}", kind, KINDS);
            }
            parse_duration_seconds(duration)?;
            let mut body: Value = json!({
                "kind": kind,
                "target": target,
                "duration": duration,
            });
            if let Some(i) = id {
                body["id"] = json!(i);
            }
            Ok(PreparedRequest::new(HttpVerb::Post, scoped(tenant.as_deref(), None))
                .with_body(body))
        }
        ChaosCmd::List { tenant, status } => {
            let mut path = scoped(tenant.as_deref(), None);
            if let Some(s) = status {
                if !STATUSES.contains(&s.as_str()) {
                    bail!("unknown status `{}`; want one of {:?}", s, STATUSES);
                }
                path.push_str(&format!("?status={}", s));
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        ChaosCmd::Abort { id, tenant } => {
            if id.is_empty() {
                bail!("experiment id required");
            }
            let path = format!("{}/abort", scoped(tenant.as_deref(), Some(id)));
            Ok(PreparedRequest::new(HttpVerb::Post, path).with_body(json!({})))
        }
    }
}

fn scoped(tenant: Option<&str>, id: Option<&str>) -> String {
    let base = match tenant {
        Some(t) => format!("/api/native/tenants/{}/chaos", t),
        None => "/api/native/chaos".to_string(),
    };
    match id {
        Some(i) => format!("{}/{}", base, i),
        None => base,
    }
}

/// Convert a duration string like `5m`/`30s`/`1h` into seconds. Strict.
pub fn parse_duration_seconds(s: &str) -> Result<u64> {
    if s.is_empty() {
        bail!("empty duration");
    }
    let split = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let (num, suffix) = s.split_at(split);
    let n: u64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid number `{}`", num))?;
    let secs = match suffix {
        "ms" => {
            if n % 1000 != 0 {
                bail!("ms must be multiple of 1000");
            }
            n / 1000
        }
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        other => bail!("unknown duration suffix `{}`", other),
    };
    if secs == 0 {
        bail!("duration must be positive");
    }
    Ok(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_default_path() {
        let r = prepare(&ChaosCmd::Run {
            kind: "network".into(),
            target: "app=api".into(),
            duration: "5m".into(),
            tenant: None,
            id: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(r.path, "/api/native/chaos");
    }

    #[test]
    fn run_with_tenant() {
        let r = prepare(&ChaosCmd::Run {
            kind: "pod".into(),
            target: "app=api".into(),
            duration: "30s".into(),
            tenant: Some("acme".into()),
            id: None,
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/tenants/acme/chaos");
    }

    #[test]
    fn run_kinds_round_trip() {
        for k in KINDS {
            let r = prepare(&ChaosCmd::Run {
                kind: (*k).into(),
                target: "x".into(),
                duration: "30s".into(),
                tenant: None,
                id: None,
            });
            assert!(r.is_ok(), "kind {} should be accepted", k);
        }
    }

    #[test]
    fn run_rejects_unknown_kind() {
        assert!(prepare(&ChaosCmd::Run {
            kind: "wormhole".into(),
            target: "x".into(),
            duration: "30s".into(),
            tenant: None,
            id: None,
        })
        .is_err());
    }

    #[test]
    fn run_rejects_bad_duration() {
        assert!(prepare(&ChaosCmd::Run {
            kind: "pod".into(),
            target: "x".into(),
            duration: "5x".into(),
            tenant: None,
            id: None,
        })
        .is_err());
    }

    #[test]
    fn run_with_explicit_id() {
        let r = prepare(&ChaosCmd::Run {
            kind: "pod".into(),
            target: "x".into(),
            duration: "30s".into(),
            tenant: None,
            id: Some("exp-1".into()),
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["id"], "exp-1");
    }

    #[test]
    fn run_omits_id_when_unset() {
        let r = prepare(&ChaosCmd::Run {
            kind: "pod".into(),
            target: "x".into(),
            duration: "30s".into(),
            tenant: None,
            id: None,
        })
        .unwrap();
        assert!(r.body.unwrap().get("id").is_none());
    }

    #[test]
    fn list_no_filter() {
        let r = prepare(&ChaosCmd::List {
            tenant: None,
            status: None,
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/chaos");
    }

    #[test]
    fn list_with_status() {
        let r = prepare(&ChaosCmd::List {
            tenant: None,
            status: Some("running".into()),
        })
        .unwrap();
        assert!(r.path.contains("status=running"));
    }

    #[test]
    fn list_statuses_round_trip() {
        for s in STATUSES {
            let r = prepare(&ChaosCmd::List {
                tenant: None,
                status: Some((*s).into()),
            });
            assert!(r.is_ok(), "status {} should be accepted", s);
        }
    }

    #[test]
    fn list_rejects_unknown_status() {
        assert!(prepare(&ChaosCmd::List {
            tenant: None,
            status: Some("zombie".into()),
        })
        .is_err());
    }

    #[test]
    fn list_with_tenant() {
        let r = prepare(&ChaosCmd::List {
            tenant: Some("acme".into()),
            status: None,
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/tenants/acme/chaos");
    }

    #[test]
    fn abort_path() {
        let r = prepare(&ChaosCmd::Abort {
            id: "x".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/chaos/x/abort");
    }

    #[test]
    fn abort_with_tenant() {
        let r = prepare(&ChaosCmd::Abort {
            id: "x".into(),
            tenant: Some("acme".into()),
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/tenants/acme/chaos/x/abort");
    }

    #[test]
    fn abort_uses_post() {
        let r = prepare(&ChaosCmd::Abort {
            id: "x".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
    }

    #[test]
    fn abort_rejects_empty_id() {
        assert!(prepare(&ChaosCmd::Abort {
            id: "".into(),
            tenant: None,
        })
        .is_err());
    }

    #[test]
    fn duration_seconds() {
        assert_eq!(parse_duration_seconds("30s").unwrap(), 30);
    }

    #[test]
    fn duration_minutes() {
        assert_eq!(parse_duration_seconds("5m").unwrap(), 300);
    }

    #[test]
    fn duration_hours() {
        assert_eq!(parse_duration_seconds("2h").unwrap(), 7200);
    }

    #[test]
    fn duration_days() {
        assert_eq!(parse_duration_seconds("1d").unwrap(), 86_400);
    }

    #[test]
    fn duration_ms_round() {
        assert_eq!(parse_duration_seconds("3000ms").unwrap(), 3);
    }

    #[test]
    fn duration_ms_unround_rejects() {
        assert!(parse_duration_seconds("500ms").is_err());
    }

    #[test]
    fn duration_zero_rejects() {
        assert!(parse_duration_seconds("0s").is_err());
    }

    #[test]
    fn duration_empty_rejects() {
        assert!(parse_duration_seconds("").is_err());
    }

    #[test]
    fn duration_no_unit_rejects() {
        assert!(parse_duration_seconds("30").is_err());
    }

    #[test]
    fn duration_unknown_unit_rejects() {
        assert!(parse_duration_seconds("5y").is_err());
    }

    #[test]
    fn run_uses_post() {
        let r = prepare(&ChaosCmd::Run {
            kind: "pod".into(),
            target: "x".into(),
            duration: "1m".into(),
            tenant: None,
            id: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
    }

    #[test]
    fn list_no_body() {
        let r = prepare(&ChaosCmd::List {
            tenant: None,
            status: None,
        })
        .unwrap();
        assert!(r.body.is_none());
    }

    #[test]
    fn run_target_in_body() {
        let r = prepare(&ChaosCmd::Run {
            kind: "io".into(),
            target: "tier=db".into(),
            duration: "30s".into(),
            tenant: None,
            id: None,
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["target"], "tier=db");
    }
}
