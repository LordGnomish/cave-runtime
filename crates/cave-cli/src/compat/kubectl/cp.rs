//! `cavectl kubectl cp <src> <dst>` — copy files in/out of pods.

use anyhow::{bail, Result};
use clap::Args;

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct KubectlCpArgs {
    pub src: String,
    pub dst: String,
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
    #[arg(short = 'c', long)]
    pub container: Option<String>,
    /// Don't preserve file permissions (kubectl `--no-preserve`).
    #[arg(long = "no-preserve")]
    pub no_preserve: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpDirection {
    /// `cp <local> <pod>:<path>`
    Into,
    /// `cp <pod>:<path> <local>`
    Out,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpPlan {
    pub pod: String,
    pub direction: CpDirection,
    pub local_path: String,
    pub pod_path: String,
}

pub fn parse_plan(src: &str, dst: &str) -> Result<CpPlan> {
    let src_pod = pod_ref(src);
    let dst_pod = pod_ref(dst);
    match (src_pod, dst_pod) {
        (Some(_), Some(_)) => bail!("cp does not support pod-to-pod copies"),
        (None, None) => bail!("cp needs at least one pod reference (pod:/path)"),
        (Some((pod, src_path)), None) => Ok(CpPlan {
            pod: pod.to_string(),
            direction: CpDirection::Out,
            local_path: dst.to_string(),
            pod_path: src_path.to_string(),
        }),
        (None, Some((pod, dst_path))) => Ok(CpPlan {
            pod: pod.to_string(),
            direction: CpDirection::Into,
            local_path: src.to_string(),
            pod_path: dst_path.to_string(),
        }),
    }
}

/// Returns `(pod, path)` if the string is a pod-ref of the form
/// `<pod>:<path>` and the prefix is plausibly a pod name (not a
/// Windows drive letter, not an absolute path with `/`).
fn pod_ref(s: &str) -> Option<(&str, &str)> {
    let (pod, path) = s.split_once(':')?;
    if pod.is_empty()
        || pod.contains('/')
        || pod.contains('\\')
        || (pod.len() == 1 && pod.chars().next().unwrap().is_ascii_alphabetic())
    {
        return None;
    }
    Some((pod, path))
}

pub fn prepare(args: &KubectlCpArgs) -> Result<(PreparedRequest, CpPlan)> {
    let plan = parse_plan(&args.src, &args.dst)?;
    let ns = args.namespace.as_deref().unwrap_or("default");
    let verb_segment = match plan.direction {
        CpDirection::Into => "cp-in",
        CpDirection::Out => "cp-out",
    };
    let mut path = format!(
        "/api/compat/kubectl/v1/namespaces/{}/pods/{}/{}",
        ns, plan.pod, verb_segment
    );
    let mut params: Vec<String> = Vec::new();
    if let Some(c) = &args.container {
        params.push(format!("container={}", c));
    }
    if args.no_preserve {
        params.push("preserve=false".to_string());
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    Ok((PreparedRequest::new(HttpVerb::Post, path), plan))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(src: &str, dst: &str) -> KubectlCpArgs {
        KubectlCpArgs {
            src: src.into(),
            dst: dst.into(),
            namespace: None,
            container: None,
            no_preserve: false,
        }
    }

    #[test]
    fn cp_into_pod() {
        let (r, plan) = prepare(&args("/tmp/x", "nginx:/etc/x")).unwrap();
        assert_eq!(plan.direction, CpDirection::Into);
        assert_eq!(plan.pod, "nginx");
        assert_eq!(plan.local_path, "/tmp/x");
        assert_eq!(plan.pod_path, "/etc/x");
        assert!(r.path.ends_with("/cp-in"));
    }

    #[test]
    fn cp_out_of_pod() {
        let (r, plan) = prepare(&args("nginx:/var/log", "/tmp/log")).unwrap();
        assert_eq!(plan.direction, CpDirection::Out);
        assert!(r.path.ends_with("/cp-out"));
    }

    #[test]
    fn cp_with_namespace() {
        let mut a = args("/tmp/x", "nginx:/etc/x");
        a.namespace = Some("prod".into());
        let (r, _) = prepare(&a).unwrap();
        assert!(r.path.contains("/namespaces/prod/pods/nginx/"));
    }

    #[test]
    fn cp_with_container() {
        let mut a = args("/tmp/x", "nginx:/etc/x");
        a.container = Some("sidecar".into());
        let (r, _) = prepare(&a).unwrap();
        assert!(r.path.contains("container=sidecar"));
    }

    #[test]
    fn cp_no_preserve_flag() {
        let mut a = args("/tmp/x", "nginx:/etc/x");
        a.no_preserve = true;
        let (r, _) = prepare(&a).unwrap();
        assert!(r.path.contains("preserve=false"));
    }

    #[test]
    fn cp_pod_to_pod_rejected() {
        assert!(prepare(&args("a:/x", "b:/y")).is_err());
    }

    #[test]
    fn cp_local_to_local_rejected() {
        assert!(prepare(&args("/a", "/b")).is_err());
    }

    #[test]
    fn cp_uses_post() {
        let (r, _) = prepare(&args("/tmp/x", "nginx:/etc/x")).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
    }

    #[test]
    fn cp_compat_path_prefix() {
        let (r, _) = prepare(&args("/tmp/x", "nginx:/etc/x")).unwrap();
        assert!(r.path.starts_with("/api/compat/kubectl/"));
    }

    #[test]
    fn cp_treats_drive_letter_as_local() {
        // `c:/Users/...` shouldn't parse as pod `c`.
        let res = prepare(&args("c:/Users/a", "nginx:/etc/x"));
        // Single-char `c:` is treated as drive, so src is local, dst pod.
        let (_, plan) = res.unwrap();
        assert_eq!(plan.direction, CpDirection::Into);
        assert_eq!(plan.local_path, "c:/Users/a");
    }

    #[test]
    fn cp_treats_path_with_slash_as_local() {
        // `dir/file:foo` — colon comes after a `/`, so prefix has `/`,
        // not a pod-ref.
        let (_, plan) = prepare(&args("dir/file:foo", "nginx:/etc/x")).unwrap();
        assert_eq!(plan.direction, CpDirection::Into);
    }

    #[test]
    fn cp_default_namespace() {
        let (r, _) = prepare(&args("/tmp/x", "nginx:/etc/x")).unwrap();
        assert!(r.path.contains("/namespaces/default/"));
    }

    #[test]
    fn cp_no_query_when_no_extras() {
        let (r, _) = prepare(&args("/tmp/x", "nginx:/etc/x")).unwrap();
        assert!(!r.path.contains('?'));
    }
}
