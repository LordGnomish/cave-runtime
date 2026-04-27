//! `cavectl events` — federated event stream.
//!
//! A native cross-module verb: events surface from cave-kernel's
//! EventBus, federated across tenants when invoked with platform-admin.

use anyhow::Result;
use clap::Args;

use super::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct EventsArgs {
    #[arg(short = 't', long)]
    pub tenant: Option<String>,

    #[arg(long)]
    pub all_tenants: bool,

    /// Filter to a specific kind (e.g. `Reconcile`, `Promote`, `Alert`).
    #[arg(long)]
    pub kind: Option<String>,

    /// Filter to a source module/crate.
    #[arg(long)]
    pub source: Option<String>,

    #[arg(short = 'f', long)]
    pub follow: bool,

    /// Look back this many minutes.
    #[arg(long, default_value_t = 15)]
    pub minutes: u64,

    #[arg(long)]
    pub limit: Option<u64>,
}

pub fn prepare(args: &EventsArgs) -> Result<PreparedRequest> {
    let mut path = if args.all_tenants {
        "/api/native/all/events".to_string()
    } else {
        match args.tenant.as_deref() {
            Some(t) => format!("/api/native/tenants/{}/events", t),
            None => "/api/native/events".to_string(),
        }
    };
    let mut params: Vec<String> = Vec::new();
    if let Some(k) = &args.kind {
        params.push(format!("kind={}", k));
    }
    if let Some(s) = &args.source {
        params.push(format!("source={}", s));
    }
    if args.follow {
        params.push("follow=true".to_string());
    }
    params.push(format!("minutes={}", args.minutes));
    if let Some(l) = args.limit {
        params.push(format!("limit={}", l));
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

    fn args() -> EventsArgs {
        EventsArgs {
            tenant: None,
            all_tenants: false,
            kind: None,
            source: None,
            follow: false,
            minutes: 15,
            limit: None,
        }
    }

    #[test]
    fn events_default_path() {
        let r = prepare(&args()).unwrap();
        assert!(r.path.starts_with("/api/native/events?"));
    }

    #[test]
    fn events_default_minutes() {
        let r = prepare(&args()).unwrap();
        assert!(r.path.contains("minutes=15"));
    }

    #[test]
    fn events_with_tenant() {
        let mut a = args();
        a.tenant = Some("acme".into());
        let p = prepare(&a).unwrap().path;
        assert!(p.starts_with("/api/native/tenants/acme/events"));
    }

    #[test]
    fn events_all_tenants() {
        let mut a = args();
        a.all_tenants = true;
        assert!(prepare(&a).unwrap().path.starts_with("/api/native/all/events"));
    }

    #[test]
    fn events_kind_filter() {
        let mut a = args();
        a.kind = Some("Promote".into());
        assert!(prepare(&a).unwrap().path.contains("kind=Promote"));
    }

    #[test]
    fn events_source_filter() {
        let mut a = args();
        a.source = Some("cave-apiserver".into());
        assert!(prepare(&a)
            .unwrap()
            .path
            .contains("source=cave-apiserver"));
    }

    #[test]
    fn events_follow() {
        let mut a = args();
        a.follow = true;
        assert!(prepare(&a).unwrap().path.contains("follow=true"));
    }

    #[test]
    fn events_custom_minutes() {
        let mut a = args();
        a.minutes = 60;
        assert!(prepare(&a).unwrap().path.contains("minutes=60"));
    }

    #[test]
    fn events_with_limit() {
        let mut a = args();
        a.limit = Some(100);
        assert!(prepare(&a).unwrap().path.contains("limit=100"));
    }

    #[test]
    fn events_uses_get() {
        let r = prepare(&args()).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn events_no_body() {
        let r = prepare(&args()).unwrap();
        assert!(r.body.is_none());
    }

    #[test]
    fn events_combined_filters() {
        let mut a = args();
        a.kind = Some("Alert".into());
        a.source = Some("cave-net".into());
        a.follow = true;
        let p = prepare(&a).unwrap().path;
        assert!(p.contains("kind=Alert"));
        assert!(p.contains("source=cave-net"));
        assert!(p.contains("follow=true"));
    }

    #[test]
    fn events_all_tenants_with_kind() {
        let mut a = args();
        a.all_tenants = true;
        a.kind = Some("Reconcile".into());
        let p = prepare(&a).unwrap().path;
        assert!(p.starts_with("/api/native/all/events?"));
        assert!(p.contains("kind=Reconcile"));
    }
}
