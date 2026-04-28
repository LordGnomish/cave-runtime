//! `cavectl etcdctl …` — etcd v3 client compat shim.
//!
//! Targets `/api/compat/etcd/v3/...`. cave-etcd owns the actual
//! KV/watch/lease/auth backends; this layer translates `etcdctl`'s
//! verb shape into routed requests.

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Subcommand, Debug, Clone)]
pub enum EtcdctlCmd {
    /// `etcdctl get <key>` (range read).
    Get {
        key: String,
        /// `--prefix` — return all keys with this prefix.
        #[arg(long)]
        prefix: bool,
        /// `--from-key` — open-ended range from `key`.
        #[arg(long = "from-key")]
        from_key: bool,
        /// `--rev` — at this mod-revision.
        #[arg(long)]
        rev: Option<i64>,
        /// `--limit` — cap returned key count.
        #[arg(long)]
        limit: Option<u64>,
        /// `--keys-only`.
        #[arg(long = "keys-only")]
        keys_only: bool,
    },
    /// `etcdctl put <key> <value>`.
    Put {
        key: String,
        value: String,
        #[arg(long)]
        lease: Option<String>,
        /// `--prev-kv` — return previous KV pair in response.
        #[arg(long = "prev-kv")]
        prev_kv: bool,
    },
    /// `etcdctl del <key>` (range delete).
    Del {
        key: String,
        #[arg(long)]
        prefix: bool,
        #[arg(long = "from-key")]
        from_key: bool,
        #[arg(long = "prev-kv")]
        prev_kv: bool,
    },
    /// `etcdctl watch <key>`.
    Watch {
        key: String,
        #[arg(long)]
        prefix: bool,
        #[arg(long = "rev")]
        rev: Option<i64>,
        #[arg(long = "prev-kv")]
        prev_kv: bool,
    },
    Snapshot {
        #[command(subcommand)]
        cmd: SnapshotCmd,
    },
    Lease {
        #[command(subcommand)]
        cmd: LeaseCmd,
    },
    Member {
        #[command(subcommand)]
        cmd: MemberCmd,
    },
    Endpoint {
        #[command(subcommand)]
        cmd: EndpointCmd,
    },
    Compaction {
        revision: i64,
        #[arg(long)]
        physical: bool,
    },
    Defrag,
    Alarm {
        #[command(subcommand)]
        cmd: AlarmCmd,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum SnapshotCmd {
    Save { path: String },
    Restore { path: String },
    Status { path: String },
}

#[derive(Subcommand, Debug, Clone)]
pub enum LeaseCmd {
    Grant { ttl: u64 },
    Revoke { id: String },
    KeepAlive { id: String },
    TimeToLive { id: String },
    List,
}

#[derive(Subcommand, Debug, Clone)]
pub enum MemberCmd {
    List,
    Add { name: String, peer_urls: String },
    Remove { id: String },
    Update { id: String, peer_urls: String },
}

#[derive(Subcommand, Debug, Clone)]
pub enum EndpointCmd {
    Health,
    Status,
    HashKv { rev: Option<i64> },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AlarmCmd {
    List,
    Disarm,
}

pub fn prepare(cmd: &EtcdctlCmd) -> Result<PreparedRequest> {
    match cmd {
        EtcdctlCmd::Get {
            key,
            prefix,
            from_key,
            rev,
            limit,
            keys_only,
        } => {
            validate_key(key)?;
            if *prefix && *from_key {
                bail!("--prefix and --from-key are mutually exclusive");
            }
            let mut body: Value = json!({"key": key});
            if *prefix {
                body["range_end"] = json!(prefix_end(key));
            } else if *from_key {
                body["from_key"] = json!(true);
            }
            if let Some(r) = rev {
                body["revision"] = json!(r);
            }
            if let Some(l) = limit {
                body["limit"] = json!(l);
            }
            if *keys_only {
                body["keys_only"] = json!(true);
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/etcd/v3/kv/range")
                    .with_body(body),
            )
        }
        EtcdctlCmd::Put {
            key,
            value,
            lease,
            prev_kv,
        } => {
            validate_key(key)?;
            let mut body: Value = json!({"key": key, "value": value});
            if let Some(l) = lease {
                body["lease"] = json!(l);
            }
            if *prev_kv {
                body["prev_kv"] = json!(true);
            }
            Ok(PreparedRequest::new(HttpVerb::Post, "/api/compat/etcd/v3/kv/put").with_body(body))
        }
        EtcdctlCmd::Del {
            key,
            prefix,
            from_key,
            prev_kv,
        } => {
            validate_key(key)?;
            if *prefix && *from_key {
                bail!("--prefix and --from-key are mutually exclusive");
            }
            let mut body: Value = json!({"key": key});
            if *prefix {
                body["range_end"] = json!(prefix_end(key));
            } else if *from_key {
                body["from_key"] = json!(true);
            }
            if *prev_kv {
                body["prev_kv"] = json!(true);
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/etcd/v3/kv/deleterange")
                    .with_body(body),
            )
        }
        EtcdctlCmd::Watch {
            key,
            prefix,
            rev,
            prev_kv,
        } => {
            validate_key(key)?;
            let mut body: Value = json!({"key": key});
            if *prefix {
                body["range_end"] = json!(prefix_end(key));
            }
            if let Some(r) = rev {
                body["start_revision"] = json!(r);
            }
            if *prev_kv {
                body["prev_kv"] = json!(true);
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/etcd/v3/watch")
                    .with_body(body),
            )
        }
        EtcdctlCmd::Snapshot { cmd } => prepare_snapshot(cmd),
        EtcdctlCmd::Lease { cmd } => prepare_lease(cmd),
        EtcdctlCmd::Member { cmd } => prepare_member(cmd),
        EtcdctlCmd::Endpoint { cmd } => prepare_endpoint(cmd),
        EtcdctlCmd::Compaction { revision, physical } => {
            let body = json!({"revision": revision, "physical": physical});
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/etcd/v3/kv/compaction")
                    .with_body(body),
            )
        }
        EtcdctlCmd::Defrag => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/etcd/v3/maintenance/defragment",
        )
        .with_body(json!({}))),
        EtcdctlCmd::Alarm { cmd } => prepare_alarm(cmd),
    }
}

fn prepare_snapshot(cmd: &SnapshotCmd) -> Result<PreparedRequest> {
    match cmd {
        SnapshotCmd::Save { path } => {
            if path.is_empty() {
                bail!("snapshot path required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/etcd/v3/maintenance/snapshot",
            )
            .with_body(json!({"path": path})))
        }
        SnapshotCmd::Restore { path } => {
            if path.is_empty() {
                bail!("snapshot path required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/etcd/v3/maintenance/snapshot/restore",
            )
            .with_body(json!({"path": path})))
        }
        SnapshotCmd::Status { path } => {
            if path.is_empty() {
                bail!("snapshot path required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!(
                    "/api/compat/etcd/v3/maintenance/snapshot/status?path={}",
                    path
                ),
            ))
        }
    }
}

fn prepare_lease(cmd: &LeaseCmd) -> Result<PreparedRequest> {
    match cmd {
        LeaseCmd::Grant { ttl } => {
            if *ttl == 0 {
                bail!("ttl must be > 0");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/etcd/v3/lease/grant",
            )
            .with_body(json!({"ttl": ttl})))
        }
        LeaseCmd::Revoke { id } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/etcd/v3/lease/revoke",
        )
        .with_body(json!({"id": id}))),
        LeaseCmd::KeepAlive { id } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/etcd/v3/lease/keepalive",
        )
        .with_body(json!({"id": id}))),
        LeaseCmd::TimeToLive { id } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/etcd/v3/lease/timetolive",
        )
        .with_body(json!({"id": id}))),
        LeaseCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/etcd/v3/lease/leases",
        )),
    }
}

fn prepare_member(cmd: &MemberCmd) -> Result<PreparedRequest> {
    match cmd {
        MemberCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/etcd/v3/cluster/member/list",
        )),
        MemberCmd::Add { name, peer_urls } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/etcd/v3/cluster/member/add",
        )
        .with_body(json!({"name": name, "peerURLs": split_urls(peer_urls)}))),
        MemberCmd::Remove { id } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/etcd/v3/cluster/member/remove",
        )
        .with_body(json!({"id": id}))),
        MemberCmd::Update { id, peer_urls } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/etcd/v3/cluster/member/update",
        )
        .with_body(json!({"id": id, "peerURLs": split_urls(peer_urls)}))),
    }
}

fn prepare_endpoint(cmd: &EndpointCmd) -> Result<PreparedRequest> {
    match cmd {
        EndpointCmd::Health => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/etcd/v3/maintenance/health",
        )),
        EndpointCmd::Status => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/etcd/v3/maintenance/status",
        )),
        EndpointCmd::HashKv { rev } => {
            let mut path = "/api/compat/etcd/v3/maintenance/hash".to_string();
            if let Some(r) = rev {
                path.push_str(&format!("?revision={}", r));
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
    }
}

fn prepare_alarm(cmd: &AlarmCmd) -> Result<PreparedRequest> {
    match cmd {
        AlarmCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/etcd/v3/maintenance/alarms",
        )),
        AlarmCmd::Disarm => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/etcd/v3/maintenance/alarms/disarm",
        )
        .with_body(json!({}))),
    }
}

fn validate_key(k: &str) -> Result<()> {
    if k.is_empty() {
        bail!("key required");
    }
    Ok(())
}

/// Compute the prefix `range_end` per etcd convention: increment last byte.
pub fn prefix_end(key: &str) -> String {
    if key.is_empty() {
        return "\0".to_string();
    }
    let mut bytes = key.as_bytes().to_vec();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] < 0xff {
            bytes[i] += 1;
            return String::from_utf8_lossy(&bytes[..i + 1]).into_owned();
        }
    }
    // All 0xff — etcd treats this as infinite range.
    "\0".to_string()
}

fn split_urls(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_basic() {
        let r = prepare(&EtcdctlCmd::Get {
            key: "/foo".into(),
            prefix: false,
            from_key: false,
            rev: None,
            limit: None,
            keys_only: false,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/etcd/v3/kv/range");
        assert_eq!(r.body.unwrap()["key"], "/foo");
    }

    #[test]
    fn get_prefix() {
        let r = prepare(&EtcdctlCmd::Get {
            key: "/foo".into(),
            prefix: true,
            from_key: false,
            rev: None,
            limit: None,
            keys_only: false,
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["range_end"], "/fop");
    }

    #[test]
    fn get_from_key() {
        let r = prepare(&EtcdctlCmd::Get {
            key: "a".into(),
            prefix: false,
            from_key: true,
            rev: None,
            limit: None,
            keys_only: false,
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["from_key"], true);
    }

    #[test]
    fn get_prefix_and_from_key_rejected() {
        assert!(prepare(&EtcdctlCmd::Get {
            key: "a".into(),
            prefix: true,
            from_key: true,
            rev: None,
            limit: None,
            keys_only: false,
        })
        .is_err());
    }

    #[test]
    fn get_with_rev_and_limit() {
        let r = prepare(&EtcdctlCmd::Get {
            key: "/foo".into(),
            prefix: false,
            from_key: false,
            rev: Some(42),
            limit: Some(10),
            keys_only: true,
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["revision"], 42);
        assert_eq!(body["limit"], 10);
        assert_eq!(body["keys_only"], true);
    }

    #[test]
    fn get_rejects_empty_key() {
        assert!(prepare(&EtcdctlCmd::Get {
            key: "".into(),
            prefix: false,
            from_key: false,
            rev: None,
            limit: None,
            keys_only: false,
        })
        .is_err());
    }

    #[test]
    fn put_basic() {
        let r = prepare(&EtcdctlCmd::Put {
            key: "/foo".into(),
            value: "bar".into(),
            lease: None,
            prev_kv: false,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/etcd/v3/kv/put");
        let body = r.body.unwrap();
        assert_eq!(body["value"], "bar");
    }

    #[test]
    fn put_with_lease() {
        let r = prepare(&EtcdctlCmd::Put {
            key: "/foo".into(),
            value: "bar".into(),
            lease: Some("694d8b09".into()),
            prev_kv: true,
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["lease"], "694d8b09");
        assert_eq!(body["prev_kv"], true);
    }

    #[test]
    fn del_basic() {
        let r = prepare(&EtcdctlCmd::Del {
            key: "/foo".into(),
            prefix: false,
            from_key: false,
            prev_kv: false,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/etcd/v3/kv/deleterange");
    }

    #[test]
    fn del_prefix() {
        let r = prepare(&EtcdctlCmd::Del {
            key: "/foo".into(),
            prefix: true,
            from_key: false,
            prev_kv: false,
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["range_end"], "/fop");
    }

    #[test]
    fn watch_with_prefix_and_rev() {
        let r = prepare(&EtcdctlCmd::Watch {
            key: "/foo".into(),
            prefix: true,
            rev: Some(7),
            prev_kv: true,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/etcd/v3/watch");
        let body = r.body.unwrap();
        assert_eq!(body["range_end"], "/fop");
        assert_eq!(body["start_revision"], 7);
        assert_eq!(body["prev_kv"], true);
    }

    #[test]
    fn snapshot_save() {
        let r = prepare(&EtcdctlCmd::Snapshot {
            cmd: SnapshotCmd::Save {
                path: "/tmp/etcd.db".into(),
            },
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/etcd/v3/maintenance/snapshot");
        assert_eq!(r.body.unwrap()["path"], "/tmp/etcd.db");
    }

    #[test]
    fn snapshot_restore() {
        let r = prepare(&EtcdctlCmd::Snapshot {
            cmd: SnapshotCmd::Restore {
                path: "/tmp/etcd.db".into(),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/snapshot/restore"));
    }

    #[test]
    fn snapshot_status_uses_get() {
        let r = prepare(&EtcdctlCmd::Snapshot {
            cmd: SnapshotCmd::Status {
                path: "/tmp/etcd.db".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
        assert!(r.path.contains("path=/tmp/etcd.db"));
    }

    #[test]
    fn snapshot_rejects_empty_path() {
        assert!(prepare(&EtcdctlCmd::Snapshot {
            cmd: SnapshotCmd::Save { path: "".into() },
        })
        .is_err());
    }

    #[test]
    fn lease_grant() {
        let r = prepare(&EtcdctlCmd::Lease {
            cmd: LeaseCmd::Grant { ttl: 60 },
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["ttl"], 60);
    }

    #[test]
    fn lease_grant_zero_rejected() {
        assert!(prepare(&EtcdctlCmd::Lease {
            cmd: LeaseCmd::Grant { ttl: 0 },
        })
        .is_err());
    }

    #[test]
    fn lease_revoke() {
        let r = prepare(&EtcdctlCmd::Lease {
            cmd: LeaseCmd::Revoke {
                id: "694d8b09".into(),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/lease/revoke"));
    }

    #[test]
    fn lease_keep_alive() {
        let r = prepare(&EtcdctlCmd::Lease {
            cmd: LeaseCmd::KeepAlive { id: "x".into() },
        })
        .unwrap();
        assert!(r.path.ends_with("/lease/keepalive"));
    }

    #[test]
    fn lease_ttl_lookup() {
        let r = prepare(&EtcdctlCmd::Lease {
            cmd: LeaseCmd::TimeToLive { id: "x".into() },
        })
        .unwrap();
        assert!(r.path.ends_with("/timetolive"));
    }

    #[test]
    fn lease_list() {
        let r = prepare(&EtcdctlCmd::Lease {
            cmd: LeaseCmd::List,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn member_list() {
        let r = prepare(&EtcdctlCmd::Member {
            cmd: MemberCmd::List,
        })
        .unwrap();
        assert!(r.path.ends_with("/member/list"));
    }

    #[test]
    fn member_add_splits_urls() {
        let r = prepare(&EtcdctlCmd::Member {
            cmd: MemberCmd::Add {
                name: "n1".into(),
                peer_urls: "https://a:2380, https://b:2380".into(),
            },
        })
        .unwrap();
        let body = r.body.unwrap();
        let urls = body["peerURLs"].as_array().unwrap();
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn member_remove() {
        let r = prepare(&EtcdctlCmd::Member {
            cmd: MemberCmd::Remove { id: "abc".into() },
        })
        .unwrap();
        assert!(r.path.ends_with("/member/remove"));
    }

    #[test]
    fn endpoint_health() {
        let r = prepare(&EtcdctlCmd::Endpoint {
            cmd: EndpointCmd::Health,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
        assert!(r.path.ends_with("/maintenance/health"));
    }

    #[test]
    fn endpoint_status() {
        let r = prepare(&EtcdctlCmd::Endpoint {
            cmd: EndpointCmd::Status,
        })
        .unwrap();
        assert!(r.path.ends_with("/maintenance/status"));
    }

    #[test]
    fn endpoint_hash_with_rev() {
        let r = prepare(&EtcdctlCmd::Endpoint {
            cmd: EndpointCmd::HashKv { rev: Some(42) },
        })
        .unwrap();
        assert!(r.path.contains("revision=42"));
    }

    #[test]
    fn compaction() {
        let r = prepare(&EtcdctlCmd::Compaction {
            revision: 100,
            physical: true,
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["revision"], 100);
        assert_eq!(body["physical"], true);
    }

    #[test]
    fn defrag() {
        let r = prepare(&EtcdctlCmd::Defrag).unwrap();
        assert!(r.path.ends_with("/maintenance/defragment"));
    }

    #[test]
    fn alarm_list() {
        let r = prepare(&EtcdctlCmd::Alarm {
            cmd: AlarmCmd::List,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn alarm_disarm() {
        let r = prepare(&EtcdctlCmd::Alarm {
            cmd: AlarmCmd::Disarm,
        })
        .unwrap();
        assert!(r.path.ends_with("/alarms/disarm"));
    }

    #[test]
    fn prefix_end_simple() {
        assert_eq!(prefix_end("/foo"), "/fop");
    }

    #[test]
    fn prefix_end_empty() {
        assert_eq!(prefix_end(""), "\0");
    }

    #[test]
    fn prefix_end_all_ff() {
        // Can't represent in str easily — rely on byte logic.
        let s: String = String::from_utf8(vec![0xff, 0xff]).unwrap_or_else(|_| "".into());
        // utf8 disallows raw 0xff so this is a noop test; documents intent.
        let _ = prefix_end(&s);
    }

    #[test]
    fn paths_use_compat_etcd_prefix() {
        let r = prepare(&EtcdctlCmd::Defrag).unwrap();
        assert!(r.path.starts_with("/api/compat/etcd/v3/"));
    }
}
