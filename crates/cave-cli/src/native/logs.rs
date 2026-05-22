// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl logs <crate-or-pod> [-c <component>] [--follow] [--tail <n>]`
//!
//! Native logs verb. Targets a *Cave module* (a crate name) or a pod.
//! When the target is a module, the runtime fans out across every
//! pod that crate currently runs in.

use anyhow::{Result, bail};
use clap::Args;

use super::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct LogsArgs {
    /// Target — a crate name (e.g. `cave-apiserver`) or a pod name.
    pub target: String,

    #[arg(short = 't', long)]
    pub tenant: Option<String>,

    /// Specific component within the target. For a crate, this picks
    /// one of its replicas; for a pod, a container.
    #[arg(short = 'c', long)]
    pub component: Option<String>,

    #[arg(short = 'f', long)]
    pub follow: bool,

    #[arg(long)]
    pub tail: Option<u64>,

    /// ISO-8601 timestamp to start from.
    #[arg(long)]
    pub since: Option<String>,

    /// Show timestamps in output.
    #[arg(long)]
    pub timestamps: bool,
}

pub fn prepare(args: &LogsArgs) -> Result<PreparedRequest> {
    if args.target.is_empty() {
        bail!("target required");
    }
    let mut path = match args.tenant.as_deref() {
        Some(t) => format!("/api/native/tenants/{}/logs/{}", t, args.target),
        None => format!("/api/native/logs/{}", args.target),
    };
    let mut params: Vec<String> = Vec::new();
    if let Some(c) = &args.component {
        params.push(format!("component={}", c));
    }
    if args.follow {
        params.push("follow=true".to_string());
    }
    if let Some(n) = args.tail {
        params.push(format!("tail={}", n));
    }
    if let Some(s) = &args.since {
        params.push(format!("since={}", s));
    }
    if args.timestamps {
        params.push("timestamps=true".to_string());
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    Ok(PreparedRequest::new(HttpVerb::Get, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(target: &str) -> LogsArgs {
        LogsArgs {
            target: target.into(),
            tenant: None,
            component: None,
            follow: false,
            tail: None,
            since: None,
            timestamps: false,
        }
    }

    #[test]
    fn logs_default_path() {
        let r = prepare(&args("cave-apiserver")).unwrap();
        assert_eq!(r.path, "/api/native/logs/cave-apiserver");
    }

    #[test]
    fn logs_with_tenant() {
        let mut a = args("cave-apiserver");
        a.tenant = Some("acme".into());
        let r = prepare(&a).unwrap();
        assert_eq!(r.path, "/api/native/tenants/acme/logs/cave-apiserver");
    }

    #[test]
    fn logs_follow() {
        let mut a = args("cave-x");
        a.follow = true;
        let p = prepare(&a).unwrap().path;
        assert!(p.contains("follow=true"));
    }

    #[test]
    fn logs_tail_n() {
        let mut a = args("cave-x");
        a.tail = Some(50);
        assert!(prepare(&a).unwrap().path.contains("tail=50"));
    }

    #[test]
    fn logs_since() {
        let mut a = args("cave-x");
        a.since = Some("2026-04-26T10:00:00Z".into());
        assert!(
            prepare(&a)
                .unwrap()
                .path
                .contains("since=2026-04-26T10:00:00Z")
        );
    }

    #[test]
    fn logs_timestamps() {
        let mut a = args("cave-x");
        a.timestamps = true;
        assert!(prepare(&a).unwrap().path.contains("timestamps=true"));
    }

    #[test]
    fn logs_component() {
        let mut a = args("cave-x");
        a.component = Some("worker".into());
        assert!(prepare(&a).unwrap().path.contains("component=worker"));
    }

    #[test]
    fn logs_uses_get() {
        let r = prepare(&args("cave-x")).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn logs_no_body() {
        let r = prepare(&args("cave-x")).unwrap();
        assert!(r.body.is_none());
    }

    #[test]
    fn logs_rejects_empty_target() {
        assert!(prepare(&args("")).is_err());
    }

    #[test]
    fn logs_combined_flags() {
        let mut a = args("cave-x");
        a.follow = true;
        a.tail = Some(10);
        a.timestamps = true;
        let p = prepare(&a).unwrap().path;
        assert!(p.contains("follow=true"));
        assert!(p.contains("tail=10"));
        assert!(p.contains("timestamps=true"));
    }

    #[test]
    fn logs_pod_name_target() {
        let r = prepare(&args("nginx-x1")).unwrap();
        assert_eq!(r.path, "/api/native/logs/nginx-x1");
    }

    #[test]
    fn logs_query_appended_after_path() {
        let mut a = args("cave-x");
        a.follow = true;
        let p = prepare(&a).unwrap().path;
        assert!(p.contains('?'));
        let qpos = p.find('?').unwrap();
        assert!(qpos > "/api/native/logs/".len());
    }

    #[test]
    fn logs_no_query_when_no_flags() {
        let p = prepare(&args("cave-x")).unwrap().path;
        assert!(!p.contains('?'));
    }

    #[test]
    fn logs_tail_zero() {
        let mut a = args("cave-x");
        a.tail = Some(0);
        assert!(prepare(&a).unwrap().path.contains("tail=0"));
    }

    #[test]
    fn logs_long_tail() {
        let mut a = args("cave-x");
        a.tail = Some(1_000_000);
        assert!(prepare(&a).unwrap().path.contains("tail=1000000"));
    }
}
