// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cavectl kubectl delete …`

use anyhow::{bail, Result};
use clap::Args;

use crate::compat::kubectl::resource::ns_path;
use crate::native::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct KubectlDeleteArgs {
    pub resource: String,
    pub names: Vec<String>,
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
    #[arg(short = 'l', long = "selector")]
    pub selector: Option<String>,
    #[arg(long)]
    pub all: bool,
    #[arg(short = 'f', long = "filename")]
    pub filename: Option<String>,
    /// Grace period in seconds (`--grace-period`).
    #[arg(long = "grace-period")]
    pub grace_period: Option<i64>,
    #[arg(long)]
    pub force: bool,
}

pub fn prepare(args: &KubectlDeleteArgs) -> Result<PreparedRequest> {
    let chosen = [
        !args.names.is_empty(),
        args.selector.is_some(),
        args.all,
        args.filename.is_some(),
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if chosen == 0 {
        bail!("delete needs name(s), --selector, --all, or -f");
    }
    if chosen > 1 {
        bail!("delete: name(s)/selector/all/-f are mutually exclusive");
    }

    let mut path = ns_path(&args.resource, args.namespace.as_deref(), false)?;
    if let Some(name) = args.names.first() {
        path.push('/');
        path.push_str(name);
    }

    let mut params: Vec<String> = Vec::new();
    if let Some(s) = &args.selector {
        params.push(format!("labelSelector={}", s));
    }
    if let Some(g) = args.grace_period {
        params.push(format!("gracePeriodSeconds={}", g));
    }
    if args.force {
        params.push("force=true".to_string());
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    Ok(PreparedRequest::new(HttpVerb::Delete, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(resource: &str) -> KubectlDeleteArgs {
        KubectlDeleteArgs {
            resource: resource.into(),
            names: vec![],
            namespace: None,
            selector: None,
            all: false,
            filename: None,
            grace_period: None,
            force: false,
        }
    }

    #[test]
    fn delete_by_name() {
        let mut a = args("pods");
        a.names = vec!["nginx".into()];
        let r = prepare(&a).unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
        assert_eq!(
            r.path,
            "/api/compat/kubectl/v1/namespaces/default/pods/nginx"
        );
    }

    #[test]
    fn delete_by_selector() {
        let mut a = args("pods");
        a.selector = Some("app=stale".into());
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("labelSelector=app=stale"));
    }

    #[test]
    fn delete_all() {
        let mut a = args("pods");
        a.all = true;
        let r = prepare(&a).unwrap();
        assert_eq!(
            r.path,
            "/api/compat/kubectl/v1/namespaces/default/pods"
        );
    }

    #[test]
    fn delete_with_filename() {
        let mut a = args("pods");
        a.filename = Some("x.yaml".into());
        assert!(prepare(&a).is_ok());
    }

    #[test]
    fn delete_no_target() {
        let r = prepare(&args("pods"));
        assert!(r.is_err());
    }

    #[test]
    fn delete_mixed_targets() {
        let mut a = args("pods");
        a.names = vec!["x".into()];
        a.all = true;
        assert!(prepare(&a).is_err());
    }

    #[test]
    fn delete_grace_period() {
        let mut a = args("pods");
        a.names = vec!["x".into()];
        a.grace_period = Some(0);
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("gracePeriodSeconds=0"));
    }

    #[test]
    fn delete_force() {
        let mut a = args("pods");
        a.names = vec!["x".into()];
        a.force = true;
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("force=true"));
    }

    #[test]
    fn delete_namespace() {
        let mut a = args("pods");
        a.names = vec!["x".into()];
        a.namespace = Some("prod".into());
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("/namespaces/prod/"));
    }

    #[test]
    fn delete_short_alias() {
        let mut a = args("po");
        a.names = vec!["x".into()];
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("/pods/x"));
    }

    #[test]
    fn delete_combined_force_grace() {
        let mut a = args("pods");
        a.names = vec!["x".into()];
        a.force = true;
        a.grace_period = Some(30);
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("force=true"));
        assert!(r.path.contains("gracePeriodSeconds=30"));
    }

    #[test]
    fn delete_no_body() {
        let mut a = args("pods");
        a.names = vec!["x".into()];
        let r = prepare(&a).unwrap();
        assert!(r.body.is_none());
    }
}
