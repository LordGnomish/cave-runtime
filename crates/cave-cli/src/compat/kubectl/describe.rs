// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cavectl kubectl describe …`

use anyhow::{bail, Result};
use clap::Args;

use crate::compat::kubectl::resource::ns_path;
use crate::native::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct KubectlDescribeArgs {
    pub resource: String,
    pub name: String,
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
    #[arg(short = 'l', long = "selector")]
    pub selector: Option<String>,
    #[arg(short = 'A', long = "all-namespaces")]
    pub all_namespaces: bool,
}

pub fn prepare(args: &KubectlDescribeArgs) -> Result<PreparedRequest> {
    if args.name.is_empty() && args.selector.is_none() {
        bail!("describe needs a name or --selector");
    }
    let mut path = ns_path(
        &args.resource,
        args.namespace.as_deref(),
        args.all_namespaces,
    )?;
    if !args.name.is_empty() {
        path.push('/');
        path.push_str(&args.name);
    }
    let mut params = vec!["describe=true".to_string()];
    if let Some(s) = &args.selector {
        params.push(format!("labelSelector={}", s));
    }
    path.push('?');
    path.push_str(&params.join("&"));
    Ok(PreparedRequest::new(HttpVerb::Get, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(r: &str, n: &str) -> KubectlDescribeArgs {
        KubectlDescribeArgs {
            resource: r.into(),
            name: n.into(),
            namespace: None,
            selector: None,
            all_namespaces: false,
        }
    }

    #[test]
    fn describe_pod() {
        let r = prepare(&args("pods", "nginx")).unwrap();
        assert_eq!(
            r.path,
            "/api/compat/kubectl/v1/namespaces/default/pods/nginx?describe=true"
        );
    }

    #[test]
    fn describe_with_namespace() {
        let mut a = args("pods", "nginx");
        a.namespace = Some("prod".into());
        let r = prepare(&a).unwrap();
        assert_eq!(
            r.path,
            "/api/compat/kubectl/v1/namespaces/prod/pods/nginx?describe=true"
        );
    }

    #[test]
    fn describe_short_alias() {
        let r = prepare(&args("po", "x")).unwrap();
        assert!(r.path.contains("/pods/x"));
    }

    #[test]
    fn describe_with_selector_only() {
        let mut a = args("pods", "");
        a.selector = Some("app=foo".into());
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("labelSelector=app=foo"));
    }

    #[test]
    fn describe_rejects_no_target() {
        let r = prepare(&args("pods", ""));
        assert!(r.is_err());
    }

    #[test]
    fn describe_uses_get() {
        let r = prepare(&args("pods", "x")).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn describe_path_compat_prefix() {
        let r = prepare(&args("pods", "x")).unwrap();
        assert!(r.path.starts_with("/api/compat/kubectl/"));
    }
}
