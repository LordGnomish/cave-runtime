//! Security context materialization — seccomp, capabilities, runAsUser,
//! SELinux, fsGroup, privileged, allowPrivilegeEscalation, procMount.
//!
//! Mirrors `pkg/securitycontext/accessors.go` + `pkg/kubelet/kuberuntime/
//! security_context.go`: pod-level → container-level merging with the
//! container value winning, validation rules that the kubelet enforces
//! before admission, and seccomp profile materialization (RuntimeDefault /
//! Unconfined / Localhost) into the container runtime's expected form.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeccompProfileType {
    Unconfined,
    RuntimeDefault,
    /// Path under `--root-dir/seccomp/`, validated.
    Localhost(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodSecurityContext {
    pub run_as_user: Option<u32>,
    pub run_as_group: Option<u32>,
    pub run_as_non_root: Option<bool>,
    pub fs_group: Option<u32>,
    pub fs_group_change_policy: Option<FsGroupChangePolicy>,
    pub supplemental_groups: Vec<u32>,
    pub se_linux_options: Option<SeLinuxOptions>,
    pub seccomp_profile: Option<SeccompProfileType>,
    pub sysctls: Vec<(String, String)>,
    pub windows_options: Option<WindowsSecurityContextOptions>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerSecurityContext {
    pub run_as_user: Option<u32>,
    pub run_as_group: Option<u32>,
    pub run_as_non_root: Option<bool>,
    pub privileged: Option<bool>,
    pub allow_privilege_escalation: Option<bool>,
    pub read_only_root_filesystem: Option<bool>,
    pub se_linux_options: Option<SeLinuxOptions>,
    pub seccomp_profile: Option<SeccompProfileType>,
    pub capabilities: Option<Capabilities>,
    pub proc_mount: Option<ProcMountType>,
    pub windows_options: Option<WindowsSecurityContextOptions>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FsGroupChangePolicy {
    /// Always recursively chown.
    Always,
    /// Only chown if top-level dir mismatch.
    OnRootMismatch,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeLinuxOptions {
    pub user: Option<String>,
    pub role: Option<String>,
    pub type_: Option<String>,
    pub level: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub add: Vec<String>,
    pub drop: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcMountType {
    /// Standard masking of /proc paths.
    Default,
    /// Unmasked /proc — requires UserNamespace gate.
    Unmasked,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsSecurityContextOptions {
    pub run_as_user_name: Option<String>,
    pub host_process: Option<bool>,
    pub gmsa_credential_spec: Option<String>,
}

/// Materialised view that the runtime layer can act on directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveSecurityContext {
    pub run_as_user: Option<u32>,
    pub run_as_group: Option<u32>,
    pub run_as_non_root: bool,
    pub privileged: bool,
    pub allow_privilege_escalation: bool,
    pub read_only_root_filesystem: bool,
    pub se_linux_options: SeLinuxOptions,
    pub seccomp_profile: SeccompProfileType,
    pub capabilities: Capabilities,
    pub proc_mount: ProcMountType,
    pub fs_group: Option<u32>,
    pub fs_group_change_policy: FsGroupChangePolicy,
    pub supplemental_groups: Vec<u32>,
    pub sysctls: Vec<(String, String)>,
    pub windows: Option<WindowsSecurityContextOptions>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecurityError {
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
}

pub type SecResult<T> = Result<T, SecurityError>;

/// Merge pod-level + container-level SecurityContext into the effective
/// context the runtime is given. Container values win; pod-level fills the
/// gaps. Defaults match upstream:
///   privileged=false, allowPrivilegeEscalation=true (false when SELinux
///   restricted, but kubelet itself defaults to true), readOnlyRoot=false,
///   runAsNonRoot=false, procMount=Default, seccomp=Unconfined,
///   fsGroupChangePolicy=Always.
pub fn materialize(
    pod: &PodSecurityContext,
    container: &ContainerSecurityContext,
) -> SecResult<EffectiveSecurityContext> {
    // Merge.
    let run_as_user = container.run_as_user.or(pod.run_as_user);
    let run_as_group = container.run_as_group.or(pod.run_as_group);
    let run_as_non_root = container.run_as_non_root.or(pod.run_as_non_root).unwrap_or(false);
    let privileged = container.privileged.unwrap_or(false);
    let allow_privilege_escalation = container.allow_privilege_escalation.unwrap_or(true);
    let read_only_root_filesystem = container.read_only_root_filesystem.unwrap_or(false);
    let se_linux = container
        .se_linux_options
        .clone()
        .or_else(|| pod.se_linux_options.clone())
        .unwrap_or_default();
    let seccomp = container
        .seccomp_profile
        .clone()
        .or_else(|| pod.seccomp_profile.clone())
        .unwrap_or(SeccompProfileType::RuntimeDefault);
    let capabilities = container.capabilities.clone().unwrap_or_default();
    let proc_mount = container.proc_mount.unwrap_or(ProcMountType::Default);
    let fs_group = pod.fs_group;
    let fs_group_change_policy = pod
        .fs_group_change_policy
        .unwrap_or(FsGroupChangePolicy::Always);
    let supplemental_groups = pod.supplemental_groups.clone();
    let sysctls = pod.sysctls.clone();
    let windows = container
        .windows_options
        .clone()
        .or_else(|| pod.windows_options.clone());

    let effective = EffectiveSecurityContext {
        run_as_user,
        run_as_group,
        run_as_non_root,
        privileged,
        allow_privilege_escalation,
        read_only_root_filesystem,
        se_linux_options: se_linux,
        seccomp_profile: seccomp,
        capabilities,
        proc_mount,
        fs_group,
        fs_group_change_policy,
        supplemental_groups,
        sysctls,
        windows,
    };
    validate(&effective)?;
    Ok(effective)
}

/// Kubelet admission validation — rules that, if violated, cause the kubelet
/// to reject the pod with `CreateContainerConfigError`.
pub fn validate(ctx: &EffectiveSecurityContext) -> SecResult<()> {
    // runAsNonRoot + runAsUser=0 → fail at admission (would also fail at
    // container start). Upstream: pkg/kubelet/kuberuntime/security_context.go
    if ctx.run_as_non_root {
        if let Some(0) = ctx.run_as_user {
            return Err(SecurityError::Forbidden(
                "runAsNonRoot=true incompatible with runAsUser=0".into(),
            ));
        }
    }
    // allowPrivilegeEscalation=false + privileged=true → invalid.
    if ctx.privileged && !ctx.allow_privilege_escalation {
        return Err(SecurityError::Invalid(
            "privileged=true requires allowPrivilegeEscalation=true".into(),
        ));
    }
    // CAP_SYS_ADMIN add → kubelet allows but treats as privilege-escalation;
    // explicit allowPrivilegeEscalation=false then makes it inconsistent.
    if !ctx.allow_privilege_escalation
        && ctx.capabilities.add.iter().any(|c| c == "SYS_ADMIN" || c == "CAP_SYS_ADMIN")
    {
        return Err(SecurityError::Invalid(
            "CAP_SYS_ADMIN requires allowPrivilegeEscalation=true".into(),
        ));
    }
    // procMount=Unmasked requires privileged or UserNamespace; when neither
    // is set, kubelet rejects (the alpha gate hosts this; we encode the
    // privileged-fallback path).
    if ctx.proc_mount == ProcMountType::Unmasked && !ctx.privileged {
        return Err(SecurityError::Forbidden(
            "procMount=Unmasked requires privileged=true (or UserNamespace gate)".into(),
        ));
    }
    // Seccomp profile validation.
    validate_seccomp(&ctx.seccomp_profile)?;
    // Capability validation.
    validate_capabilities(&ctx.capabilities)?;
    // Windows + Linux fields are mutually exclusive.
    if ctx.windows.is_some() && (ctx.run_as_user.is_some() || ctx.run_as_group.is_some()) {
        return Err(SecurityError::Invalid(
            "windowsOptions cannot be combined with runAsUser/runAsGroup".into(),
        ));
    }
    Ok(())
}

pub fn validate_seccomp(p: &SeccompProfileType) -> SecResult<()> {
    if let SeccompProfileType::Localhost(path) = p {
        if path.is_empty() {
            return Err(SecurityError::Invalid(
                "seccomp Localhost requires localhostProfile".into(),
            ));
        }
        if path.starts_with('/') {
            return Err(SecurityError::Invalid(
                "localhostProfile must be relative to kubelet seccomp dir".into(),
            ));
        }
        if path.split('/').any(|c| c == "..") {
            return Err(SecurityError::Invalid(
                "localhostProfile must not contain '..'".into(),
            ));
        }
    }
    Ok(())
}

/// Recognised Linux capabilities. ALL is a sentinel meaning every capability.
const KNOWN_CAPS: &[&str] = &[
    "ALL",
    "AUDIT_CONTROL",
    "AUDIT_READ",
    "AUDIT_WRITE",
    "BLOCK_SUSPEND",
    "BPF",
    "CHECKPOINT_RESTORE",
    "CHOWN",
    "DAC_OVERRIDE",
    "DAC_READ_SEARCH",
    "FOWNER",
    "FSETID",
    "IPC_LOCK",
    "IPC_OWNER",
    "KILL",
    "LEASE",
    "LINUX_IMMUTABLE",
    "MAC_ADMIN",
    "MAC_OVERRIDE",
    "MKNOD",
    "NET_ADMIN",
    "NET_BIND_SERVICE",
    "NET_BROADCAST",
    "NET_RAW",
    "PERFMON",
    "SETFCAP",
    "SETGID",
    "SETPCAP",
    "SETUID",
    "SYS_ADMIN",
    "SYS_BOOT",
    "SYS_CHROOT",
    "SYS_MODULE",
    "SYS_NICE",
    "SYS_PACCT",
    "SYS_PTRACE",
    "SYS_RAWIO",
    "SYS_RESOURCE",
    "SYS_TIME",
    "SYS_TTY_CONFIG",
    "SYSLOG",
    "WAKE_ALARM",
];

pub fn is_known_capability(name: &str) -> bool {
    let bare = name.trim_start_matches("CAP_");
    KNOWN_CAPS.contains(&bare)
}

pub fn validate_capabilities(c: &Capabilities) -> SecResult<()> {
    for cap in c.add.iter().chain(c.drop.iter()) {
        if !is_known_capability(cap) {
            return Err(SecurityError::Invalid(format!(
                "unknown capability: {}",
                cap
            )));
        }
    }
    // Add and drop must be disjoint.
    let add: BTreeSet<_> = c.add.iter().map(|s| normalize_cap(s)).collect();
    let drop: BTreeSet<_> = c.drop.iter().map(|s| normalize_cap(s)).collect();
    let intersection: Vec<_> = add.intersection(&drop).collect();
    if !intersection.is_empty() {
        return Err(SecurityError::Invalid(format!(
            "capability appears in both add and drop: {:?}",
            intersection
        )));
    }
    Ok(())
}

pub fn normalize_cap(name: &str) -> String {
    let bare = name.trim_start_matches("CAP_");
    bare.to_string()
}

/// Materialise seccomp profile into the runtime's expected JSON path.
/// RuntimeDefault → `runtime/default`; Unconfined → `unconfined`; Localhost
/// → `localhost/<resolved-path>` after sandboxing.
pub fn materialize_seccomp_path(profile: &SeccompProfileType, kubelet_seccomp_dir: &str) -> SecResult<String> {
    match profile {
        SeccompProfileType::RuntimeDefault => Ok("runtime/default".into()),
        SeccompProfileType::Unconfined => Ok("unconfined".into()),
        SeccompProfileType::Localhost(path) => {
            validate_seccomp(profile)?;
            let dir = kubelet_seccomp_dir.trim_end_matches('/');
            Ok(format!("localhost/{}/{}", dir, path))
        }
    }
}

/// Returns the supplementalGroups merged with run_as_group, sorted, deduped.
pub fn effective_groups(ctx: &EffectiveSecurityContext) -> Vec<u32> {
    let mut g: BTreeSet<u32> = ctx.supplemental_groups.iter().copied().collect();
    if let Some(rg) = ctx.run_as_group {
        g.insert(rg);
    }
    g.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod_default() -> PodSecurityContext {
        PodSecurityContext::default()
    }

    fn cont_default() -> ContainerSecurityContext {
        ContainerSecurityContext::default()
    }

    #[test]
    fn materialize_defaults_when_both_empty() {
        let e = materialize(&pod_default(), &cont_default()).unwrap();
        assert_eq!(e.privileged, false);
        assert_eq!(e.allow_privilege_escalation, true);
        assert_eq!(e.read_only_root_filesystem, false);
        assert_eq!(e.run_as_non_root, false);
        assert_eq!(e.proc_mount, ProcMountType::Default);
        assert_eq!(e.seccomp_profile, SeccompProfileType::RuntimeDefault);
        assert_eq!(e.fs_group_change_policy, FsGroupChangePolicy::Always);
    }

    #[test]
    fn container_overrides_pod_run_as_user() {
        let mut pod = pod_default();
        pod.run_as_user = Some(1000);
        let mut cont = cont_default();
        cont.run_as_user = Some(2000);
        let e = materialize(&pod, &cont).unwrap();
        assert_eq!(e.run_as_user, Some(2000));
    }

    #[test]
    fn pod_run_as_user_inherited_when_container_unset() {
        let mut pod = pod_default();
        pod.run_as_user = Some(1000);
        let e = materialize(&pod, &cont_default()).unwrap();
        assert_eq!(e.run_as_user, Some(1000));
    }

    #[test]
    fn run_as_non_root_inherits_from_pod() {
        let mut pod = pod_default();
        pod.run_as_non_root = Some(true);
        pod.run_as_user = Some(1000);
        let e = materialize(&pod, &cont_default()).unwrap();
        assert!(e.run_as_non_root);
    }

    #[test]
    fn run_as_non_root_with_run_as_user_zero_rejected() {
        let mut cont = cont_default();
        cont.run_as_non_root = Some(true);
        cont.run_as_user = Some(0);
        let err = materialize(&pod_default(), &cont).unwrap_err();
        assert!(matches!(err, SecurityError::Forbidden(_)));
    }

    #[test]
    fn privileged_with_allow_priv_escalation_false_rejected() {
        let mut cont = cont_default();
        cont.privileged = Some(true);
        cont.allow_privilege_escalation = Some(false);
        let err = materialize(&pod_default(), &cont).unwrap_err();
        assert!(matches!(err, SecurityError::Invalid(_)));
    }

    #[test]
    fn cap_sys_admin_with_allow_priv_escalation_false_rejected() {
        let mut cont = cont_default();
        cont.allow_privilege_escalation = Some(false);
        cont.capabilities = Some(Capabilities {
            add: vec!["SYS_ADMIN".into()],
            drop: vec![],
        });
        let err = materialize(&pod_default(), &cont).unwrap_err();
        assert!(matches!(err, SecurityError::Invalid(_)));
    }

    #[test]
    fn cap_with_cap_prefix_normalised() {
        let mut cont = cont_default();
        cont.allow_privilege_escalation = Some(false);
        cont.capabilities = Some(Capabilities {
            add: vec!["CAP_SYS_ADMIN".into()],
            drop: vec![],
        });
        let err = materialize(&pod_default(), &cont).unwrap_err();
        assert!(matches!(err, SecurityError::Invalid(_)));
    }

    #[test]
    fn proc_mount_unmasked_requires_privileged() {
        let mut cont = cont_default();
        cont.proc_mount = Some(ProcMountType::Unmasked);
        let err = materialize(&pod_default(), &cont).unwrap_err();
        assert!(matches!(err, SecurityError::Forbidden(_)));
    }

    #[test]
    fn proc_mount_unmasked_ok_with_privileged() {
        let mut cont = cont_default();
        cont.proc_mount = Some(ProcMountType::Unmasked);
        cont.privileged = Some(true);
        materialize(&pod_default(), &cont).unwrap();
    }

    #[test]
    fn seccomp_localhost_requires_path() {
        let mut cont = cont_default();
        cont.seccomp_profile = Some(SeccompProfileType::Localhost("".into()));
        let err = materialize(&pod_default(), &cont).unwrap_err();
        assert!(matches!(err, SecurityError::Invalid(_)));
    }

    #[test]
    fn seccomp_localhost_rejects_absolute_path() {
        let p = SeccompProfileType::Localhost("/etc/profile.json".into());
        assert!(validate_seccomp(&p).is_err());
    }

    #[test]
    fn seccomp_localhost_rejects_dotdot() {
        let p = SeccompProfileType::Localhost("../escape.json".into());
        assert!(validate_seccomp(&p).is_err());
    }

    #[test]
    fn seccomp_localhost_accepts_relative() {
        let p = SeccompProfileType::Localhost("profiles/strict.json".into());
        validate_seccomp(&p).unwrap();
    }

    #[test]
    fn seccomp_runtime_default_path_string() {
        let s = materialize_seccomp_path(&SeccompProfileType::RuntimeDefault, "/var/lib/kubelet/seccomp").unwrap();
        assert_eq!(s, "runtime/default");
    }

    #[test]
    fn seccomp_unconfined_path_string() {
        let s = materialize_seccomp_path(&SeccompProfileType::Unconfined, "/var/lib/kubelet/seccomp").unwrap();
        assert_eq!(s, "unconfined");
    }

    #[test]
    fn seccomp_localhost_path_string_appends_dir() {
        let s = materialize_seccomp_path(
            &SeccompProfileType::Localhost("strict.json".into()),
            "/var/lib/kubelet/seccomp",
        )
        .unwrap();
        assert_eq!(s, "localhost//var/lib/kubelet/seccomp/strict.json");
    }

    #[test]
    fn seccomp_localhost_path_string_strips_trailing_slash() {
        let s = materialize_seccomp_path(
            &SeccompProfileType::Localhost("a.json".into()),
            "/var/lib/kubelet/seccomp/",
        )
        .unwrap();
        assert!(s.ends_with("/var/lib/kubelet/seccomp/a.json"));
    }

    #[test]
    fn capability_known_known_unknown() {
        assert!(is_known_capability("NET_ADMIN"));
        assert!(is_known_capability("CAP_NET_ADMIN"));
        assert!(is_known_capability("SYS_ADMIN"));
        assert!(!is_known_capability("FAKE_CAP"));
    }

    #[test]
    fn capability_validation_rejects_unknown_in_add() {
        let c = Capabilities { add: vec!["FAKE".into()], drop: vec![] };
        assert!(validate_capabilities(&c).is_err());
    }

    #[test]
    fn capability_validation_rejects_unknown_in_drop() {
        let c = Capabilities { add: vec![], drop: vec!["FAKE".into()] };
        assert!(validate_capabilities(&c).is_err());
    }

    #[test]
    fn capability_validation_rejects_overlap_add_drop() {
        let c = Capabilities {
            add: vec!["NET_ADMIN".into()],
            drop: vec!["CAP_NET_ADMIN".into()],
        };
        assert!(validate_capabilities(&c).is_err());
    }

    #[test]
    fn capability_validation_accepts_disjoint_lists() {
        let c = Capabilities {
            add: vec!["NET_ADMIN".into()],
            drop: vec!["SYS_TIME".into()],
        };
        validate_capabilities(&c).unwrap();
    }

    #[test]
    fn capability_all_recognised() {
        assert!(is_known_capability("ALL"));
    }

    #[test]
    fn pod_seccomp_inherits_when_container_unset() {
        let mut pod = pod_default();
        pod.seccomp_profile = Some(SeccompProfileType::Unconfined);
        let e = materialize(&pod, &cont_default()).unwrap();
        assert_eq!(e.seccomp_profile, SeccompProfileType::Unconfined);
    }

    #[test]
    fn container_seccomp_overrides_pod() {
        let mut pod = pod_default();
        pod.seccomp_profile = Some(SeccompProfileType::Unconfined);
        let mut cont = cont_default();
        cont.seccomp_profile = Some(SeccompProfileType::RuntimeDefault);
        let e = materialize(&pod, &cont).unwrap();
        assert_eq!(e.seccomp_profile, SeccompProfileType::RuntimeDefault);
    }

    #[test]
    fn se_linux_inherits_from_pod() {
        let mut pod = pod_default();
        pod.se_linux_options = Some(SeLinuxOptions {
            level: Some("s0:c1,c2".into()),
            ..Default::default()
        });
        let e = materialize(&pod, &cont_default()).unwrap();
        assert_eq!(e.se_linux_options.level, Some("s0:c1,c2".into()));
    }

    #[test]
    fn fs_group_only_at_pod_level() {
        let mut pod = pod_default();
        pod.fs_group = Some(2000);
        let e = materialize(&pod, &cont_default()).unwrap();
        assert_eq!(e.fs_group, Some(2000));
    }

    #[test]
    fn fs_group_change_policy_inherited() {
        let mut pod = pod_default();
        pod.fs_group_change_policy = Some(FsGroupChangePolicy::OnRootMismatch);
        let e = materialize(&pod, &cont_default()).unwrap();
        assert_eq!(e.fs_group_change_policy, FsGroupChangePolicy::OnRootMismatch);
    }

    #[test]
    fn supplemental_groups_from_pod() {
        let mut pod = pod_default();
        pod.supplemental_groups = vec![1000, 2000, 3000];
        let e = materialize(&pod, &cont_default()).unwrap();
        assert_eq!(e.supplemental_groups, vec![1000, 2000, 3000]);
    }

    #[test]
    fn sysctls_from_pod() {
        let mut pod = pod_default();
        pod.sysctls = vec![("net.core.somaxconn".into(), "1024".into())];
        let e = materialize(&pod, &cont_default()).unwrap();
        assert_eq!(e.sysctls.len(), 1);
    }

    #[test]
    fn windows_options_at_container_overrides_pod() {
        let mut pod = pod_default();
        pod.windows_options = Some(WindowsSecurityContextOptions {
            run_as_user_name: Some("LocalSystem".into()),
            ..Default::default()
        });
        let mut cont = cont_default();
        cont.windows_options = Some(WindowsSecurityContextOptions {
            run_as_user_name: Some("ContainerUser".into()),
            ..Default::default()
        });
        let e = materialize(&pod, &cont).unwrap();
        assert_eq!(
            e.windows.unwrap().run_as_user_name,
            Some("ContainerUser".into())
        );
    }

    #[test]
    fn windows_options_with_run_as_user_rejected() {
        let mut cont = cont_default();
        cont.run_as_user = Some(1000);
        cont.windows_options = Some(WindowsSecurityContextOptions {
            host_process: Some(true),
            ..Default::default()
        });
        let err = materialize(&pod_default(), &cont).unwrap_err();
        assert!(matches!(err, SecurityError::Invalid(_)));
    }

    #[test]
    fn effective_groups_merges_run_as_group_and_supplemental_dedup_sort() {
        let mut pod = pod_default();
        pod.supplemental_groups = vec![3000, 1000, 2000];
        let mut cont = cont_default();
        cont.run_as_group = Some(2000);
        let e = materialize(&pod, &cont).unwrap();
        let g = effective_groups(&e);
        assert_eq!(g, vec![1000, 2000, 3000]);
    }

    #[test]
    fn effective_groups_when_no_run_as_group() {
        let mut pod = pod_default();
        pod.supplemental_groups = vec![5000];
        let e = materialize(&pod, &cont_default()).unwrap();
        let g = effective_groups(&e);
        assert_eq!(g, vec![5000]);
    }

    #[test]
    fn effective_groups_empty_when_unset() {
        let e = materialize(&pod_default(), &cont_default()).unwrap();
        assert!(effective_groups(&e).is_empty());
    }

    #[test]
    fn allow_priv_escalation_default_true() {
        let e = materialize(&pod_default(), &cont_default()).unwrap();
        assert!(e.allow_privilege_escalation);
    }

    #[test]
    fn allow_priv_escalation_explicit_false() {
        let mut cont = cont_default();
        cont.allow_privilege_escalation = Some(false);
        let e = materialize(&pod_default(), &cont).unwrap();
        assert!(!e.allow_privilege_escalation);
    }

    #[test]
    fn read_only_root_fs_default_false() {
        let e = materialize(&pod_default(), &cont_default()).unwrap();
        assert!(!e.read_only_root_filesystem);
    }

    #[test]
    fn read_only_root_fs_explicit_true() {
        let mut cont = cont_default();
        cont.read_only_root_filesystem = Some(true);
        let e = materialize(&pod_default(), &cont).unwrap();
        assert!(e.read_only_root_filesystem);
    }

    #[test]
    fn capabilities_default_empty() {
        let e = materialize(&pod_default(), &cont_default()).unwrap();
        assert!(e.capabilities.add.is_empty());
        assert!(e.capabilities.drop.is_empty());
    }

    #[test]
    fn capabilities_explicit_add_drop_round_trip() {
        let mut cont = cont_default();
        cont.capabilities = Some(Capabilities {
            add: vec!["NET_ADMIN".into()],
            drop: vec!["SYS_TIME".into()],
        });
        let e = materialize(&pod_default(), &cont).unwrap();
        assert_eq!(e.capabilities.add, vec!["NET_ADMIN".to_string()]);
        assert_eq!(e.capabilities.drop, vec!["SYS_TIME".to_string()]);
    }

    #[test]
    fn validate_runs_after_materialize() {
        // Direct validate should still catch the same.
        let ctx = EffectiveSecurityContext {
            run_as_user: Some(0),
            run_as_group: None,
            run_as_non_root: true,
            privileged: false,
            allow_privilege_escalation: true,
            read_only_root_filesystem: false,
            se_linux_options: SeLinuxOptions::default(),
            seccomp_profile: SeccompProfileType::RuntimeDefault,
            capabilities: Capabilities::default(),
            proc_mount: ProcMountType::Default,
            fs_group: None,
            fs_group_change_policy: FsGroupChangePolicy::Always,
            supplemental_groups: vec![],
            sysctls: vec![],
            windows: None,
        };
        assert!(validate(&ctx).is_err());
    }

    #[test]
    fn normalize_cap_strips_prefix() {
        assert_eq!(normalize_cap("CAP_NET_ADMIN"), "NET_ADMIN");
        assert_eq!(normalize_cap("NET_ADMIN"), "NET_ADMIN");
    }

    #[test]
    fn pod_se_linux_overridden_by_container() {
        let mut pod = pod_default();
        pod.se_linux_options = Some(SeLinuxOptions {
            level: Some("s0".into()),
            ..Default::default()
        });
        let mut cont = cont_default();
        cont.se_linux_options = Some(SeLinuxOptions {
            level: Some("s0:c5,c10".into()),
            ..Default::default()
        });
        let e = materialize(&pod, &cont).unwrap();
        assert_eq!(e.se_linux_options.level, Some("s0:c5,c10".into()));
    }

    #[test]
    fn run_as_non_root_with_non_zero_user_ok() {
        let mut cont = cont_default();
        cont.run_as_non_root = Some(true);
        cont.run_as_user = Some(1000);
        materialize(&pod_default(), &cont).unwrap();
    }

    #[test]
    fn capabilities_default_when_container_none() {
        let e = materialize(&pod_default(), &cont_default()).unwrap();
        assert_eq!(e.capabilities, Capabilities::default());
    }
}
