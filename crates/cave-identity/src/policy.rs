// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). Admission policy line-ported
// from pkg/server/api/entry/v1/service.go::CreateEntries (admin/downstream
// validation + TTL clamp).
//
//! Admission policy for registration entries.

use crate::error::{IdentityError, Result};
use crate::models::{RegistrationEntry, SpiffeId, TrustDomain};
use crate::spiffe_id::parse_spiffe_id;

/// Tunable policy parameters — equivalent to spire-server config TTL block.
#[derive(Debug, Clone)]
pub struct PolicyConfig {
    pub min_x509_svid_ttl_seconds: u32,
    pub max_x509_svid_ttl_seconds: u32,
    pub min_jwt_svid_ttl_seconds: u32,
    pub max_jwt_svid_ttl_seconds: u32,
    /// Caller authority needed before `downstream = true` is honoured.
    pub require_admin_for_downstream: bool,
    /// Trust domain entries must live within. Other TDs are rejected.
    pub trust_domain: TrustDomain,
}

impl PolicyConfig {
    pub fn new(trust_domain: TrustDomain) -> Self {
        Self {
            min_x509_svid_ttl_seconds: 60,
            max_x509_svid_ttl_seconds: 7 * 24 * 3600,
            min_jwt_svid_ttl_seconds: 60,
            max_jwt_svid_ttl_seconds: 24 * 3600,
            require_admin_for_downstream: true,
            trust_domain,
        }
    }
}

/// Caller authority — `[[mapped]]` corresponds to spire-server admin auth.
#[derive(Debug, Clone)]
pub struct Caller {
    pub spiffe_id: SpiffeId,
    pub admin: bool,
}

/// Admit a new entry, returning the entry with any TTLs clamped to policy
/// bounds.
pub fn admit_entry(
    cfg: &PolicyConfig,
    caller: &Caller,
    mut entry: RegistrationEntry,
) -> Result<RegistrationEntry> {
    let td = cfg.trust_domain.as_str();
    let parsed_id = parse_spiffe_id(entry.spiffe_id.as_str())?;
    if parsed_id.trust_domain.as_str() != td {
        return Err(IdentityError::PolicyViolation(format!(
            "spiffe_id trust-domain mismatch (want {}; got {})",
            td,
            parsed_id.trust_domain.as_str()
        )));
    }
    let parsed_parent = parse_spiffe_id(entry.parent_id.as_str())?;
    if parsed_parent.trust_domain.as_str() != td {
        return Err(IdentityError::PolicyViolation(format!(
            "parent_id trust-domain mismatch (want {}; got {})",
            td,
            parsed_parent.trust_domain.as_str()
        )));
    }
    clamp_ttl(
        &mut entry.x509_svid_ttl_seconds,
        cfg.min_x509_svid_ttl_seconds,
        cfg.max_x509_svid_ttl_seconds,
    )?;
    clamp_ttl(
        &mut entry.jwt_svid_ttl_seconds,
        cfg.min_jwt_svid_ttl_seconds,
        cfg.max_jwt_svid_ttl_seconds,
    )?;
    if entry.downstream && cfg.require_admin_for_downstream && !caller.admin {
        return Err(IdentityError::PolicyViolation(
            "downstream requires admin caller".into(),
        ));
    }
    if entry.admin && !caller.admin {
        return Err(IdentityError::PolicyViolation(
            "creating admin entries requires admin caller".into(),
        ));
    }
    if entry.selectors.is_empty() {
        return Err(IdentityError::PolicyViolation(
            "selectors required".into(),
        ));
    }
    Ok(entry)
}

fn clamp_ttl(value: &mut u32, min: u32, max: u32) -> Result<()> {
    if *value == 0 {
        *value = min.max(60);
        return Ok(());
    }
    if *value < min {
        return Err(IdentityError::TtlOutOfBounds {
            requested: *value,
            min,
            max,
        });
    }
    if *value > max {
        return Err(IdentityError::TtlOutOfBounds {
            requested: *value,
            min,
            max,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Selector;

    fn entry() -> RegistrationEntry {
        RegistrationEntry {
            id: "e".into(),
            spiffe_id: SpiffeId::new("spiffe://example.org/svc"),
            parent_id: SpiffeId::new("spiffe://example.org/spire/agent/k8s/n"),
            selectors: vec![Selector::new("k8s", "ns:default")],
            x509_svid_ttl_seconds: 3600,
            jwt_svid_ttl_seconds: 300,
            ..Default::default()
        }
    }

    fn admin() -> Caller {
        Caller {
            spiffe_id: SpiffeId::new("spiffe://example.org/admin"),
            admin: true,
        }
    }

    fn cfg() -> PolicyConfig {
        PolicyConfig::new(TrustDomain::new("example.org"))
    }

    #[test]
    fn admits_basic_entry() {
        let r = admit_entry(&cfg(), &admin(), entry()).unwrap();
        assert_eq!(r.x509_svid_ttl_seconds, 3600);
    }

    #[test]
    fn rejects_foreign_trust_domain() {
        let mut e = entry();
        e.spiffe_id = SpiffeId::new("spiffe://other.org/svc");
        assert!(matches!(
            admit_entry(&cfg(), &admin(), e),
            Err(IdentityError::PolicyViolation(_))
        ));
    }

    #[test]
    fn rejects_downstream_without_admin() {
        let mut e = entry();
        e.downstream = true;
        let nonadmin = Caller {
            spiffe_id: SpiffeId::new("spiffe://example.org/user"),
            admin: false,
        };
        assert!(matches!(
            admit_entry(&cfg(), &nonadmin, e),
            Err(IdentityError::PolicyViolation(_))
        ));
    }

    #[test]
    fn rejects_admin_creation_by_nonadmin() {
        let mut e = entry();
        e.admin = true;
        let nonadmin = Caller {
            spiffe_id: SpiffeId::new("spiffe://example.org/user"),
            admin: false,
        };
        assert!(matches!(
            admit_entry(&cfg(), &nonadmin, e),
            Err(IdentityError::PolicyViolation(_))
        ));
    }

    #[test]
    fn rejects_ttl_too_small() {
        let mut e = entry();
        e.x509_svid_ttl_seconds = 1;
        assert!(matches!(
            admit_entry(&cfg(), &admin(), e),
            Err(IdentityError::TtlOutOfBounds { .. })
        ));
    }

    #[test]
    fn rejects_ttl_too_big() {
        let mut e = entry();
        e.x509_svid_ttl_seconds = 365 * 24 * 3600;
        assert!(matches!(
            admit_entry(&cfg(), &admin(), e),
            Err(IdentityError::TtlOutOfBounds { .. })
        ));
    }

    #[test]
    fn zero_ttl_is_filled_with_floor() {
        let mut e = entry();
        e.x509_svid_ttl_seconds = 0;
        let r = admit_entry(&cfg(), &admin(), e).unwrap();
        assert!(r.x509_svid_ttl_seconds >= 60);
    }

    #[test]
    fn rejects_empty_selectors() {
        let mut e = entry();
        e.selectors.clear();
        assert!(matches!(
            admit_entry(&cfg(), &admin(), e),
            Err(IdentityError::PolicyViolation(_))
        ));
    }
}
