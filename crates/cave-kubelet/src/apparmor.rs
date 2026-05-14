// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! AppArmor profile validation, loading, and application.
//!
//! Mirrors `pkg/security/apparmor/validate.go` + admission helpers and the
//! 1.30 GA `securityContext.appArmorProfile` field, including the legacy
//! `container.apparmor.security.beta.kubernetes.io/<container>` annotation.
//!
//! All host interactions (`/sys/kernel/security/apparmor/profiles`) are
//! abstracted behind a `LoadedProfiles` set so the validation/conversion
//! state machine is testable in isolation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const ANNOTATION_KEY_PREFIX: &str = "container.apparmor.security.beta.kubernetes.io/";

/// AppArmor profile spec. Mirrors `core/v1.AppArmorProfile`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppArmorProfile {
    Unconfined,
    RuntimeDefault,
    Localhost(String),
}

impl AppArmorProfile {
    pub fn type_str(&self) -> &'static str {
        match self {
            Self::Unconfined => "Unconfined",
            Self::RuntimeDefault => "RuntimeDefault",
            Self::Localhost(_) => "Localhost",
        }
    }
}

/// Parse an annotation value into a profile.
/// Accepted forms (per upstream `pkg/security/apparmor/helpers.go`):
///   - `runtime/default`
///   - `unconfined`
///   - `localhost/<name>`
pub fn parse_annotation(value: &str) -> Result<AppArmorProfile, AppArmorError> {
    if value == "runtime/default" {
        Ok(AppArmorProfile::RuntimeDefault)
    } else if value == "unconfined" {
        Ok(AppArmorProfile::Unconfined)
    } else if let Some(rest) = value.strip_prefix("localhost/") {
        if rest.is_empty() {
            return Err(AppArmorError::Invalid(
                "localhost profile name empty".into(),
            ));
        }
        validate_profile_name(rest)?;
        Ok(AppArmorProfile::Localhost(rest.to_string()))
    } else {
        Err(AppArmorError::Invalid(format!(
            "unrecognised AppArmor profile annotation: {}",
            value
        )))
    }
}

/// Render profile as an annotation value.
pub fn render_annotation(p: &AppArmorProfile) -> String {
    match p {
        AppArmorProfile::RuntimeDefault => "runtime/default".to_string(),
        AppArmorProfile::Unconfined => "unconfined".to_string(),
        AppArmorProfile::Localhost(name) => format!("localhost/{}", name),
    }
}

/// Profile name validation: AppArmor accepts `[A-Za-z0-9._-]+` and the
/// kubelet additionally rejects names starting with `.` or containing `..`.
pub fn validate_profile_name(name: &str) -> Result<(), AppArmorError> {
    if name.is_empty() {
        return Err(AppArmorError::Invalid("profile name empty".into()));
    }
    if name.starts_with('.') {
        return Err(AppArmorError::Invalid(
            "profile name must not start with '.'".into(),
        ));
    }
    if name.contains("..") {
        return Err(AppArmorError::Invalid(
            "profile name must not contain '..'".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
    {
        return Err(AppArmorError::Invalid(format!(
            "profile name '{}' contains invalid characters",
            name
        )));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AppArmorError {
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not loaded: {0}")]
    NotLoaded(String),
    #[error("kernel apparmor disabled")]
    KernelDisabled,
    #[error("conflict: {0}")]
    Conflict(String),
}

/// Loaded profiles registry — represents `/sys/kernel/security/apparmor/profiles`.
#[derive(Debug, Default, Clone)]
pub struct LoadedProfiles {
    pub kernel_enabled: bool,
    profiles: BTreeSet<String>,
}

impl LoadedProfiles {
    pub fn enabled() -> Self {
        Self {
            kernel_enabled: true,
            profiles: BTreeSet::new(),
        }
    }

    pub fn disabled() -> Self {
        Self {
            kernel_enabled: false,
            profiles: BTreeSet::new(),
        }
    }

    pub fn with(mut self, name: &str) -> Self {
        self.profiles.insert(name.to_string());
        self
    }

    pub fn load(&mut self, name: &str) {
        self.profiles.insert(name.to_string());
    }

    pub fn unload(&mut self, name: &str) {
        self.profiles.remove(name);
    }

    pub fn is_loaded(&self, name: &str) -> bool {
        self.profiles.contains(name)
    }

    pub fn count(&self) -> usize {
        self.profiles.len()
    }
}

/// Validate that `profile` can be applied on this host given currently loaded
/// kernel profiles. Mirrors upstream `Validator.Validate()`.
pub fn validate_against_loaded(
    profile: &AppArmorProfile,
    loaded: &LoadedProfiles,
) -> Result<(), AppArmorError> {
    if !loaded.kernel_enabled {
        // Unconfined is always OK because it's a no-op.
        if matches!(profile, AppArmorProfile::Unconfined) {
            return Ok(());
        }
        return Err(AppArmorError::KernelDisabled);
    }
    match profile {
        AppArmorProfile::Unconfined | AppArmorProfile::RuntimeDefault => Ok(()),
        AppArmorProfile::Localhost(name) => {
            validate_profile_name(name)?;
            if loaded.is_loaded(name) {
                Ok(())
            } else {
                Err(AppArmorError::NotLoaded(name.clone()))
            }
        }
    }
}

/// Convert to the runtime CRI form used in `LinuxContainerSecurityContext.apparmor`.
/// `unconfined`, `runtime/default`, or `localhost/<name>`.
pub fn to_cri_profile_string(profile: &AppArmorProfile) -> String {
    render_annotation(profile)
}

/// When both annotation and spec field are set, they must agree. (Upstream
/// admission emits a warning + reconciles to the spec field; we treat
/// disagreement as a conflict so callers can decide.)
pub fn reconcile_annotation_and_spec(
    annotation: Option<&AppArmorProfile>,
    spec: Option<&AppArmorProfile>,
) -> Result<Option<AppArmorProfile>, AppArmorError> {
    match (annotation, spec) {
        (None, None) => Ok(None),
        (Some(a), None) => Ok(Some(a.clone())),
        (None, Some(s)) => Ok(Some(s.clone())),
        (Some(a), Some(s)) if a == s => Ok(Some(s.clone())),
        (Some(a), Some(s)) => Err(AppArmorError::Conflict(format!(
            "annotation says {} but spec says {}",
            render_annotation(a),
            render_annotation(s)
        ))),
    }
}

/// Extract a per-container profile from the pod's annotations.
pub fn profile_from_annotations<'a, I>(
    annotations: I,
    container_name: &str,
) -> Result<Option<AppArmorProfile>, AppArmorError>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let key = format!("{}{}", ANNOTATION_KEY_PREFIX, container_name);
    for (k, v) in annotations {
        if k == key {
            return parse_annotation(v).map(Some);
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_annotation_runtime_default() {
        assert_eq!(parse_annotation("runtime/default").unwrap(), AppArmorProfile::RuntimeDefault);
    }

    #[test]
    fn parse_annotation_unconfined() {
        assert_eq!(parse_annotation("unconfined").unwrap(), AppArmorProfile::Unconfined);
    }

    #[test]
    fn parse_annotation_localhost_with_name() {
        assert_eq!(
            parse_annotation("localhost/my-strict").unwrap(),
            AppArmorProfile::Localhost("my-strict".into())
        );
    }

    #[test]
    fn parse_annotation_localhost_empty_name_errors() {
        assert!(parse_annotation("localhost/").is_err());
    }

    #[test]
    fn parse_annotation_unknown_form_errors() {
        assert!(parse_annotation("unknown").is_err());
        assert!(parse_annotation("/etc/profile").is_err());
        assert!(parse_annotation("").is_err());
    }

    #[test]
    fn render_annotation_round_trip() {
        let cases = [
            AppArmorProfile::RuntimeDefault,
            AppArmorProfile::Unconfined,
            AppArmorProfile::Localhost("nginx-strict".into()),
        ];
        for c in cases {
            let s = render_annotation(&c);
            assert_eq!(parse_annotation(&s).unwrap(), c);
        }
    }

    #[test]
    fn type_str_matches_kind() {
        assert_eq!(AppArmorProfile::Unconfined.type_str(), "Unconfined");
        assert_eq!(AppArmorProfile::RuntimeDefault.type_str(), "RuntimeDefault");
        assert_eq!(AppArmorProfile::Localhost("x".into()).type_str(), "Localhost");
    }

    #[test]
    fn validate_profile_name_accepts_alnum() {
        validate_profile_name("nginx-strict-2.0").unwrap();
        validate_profile_name("docker_default").unwrap();
        validate_profile_name("my.profile-1").unwrap();
    }

    #[test]
    fn validate_profile_name_rejects_empty() {
        assert!(validate_profile_name("").is_err());
    }

    #[test]
    fn validate_profile_name_rejects_dot_prefix() {
        assert!(validate_profile_name(".hidden").is_err());
    }

    #[test]
    fn validate_profile_name_rejects_dotdot() {
        assert!(validate_profile_name("escape..bad").is_err());
        assert!(validate_profile_name("..").is_err());
    }

    #[test]
    fn validate_profile_name_rejects_invalid_chars() {
        assert!(validate_profile_name("with space").is_err());
        assert!(validate_profile_name("with#hash").is_err());
        assert!(validate_profile_name("with$dollar").is_err());
    }

    #[test]
    fn loaded_profiles_disabled_rejects_runtime_default() {
        let l = LoadedProfiles::disabled();
        assert!(matches!(
            validate_against_loaded(&AppArmorProfile::RuntimeDefault, &l),
            Err(AppArmorError::KernelDisabled)
        ));
    }

    #[test]
    fn loaded_profiles_disabled_rejects_localhost() {
        let l = LoadedProfiles::disabled();
        assert!(matches!(
            validate_against_loaded(&AppArmorProfile::Localhost("x".into()), &l),
            Err(AppArmorError::KernelDisabled)
        ));
    }

    #[test]
    fn loaded_profiles_disabled_allows_unconfined() {
        let l = LoadedProfiles::disabled();
        validate_against_loaded(&AppArmorProfile::Unconfined, &l).unwrap();
    }

    #[test]
    fn loaded_profiles_enabled_runtime_default_ok() {
        let l = LoadedProfiles::enabled();
        validate_against_loaded(&AppArmorProfile::RuntimeDefault, &l).unwrap();
    }

    #[test]
    fn loaded_profiles_enabled_unconfined_ok() {
        let l = LoadedProfiles::enabled();
        validate_against_loaded(&AppArmorProfile::Unconfined, &l).unwrap();
    }

    #[test]
    fn loaded_profiles_localhost_requires_loaded() {
        let l = LoadedProfiles::enabled();
        let err =
            validate_against_loaded(&AppArmorProfile::Localhost("strict".into()), &l).unwrap_err();
        assert!(matches!(err, AppArmorError::NotLoaded(_)));
    }

    #[test]
    fn loaded_profiles_localhost_ok_when_loaded() {
        let l = LoadedProfiles::enabled().with("strict");
        validate_against_loaded(&AppArmorProfile::Localhost("strict".into()), &l).unwrap();
    }

    #[test]
    fn loaded_profiles_load_then_unload_round_trip() {
        let mut l = LoadedProfiles::enabled();
        l.load("p1");
        assert!(l.is_loaded("p1"));
        l.unload("p1");
        assert!(!l.is_loaded("p1"));
    }

    #[test]
    fn loaded_profiles_count_tracks_unique_set() {
        let l = LoadedProfiles::enabled().with("a").with("b").with("a");
        assert_eq!(l.count(), 2);
    }

    #[test]
    fn cri_profile_string_round_trip() {
        for p in [
            AppArmorProfile::Unconfined,
            AppArmorProfile::RuntimeDefault,
            AppArmorProfile::Localhost("foo".into()),
        ] {
            assert_eq!(parse_annotation(&to_cri_profile_string(&p)).unwrap(), p);
        }
    }

    #[test]
    fn reconcile_both_none() {
        assert_eq!(reconcile_annotation_and_spec(None, None).unwrap(), None);
    }

    #[test]
    fn reconcile_only_annotation() {
        let a = AppArmorProfile::RuntimeDefault;
        assert_eq!(reconcile_annotation_and_spec(Some(&a), None).unwrap(), Some(a));
    }

    #[test]
    fn reconcile_only_spec() {
        let s = AppArmorProfile::Unconfined;
        assert_eq!(reconcile_annotation_and_spec(None, Some(&s)).unwrap(), Some(s));
    }

    #[test]
    fn reconcile_agreeing_pair() {
        let a = AppArmorProfile::Localhost("strict".into());
        let s = AppArmorProfile::Localhost("strict".into());
        assert_eq!(reconcile_annotation_and_spec(Some(&a), Some(&s)).unwrap(), Some(a));
    }

    #[test]
    fn reconcile_disagreeing_pair_errors() {
        let a = AppArmorProfile::RuntimeDefault;
        let s = AppArmorProfile::Unconfined;
        assert!(matches!(
            reconcile_annotation_and_spec(Some(&a), Some(&s)),
            Err(AppArmorError::Conflict(_))
        ));
    }

    #[test]
    fn profile_from_annotations_resolves_per_container() {
        let ann = vec![
            (
                "container.apparmor.security.beta.kubernetes.io/web",
                "runtime/default",
            ),
            (
                "container.apparmor.security.beta.kubernetes.io/sidecar",
                "unconfined",
            ),
        ];
        assert_eq!(
            profile_from_annotations(ann.iter().copied(), "web").unwrap(),
            Some(AppArmorProfile::RuntimeDefault)
        );
        assert_eq!(
            profile_from_annotations(ann.iter().copied(), "sidecar").unwrap(),
            Some(AppArmorProfile::Unconfined)
        );
    }

    #[test]
    fn profile_from_annotations_returns_none_when_missing() {
        let ann: Vec<(&str, &str)> = vec![];
        assert!(profile_from_annotations(ann, "web").unwrap().is_none());
    }

    #[test]
    fn profile_from_annotations_propagates_parse_error() {
        let ann = vec![(
            "container.apparmor.security.beta.kubernetes.io/web",
            "garbage",
        )];
        assert!(profile_from_annotations(ann.iter().copied(), "web").is_err());
    }

    #[test]
    fn unconfined_allowed_with_kernel_disabled_special_case() {
        // The kernel-disabled escape hatch: only Unconfined is allowed.
        let l = LoadedProfiles::disabled();
        validate_against_loaded(&AppArmorProfile::Unconfined, &l).unwrap();
        assert!(validate_against_loaded(&AppArmorProfile::RuntimeDefault, &l).is_err());
    }

    #[test]
    fn localhost_with_invalid_chars_rejected_at_parse_and_validate() {
        assert!(parse_annotation("localhost/with space").is_err());
        let l = LoadedProfiles::enabled();
        assert!(validate_against_loaded(
            &AppArmorProfile::Localhost("with space".into()),
            &l
        )
        .is_err());
    }

    #[test]
    fn loaded_profiles_with_builder_chains() {
        let l = LoadedProfiles::enabled().with("a").with("b").with("c");
        assert!(l.is_loaded("a") && l.is_loaded("b") && l.is_loaded("c"));
    }

    #[test]
    fn parse_localhost_with_subdir_style_name() {
        let p = parse_annotation("localhost/team-x/strict-1").unwrap();
        match p {
            AppArmorProfile::Localhost(n) => assert_eq!(n, "team-x/strict-1"),
            _ => panic!(),
        }
    }
}
