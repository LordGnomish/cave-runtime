// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl kubectl exec …`

use anyhow::{Result, bail};
use clap::Args;

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct KubectlExecArgs {
    pub pod: String,
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
    #[arg(short = 'c', long)]
    pub container: Option<String>,
    #[arg(short = 't', long)]
    pub tty: bool,
    #[arg(short = 'i', long)]
    pub stdin: bool,
    /// Command and arguments after `--`.
    #[arg(last = true)]
    pub command: Vec<String>,
}

pub fn prepare(args: &KubectlExecArgs) -> Result<PreparedRequest> {
    if args.pod.is_empty() {
        bail!("pod required");
    }
    let ns = args.namespace.as_deref().unwrap_or("default");
    let mut path = format!(
        "/api/compat/kubectl/v1/namespaces/{}/pods/{}/exec",
        ns, args.pod
    );
    let mut params: Vec<String> = Vec::new();
    if let Some(c) = &args.container {
        params.push(format!("container={}", c));
    }
    if args.tty {
        params.push("tty=true".to_string());
    }
    if args.stdin {
        params.push("stdin=true".to_string());
    }
    for arg in &args.command {
        params.push(format!("command={}", urlencode(arg)));
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    Ok(PreparedRequest::new(HttpVerb::Post, path))
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

    fn args(pod: &str) -> KubectlExecArgs {
        KubectlExecArgs {
            pod: pod.into(),
            namespace: None,
            container: None,
            tty: false,
            stdin: false,
            command: vec![],
        }
    }

    #[test]
    fn exec_default() {
        let r = prepare(&args("nginx")).unwrap();
        assert_eq!(
            r.path,
            "/api/compat/kubectl/v1/namespaces/default/pods/nginx/exec"
        );
    }

    #[test]
    fn exec_with_command() {
        let mut a = args("nginx");
        a.command = vec!["sh".into(), "-c".into(), "echo hi".into()];
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("command=sh"));
        assert!(r.path.contains("command=-c"));
        assert!(r.path.contains("command=echo%20hi"));
    }

    #[test]
    fn exec_tty_stdin() {
        let mut a = args("nginx");
        a.tty = true;
        a.stdin = true;
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("tty=true"));
        assert!(r.path.contains("stdin=true"));
    }

    #[test]
    fn exec_with_container() {
        let mut a = args("nginx");
        a.container = Some("sidecar".into());
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("container=sidecar"));
    }

    #[test]
    fn exec_with_namespace() {
        let mut a = args("nginx");
        a.namespace = Some("prod".into());
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("/namespaces/prod/"));
    }

    #[test]
    fn exec_uses_post() {
        let r = prepare(&args("nginx")).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
    }

    #[test]
    fn exec_rejects_empty_pod() {
        assert!(prepare(&args("")).is_err());
    }

    #[test]
    fn exec_compat_path_prefix() {
        let r = prepare(&args("x")).unwrap();
        assert!(r.path.starts_with("/api/compat/kubectl/"));
    }
}
