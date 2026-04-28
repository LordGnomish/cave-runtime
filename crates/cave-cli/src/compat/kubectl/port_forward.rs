//! `cavectl kubectl port-forward …`

use anyhow::{bail, Result};
use clap::Args;

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct KubectlPortForwardArgs {
    /// Resource — `<pod>` or `pod/<name>` or `svc/<name>`.
    pub target: String,
    /// One or more `local:remote` (or just `local`) port pairs.
    #[arg(required = true)]
    pub ports: Vec<String>,
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
    #[arg(long = "address", default_value = "127.0.0.1")]
    pub address: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortForwardKind {
    Pod,
    Service,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTarget {
    pub kind: PortForwardKind,
    pub name: String,
}

pub fn parse_target(s: &str) -> Result<ParsedTarget> {
    if s.is_empty() {
        bail!("target required");
    }
    if let Some((prefix, name)) = s.split_once('/') {
        if name.is_empty() {
            bail!("target name required after `/`");
        }
        let kind = match prefix {
            "pod" | "pods" | "po" => PortForwardKind::Pod,
            "svc" | "service" | "services" => PortForwardKind::Service,
            _ => bail!("unsupported port-forward kind `{}`", prefix),
        };
        return Ok(ParsedTarget {
            kind,
            name: name.to_string(),
        });
    }
    Ok(ParsedTarget {
        kind: PortForwardKind::Pod,
        name: s.to_string(),
    })
}

pub fn parse_port_pair(spec: &str) -> Result<(u16, u16)> {
    let parts: Vec<&str> = spec.split(':').collect();
    let (l, r) = match parts.as_slice() {
        [a, b] => (*a, *b),
        [a] => (*a, *a),
        _ => bail!("invalid port spec `{}` (want LOCAL:REMOTE)", spec),
    };
    let local: u16 = l
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid local port `{}`", l))?;
    let remote: u16 = r
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid remote port `{}`", r))?;
    Ok((local, remote))
}

pub fn prepare(args: &KubectlPortForwardArgs) -> Result<PreparedRequest> {
    let target = parse_target(&args.target)?;
    let ns = args.namespace.as_deref().unwrap_or("default");
    let resource = match target.kind {
        PortForwardKind::Pod => "pods",
        PortForwardKind::Service => "services",
    };
    let mut path = format!(
        "/api/compat/kubectl/v1/namespaces/{}/{}/{}/portforward",
        ns, resource, target.name
    );
    if args.ports.is_empty() {
        bail!("at least one port pair required");
    }
    let mut params: Vec<String> = Vec::new();
    for p in &args.ports {
        let (l, r) = parse_port_pair(p)?;
        params.push(format!("ports={}:{}", l, r));
    }
    if args.address != "127.0.0.1" {
        // localhost is the kubectl default; only thread non-default through.
        params.push(format!("address={}", args.address));
    }
    path.push('?');
    path.push_str(&params.join("&"));
    Ok(PreparedRequest::new(HttpVerb::Post, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(target: &str, ports: &[&str]) -> KubectlPortForwardArgs {
        KubectlPortForwardArgs {
            target: target.into(),
            ports: ports.iter().map(|s| s.to_string()).collect(),
            namespace: None,
            address: "127.0.0.1".into(),
        }
    }

    #[test]
    fn pf_pod_default() {
        let r = prepare(&args("nginx", &["8080:80"])).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(
            r.path,
            "/api/compat/kubectl/v1/namespaces/default/pods/nginx/portforward?ports=8080:80"
        );
    }

    #[test]
    fn pf_short_local_only() {
        let r = prepare(&args("nginx", &["9090"])).unwrap();
        assert!(r.path.contains("ports=9090:9090"));
    }

    #[test]
    fn pf_multiple_ports() {
        let r = prepare(&args("nginx", &["8080:80", "9090:9090"])).unwrap();
        let p = r.path;
        assert!(p.contains("ports=8080:80"));
        assert!(p.contains("ports=9090:9090"));
    }

    #[test]
    fn pf_namespace() {
        let mut a = args("nginx", &["80"]);
        a.namespace = Some("prod".into());
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("/namespaces/prod/"));
    }

    #[test]
    fn pf_service_target() {
        let r = prepare(&args("svc/api", &["8080:80"])).unwrap();
        assert!(r.path.contains("/services/api/"));
    }

    #[test]
    fn pf_pod_explicit_prefix() {
        let r = prepare(&args("pod/nginx", &["80"])).unwrap();
        assert!(r.path.contains("/pods/nginx/"));
    }

    #[test]
    fn pf_short_pod_alias() {
        let r = prepare(&args("po/nginx", &["80"])).unwrap();
        assert!(r.path.contains("/pods/nginx/"));
    }

    #[test]
    fn pf_unknown_kind_rejected() {
        assert!(prepare(&args("deploy/x", &["80"])).is_err());
    }

    #[test]
    fn pf_empty_target() {
        assert!(prepare(&args("", &["80"])).is_err());
    }

    #[test]
    fn pf_empty_after_slash_rejected() {
        assert!(prepare(&args("pod/", &["80"])).is_err());
    }

    #[test]
    fn pf_invalid_port_rejected() {
        assert!(prepare(&args("nginx", &["notaport:80"])).is_err());
    }

    #[test]
    fn pf_three_part_port_rejected() {
        assert!(prepare(&args("nginx", &["a:b:c"])).is_err());
    }

    #[test]
    fn pf_address_default_omitted() {
        let r = prepare(&args("nginx", &["80"])).unwrap();
        assert!(!r.path.contains("address="));
    }

    #[test]
    fn pf_address_non_default_threaded() {
        let mut a = args("nginx", &["80"]);
        a.address = "0.0.0.0".into();
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("address=0.0.0.0"));
    }

    #[test]
    fn pf_uses_post() {
        let r = prepare(&args("nginx", &["80"])).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
    }

    #[test]
    fn pf_compat_path_prefix() {
        let r = prepare(&args("nginx", &["80"])).unwrap();
        assert!(r.path.starts_with("/api/compat/kubectl/"));
    }

    #[test]
    fn parse_target_bare_name_is_pod() {
        let t = parse_target("nginx").unwrap();
        assert_eq!(t.kind, PortForwardKind::Pod);
        assert_eq!(t.name, "nginx");
    }

    #[test]
    fn parse_port_pair_short_form() {
        let (l, r) = parse_port_pair("8080").unwrap();
        assert_eq!((l, r), (8080, 8080));
    }
}
