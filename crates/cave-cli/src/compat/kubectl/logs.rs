//! `cavectl kubectl logs …`

use anyhow::{bail, Result};
use clap::Args;

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct KubectlLogsArgs {
    pub pod: String,
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
    #[arg(short = 'c', long)]
    pub container: Option<String>,
    #[arg(short = 'f', long)]
    pub follow: bool,
    #[arg(long)]
    pub tail: Option<i64>,
    #[arg(short = 'p', long)]
    pub previous: bool,
    #[arg(long)]
    pub timestamps: bool,
    #[arg(long = "since-time")]
    pub since_time: Option<String>,
    #[arg(long = "since")]
    pub since: Option<String>,
}

pub fn prepare(args: &KubectlLogsArgs) -> Result<PreparedRequest> {
    if args.pod.is_empty() {
        bail!("pod required");
    }
    let ns = args.namespace.as_deref().unwrap_or("default");
    let mut path = format!(
        "/api/compat/kubectl/v1/namespaces/{}/pods/{}/log",
        ns, args.pod
    );
    let mut params: Vec<String> = Vec::new();
    if let Some(c) = &args.container {
        params.push(format!("container={}", c));
    }
    if args.follow {
        params.push("follow=true".to_string());
    }
    if let Some(n) = args.tail {
        if n < -1 {
            bail!("--tail must be >= -1");
        }
        params.push(format!("tailLines={}", n));
    }
    if args.previous {
        params.push("previous=true".to_string());
    }
    if args.timestamps {
        params.push("timestamps=true".to_string());
    }
    if let Some(s) = &args.since_time {
        params.push(format!("sinceTime={}", s));
    }
    if let Some(s) = &args.since {
        params.push(format!("sinceSeconds={}", s));
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

    fn args(pod: &str) -> KubectlLogsArgs {
        KubectlLogsArgs {
            pod: pod.into(),
            namespace: None,
            container: None,
            follow: false,
            tail: None,
            previous: false,
            timestamps: false,
            since_time: None,
            since: None,
        }
    }

    #[test]
    fn logs_default() {
        let r = prepare(&args("nginx")).unwrap();
        assert_eq!(
            r.path,
            "/api/compat/kubectl/v1/namespaces/default/pods/nginx/log"
        );
    }

    #[test]
    fn logs_follow() {
        let mut a = args("nginx");
        a.follow = true;
        assert!(prepare(&a).unwrap().path.contains("follow=true"));
    }

    #[test]
    fn logs_tail_positive() {
        let mut a = args("nginx");
        a.tail = Some(50);
        assert!(prepare(&a).unwrap().path.contains("tailLines=50"));
    }

    #[test]
    fn logs_tail_minus_one_means_all() {
        let mut a = args("nginx");
        a.tail = Some(-1);
        assert!(prepare(&a).unwrap().path.contains("tailLines=-1"));
    }

    #[test]
    fn logs_tail_too_negative_rejected() {
        let mut a = args("nginx");
        a.tail = Some(-2);
        assert!(prepare(&a).is_err());
    }

    #[test]
    fn logs_previous() {
        let mut a = args("nginx");
        a.previous = true;
        assert!(prepare(&a).unwrap().path.contains("previous=true"));
    }

    #[test]
    fn logs_timestamps() {
        let mut a = args("nginx");
        a.timestamps = true;
        assert!(prepare(&a).unwrap().path.contains("timestamps=true"));
    }

    #[test]
    fn logs_since_time() {
        let mut a = args("nginx");
        a.since_time = Some("2026-04-26T10:00:00Z".into());
        assert!(prepare(&a)
            .unwrap()
            .path
            .contains("sinceTime=2026-04-26T10:00:00Z"));
    }

    #[test]
    fn logs_since_seconds() {
        let mut a = args("nginx");
        a.since = Some("3600".into());
        assert!(prepare(&a).unwrap().path.contains("sinceSeconds=3600"));
    }

    #[test]
    fn logs_container() {
        let mut a = args("nginx");
        a.container = Some("sidecar".into());
        assert!(prepare(&a).unwrap().path.contains("container=sidecar"));
    }

    #[test]
    fn logs_namespace() {
        let mut a = args("nginx");
        a.namespace = Some("prod".into());
        assert!(prepare(&a).unwrap().path.contains("/namespaces/prod/"));
    }

    #[test]
    fn logs_uses_get() {
        let r = prepare(&args("nginx")).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn logs_rejects_empty_pod() {
        assert!(prepare(&args("")).is_err());
    }

    #[test]
    fn logs_combined() {
        let mut a = args("nginx");
        a.follow = true;
        a.tail = Some(10);
        a.timestamps = true;
        a.previous = true;
        let p = prepare(&a).unwrap().path;
        assert!(p.contains("follow=true"));
        assert!(p.contains("tailLines=10"));
        assert!(p.contains("timestamps=true"));
        assert!(p.contains("previous=true"));
    }

    #[test]
    fn logs_compat_path_prefix() {
        let r = prepare(&args("x")).unwrap();
        assert!(r.path.starts_with("/api/compat/kubectl/"));
    }
}
