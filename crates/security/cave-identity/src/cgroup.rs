// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). The cgroup pod-UID +
// container-ID extraction is line-ported from
// pkg/agent/plugin/workloadattestor/k8s/k8s_posix.go (the `cgroupREs`
// matchers + getPodUIDAndContainerIDFromCGroups). cgroup-v1 and cgroup-v2
// (`0::/...`) line shapes are both handled. The actual /proc/<pid>/cgroup
// read is performed by cave-cri / the agent host; this module owns the pure
// parsing so the k8s workload attestor can derive container identity when the
// kubelet summary does not pre-populate it.
//
//! cgroup → (pod UID, container ID) extraction.

use crate::error::{IdentityError, Result};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

use regex::Regex;

/// Outcome of parsing a process's cgroup membership.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContainerInfo {
    /// Canonicalised pod UID (dashes), when the runtime encodes it in the
    /// cgroup path. CRI-O and some others omit it.
    pub pod_uid: Option<String>,
    /// 64-hex container ID.
    pub container_id: String,
}

/// Regex matching `pod<uid>...<containerid>` (most runtimes: containerd,
/// docker-under-kubelet, systemd-cgroup slices).
fn re_with_pod() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"[^[:alnum:]]pod(?P<poduid>[[:xdigit:]]{8}[-_][[:xdigit:]]{4}[-_]",
            r"[[:xdigit:]]{4}[-_][[:xdigit:]]{4}[-_][[:xdigit:]]{12})",
            r".*[^[:alnum:]](?P<containerid>[[:xdigit:]]{64})$"
        ))
        .expect("valid pod+container cgroup regex")
    })
}

/// Regex matching a bare 64-hex container ID at the end of the path (CRI-O,
/// KubeEdge — runtimes that omit the pod UID).
fn re_container_only() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:^|[^[:alnum:]])(?P<containerid>[[:xdigit:]]{64})$")
            .expect("valid container-only cgroup regex")
    })
}

/// Return the path component of a single `/proc/<pid>/cgroup` line.
///
/// cgroup-v1 lines are `hierarchy-ID:controller-list:cgroup-path`; cgroup-v2
/// lines are `0::/cgroup-path`. Returns `None` for malformed lines.
pub fn cgroup_path(line: &str) -> Option<&str> {
    let mut it = line.splitn(3, ':');
    let _hierarchy = it.next()?;
    let _controllers = it.next()?;
    it.next()
}

/// Extract `(pod_uid, container_id)` from the full `/proc/<pid>/cgroup` body.
///
/// Mirrors SPIRE `getPodUIDAndContainerIDFromCGroups`: every matching line
/// must agree on the IDs; conflicting matches are an error, as is the absence
/// of any container ID.
pub fn extract(cgroup_content: &str) -> Result<ContainerInfo> {
    let mut found: Option<ContainerInfo> = None;
    for line in cgroup_content.lines() {
        let Some(path) = cgroup_path(line) else {
            continue;
        };
        // Trim a trailing systemd ".scope" before matching, exactly as SPIRE
        // does prior to applying the cgroup regexes.
        let path = path.strip_suffix(".scope").unwrap_or(path);

        let info = if let Some(caps) = re_with_pod().captures(path) {
            ContainerInfo {
                pod_uid: Some(canonicalize_pod_uid(&caps["poduid"])),
                container_id: caps["containerid"].to_string(),
            }
        } else if let Some(caps) = re_container_only().captures(path) {
            ContainerInfo {
                pod_uid: None,
                container_id: caps["containerid"].to_string(),
            }
        } else {
            continue;
        };

        match &found {
            None => found = Some(info),
            Some(prev) => {
                if prev.container_id != info.container_id {
                    return Err(IdentityError::AttestationFailed(format!(
                        "cgroup: conflicting container ids ({} != {})",
                        prev.container_id, info.container_id
                    )));
                }
                match (&prev.pod_uid, &info.pod_uid) {
                    // A later line that carries the pod UID supersedes a bare
                    // container-only match.
                    (None, Some(_)) => found = Some(info),
                    (Some(a), Some(b)) if a != b => {
                        return Err(IdentityError::AttestationFailed(format!(
                            "cgroup: conflicting pod uids ({} != {})",
                            a, b
                        )));
                    }
                    _ => {}
                }
            }
        }
    }
    found.ok_or_else(|| {
        IdentityError::AttestationFailed("cgroup: no container id found".into())
    })
}

/// Canonicalise a pod UID by converting every non-alphanumeric separator to a
/// dash (`2c48913c_b29f_...` → `2c48913c-b29f-...`), matching SPIRE's
/// `canonicalizePodUID`.
pub fn canonicalize_pod_uid(uid: &str) -> String {
    uid.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CID: &str = "9bca8d63d5fa610783847915bcff0ecac1273e5b4bed3f6fa1b07350e0135961";

    #[test]
    fn path_v1_three_fields() {
        assert_eq!(
            cgroup_path("11:devices:/kubepods/besteffort/podABC/xyz"),
            Some("/kubepods/besteffort/podABC/xyz")
        );
    }

    #[test]
    fn path_v2_zero_colon_colon() {
        assert_eq!(
            cgroup_path("0::/system.slice/crio-abc.scope"),
            Some("/system.slice/crio-abc.scope")
        );
    }

    #[test]
    fn path_malformed_is_none() {
        assert_eq!(cgroup_path("garbage-no-colons"), None);
    }

    #[test]
    fn canonicalize_underscores_to_dashes() {
        assert_eq!(
            canonicalize_pod_uid("2c48913c_b29f_11e7_9350_020968147796"),
            "2c48913c-b29f-11e7-9350-020968147796"
        );
    }

    #[test]
    fn containerd_v1_pod_and_container() {
        let content = format!(
            "11:devices:/kubepods/besteffort/pod2c48913c-b29f-11e7-9350-020968147796/{}",
            CID
        );
        let info = extract(&content).unwrap();
        assert_eq!(
            info.pod_uid.as_deref(),
            Some("2c48913c-b29f-11e7-9350-020968147796")
        );
        assert_eq!(info.container_id, CID);
    }

    #[test]
    fn systemd_underscore_pod_uid_canonicalised() {
        let content = format!(
            "1:name=systemd:/kubepods.slice/kubepods-burstable.slice/kubepods-burstable-pod2c48913c_b29f_11e7_9350_020968147796.slice/cri-containerd-{}.scope",
            CID
        );
        let info = extract(&content).unwrap();
        assert_eq!(
            info.pod_uid.as_deref(),
            Some("2c48913c-b29f-11e7-9350-020968147796")
        );
        assert_eq!(info.container_id, CID);
    }

    #[test]
    fn crio_container_only_no_pod_uid() {
        let content = format!("0::/system.slice/crio-{}.scope", CID);
        let info = extract(&content).unwrap();
        assert_eq!(info.pod_uid, None);
        assert_eq!(info.container_id, CID);
    }

    #[test]
    fn docker_v1_container_only() {
        let content = format!("4:devices:/docker/{}", CID);
        let info = extract(&content).unwrap();
        assert_eq!(info.pod_uid, None);
        assert_eq!(info.container_id, CID);
    }

    #[test]
    fn multiple_lines_agree() {
        let content = format!(
            "12:cpu:/kubepods/besteffort/pod2c48913c-b29f-11e7-9350-020968147796/{cid}\n11:devices:/kubepods/besteffort/pod2c48913c-b29f-11e7-9350-020968147796/{cid}",
            cid = CID
        );
        let info = extract(&content).unwrap();
        assert_eq!(info.container_id, CID);
        assert_eq!(
            info.pod_uid.as_deref(),
            Some("2c48913c-b29f-11e7-9350-020968147796")
        );
    }

    #[test]
    fn conflicting_container_ids_error() {
        let other = "1111111111111111111111111111111111111111111111111111111111111111";
        let content = format!(
            "12:cpu:/kubepods/besteffort/pod2c48913c-b29f-11e7-9350-020968147796/{}\n11:devices:/docker/{}",
            CID, other
        );
        assert!(extract(&content).is_err());
    }

    #[test]
    fn no_container_id_errors() {
        let content = "9:memory:/user.slice\n0::/init.scope";
        assert!(extract(content).is_err());
    }

    #[test]
    fn ignores_non_container_lines() {
        let content = format!(
            "9:memory:/user.slice\n11:devices:/kubepods/besteffort/pod2c48913c-b29f-11e7-9350-020968147796/{}",
            CID
        );
        let info = extract(&content).unwrap();
        assert_eq!(info.container_id, CID);
    }
}
