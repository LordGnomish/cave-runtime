// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl kubectl get …` — kubectl-flag mapping onto compat path.

use anyhow::Result;
use clap::Args;

use crate::compat::kubectl::output::{parse as parse_output, KubectlOutput};
use crate::compat::kubectl::resource::ns_path;
use crate::native::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct KubectlGetArgs {
    pub resource: String,
    pub name: Option<String>,
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
    #[arg(short = 'A', long = "all-namespaces")]
    pub all_namespaces: bool,
    #[arg(short = 'l', long = "selector")]
    pub selector: Option<String>,
    #[arg(short = 'w', long)]
    pub watch: bool,
    #[arg(short = 'o', long = "output")]
    pub output: Option<String>,
    /// `--field-selector status.phase=Running`
    #[arg(long = "field-selector")]
    pub field_selector: Option<String>,
}

pub fn prepare(args: &KubectlGetArgs) -> Result<(PreparedRequest, KubectlOutput)> {
    let output = parse_output(args.output.as_deref())?;
    let mut path = ns_path(
        &args.resource,
        args.namespace.as_deref(),
        args.all_namespaces,
    )?;
    if let Some(name) = &args.name {
        path.push('/');
        path.push_str(name);
    }
    let mut params: Vec<String> = Vec::new();
    if let Some(s) = &args.selector {
        params.push(format!("labelSelector={}", urlencode(s)));
    }
    if let Some(s) = &args.field_selector {
        params.push(format!("fieldSelector={}", urlencode(s)));
    }
    if args.watch {
        params.push("watch=true".to_string());
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    Ok((PreparedRequest::new(HttpVerb::Get, path), output))
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{:02X}", b);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(resource: &str) -> KubectlGetArgs {
        KubectlGetArgs {
            resource: resource.into(),
            name: None,
            namespace: None,
            all_namespaces: false,
            selector: None,
            watch: false,
            output: None,
            field_selector: None,
        }
    }

    #[test]
    fn get_pods_default() {
        let (r, o) = prepare(&args("pods")).unwrap();
        assert_eq!(r.path, "/api/compat/kubectl/v1/namespaces/default/pods");
        assert_eq!(o, KubectlOutput::Wide);
    }

    #[test]
    fn get_pods_namespace() {
        let mut a = args("pods");
        a.namespace = Some("kube-system".into());
        let (r, _) = prepare(&a).unwrap();
        assert_eq!(
            r.path,
            "/api/compat/kubectl/v1/namespaces/kube-system/pods"
        );
    }

    #[test]
    fn get_pods_all_namespaces() {
        let mut a = args("pods");
        a.all_namespaces = true;
        let (r, _) = prepare(&a).unwrap();
        assert_eq!(r.path, "/api/compat/kubectl/v1/pods");
    }

    #[test]
    fn get_named_pod() {
        let mut a = args("pods");
        a.name = Some("nginx".into());
        let (r, _) = prepare(&a).unwrap();
        assert_eq!(
            r.path,
            "/api/compat/kubectl/v1/namespaces/default/pods/nginx"
        );
    }

    #[test]
    fn get_short_name_pods() {
        let (r, _) = prepare(&args("po")).unwrap();
        assert!(r.path.ends_with("/pods"));
    }

    #[test]
    fn get_with_selector() {
        let mut a = args("pods");
        a.selector = Some("app=foo".into());
        let (r, _) = prepare(&a).unwrap();
        assert!(r.path.contains("labelSelector=app%3Dfoo"));
    }

    #[test]
    fn get_with_field_selector() {
        let mut a = args("pods");
        a.field_selector = Some("status.phase=Running".into());
        let (r, _) = prepare(&a).unwrap();
        assert!(r.path.contains("fieldSelector="));
    }

    #[test]
    fn get_with_watch() {
        let mut a = args("pods");
        a.watch = true;
        let (r, _) = prepare(&a).unwrap();
        assert!(r.path.contains("watch=true"));
    }

    #[test]
    fn get_output_yaml() {
        let mut a = args("pods");
        a.output = Some("yaml".into());
        let (_, o) = prepare(&a).unwrap();
        assert_eq!(o, KubectlOutput::Yaml);
    }

    #[test]
    fn get_output_json() {
        let mut a = args("pods");
        a.output = Some("json".into());
        let (_, o) = prepare(&a).unwrap();
        assert_eq!(o, KubectlOutput::Json);
    }

    #[test]
    fn get_output_name() {
        let mut a = args("pods");
        a.output = Some("name".into());
        let (_, o) = prepare(&a).unwrap();
        assert_eq!(o, KubectlOutput::Name);
    }

    #[test]
    fn get_output_jsonpath() {
        let mut a = args("pods");
        a.output = Some("jsonpath={.items[0].metadata.name}".into());
        let (_, o) = prepare(&a).unwrap();
        match o {
            KubectlOutput::Jsonpath(s) => assert!(s.contains("items")),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn get_uses_get() {
        let (r, _) = prepare(&args("pods")).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn get_no_body() {
        let (r, _) = prepare(&args("pods")).unwrap();
        assert!(r.body.is_none());
    }

    #[test]
    fn get_combined_flags() {
        let mut a = args("pods");
        a.namespace = Some("prod".into());
        a.selector = Some("tier=web".into());
        a.watch = true;
        a.output = Some("yaml".into());
        let (r, o) = prepare(&a).unwrap();
        assert_eq!(o, KubectlOutput::Yaml);
        assert!(r
            .path
            .starts_with("/api/compat/kubectl/v1/namespaces/prod/pods?"));
        assert!(r.path.contains("labelSelector=tier%3Dweb"));
        assert!(r.path.contains("watch=true"));
    }

    #[test]
    fn get_path_starts_with_compat_prefix() {
        let (r, _) = prepare(&args("pods")).unwrap();
        assert!(r.path.starts_with("/api/compat/kubectl/"));
    }
}
