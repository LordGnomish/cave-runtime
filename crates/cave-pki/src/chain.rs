//! Chain validation. Cite: RFC 5280 §6.1 (Certification Path Validation),
//! §6.1.3 (basic certificate processing), §4.2.1.9 (Basic Constraints).

use crate::ca::{Ca, CaKind, CertHandle};
use crate::crl::CrlResponder;
use crate::error::{PkiError, PkiResult};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationResult {
    /// Chain valid; trust anchor is the named root serial.
    Valid { trust_anchor: String, depth: usize },
    /// Chain invalid for the listed reason.
    Invalid(String),
}

#[derive(Debug)]
pub struct ChainValidator<'a> {
    pub ca: &'a Ca,
    pub crl: Option<&'a CrlResponder>,
    pub now: DateTime<Utc>,
}

impl<'a> ChainValidator<'a> {
    pub fn new(ca: &'a Ca) -> Self {
        Self { ca, crl: None, now: Utc::now() }
    }

    pub fn with_crl(mut self, crl: &'a CrlResponder) -> Self {
        self.crl = Some(crl);
        self
    }

    pub fn at(mut self, now: DateTime<Utc>) -> Self {
        self.now = now;
        self
    }

    /// Cite: RFC 5280 §6.1 — walk the chain leaf → root, checking:
    ///  1. validity dates (`not_before <= now <= not_after`);
    ///  2. issuer reference matches the parent's serial;
    ///  3. parent's `kind` is allowed to issue the child (Root → Plat,
    ///     Plat → Tenant, Tenant → Leaf-or-Tenant);
    ///  4. trust anchor is a Root the validator knows;
    ///  5. revocation status (CRL) of every non-root element.
    pub fn validate(&self, leaf_serial: &str) -> PkiResult<ValidationResult> {
        let chain = self.ca.chain_for(leaf_serial)?;
        if chain.is_empty() {
            return Ok(ValidationResult::Invalid("empty chain".into()));
        }

        for cert in &chain {
            if cert.is_expired(self.now) {
                return Ok(ValidationResult::Invalid(format!(
                    "{} expired at {}", cert.serial, cert.not_after
                )));
            }
            if cert.not_before > self.now {
                return Ok(ValidationResult::Invalid(format!(
                    "{} not yet valid (starts {})", cert.serial, cert.not_before
                )));
            }
            // Revocation only applies to non-root elements
            if !matches!(cert.kind, CaKind::Root) {
                if let Some(crl) = self.crl {
                    if let Some(entry) = crl.lookup(&cert.serial) {
                        return Err(PkiError::Revoked {
                            serial: entry.serial.clone(),
                            revoked_at: entry.revoked_at.to_rfc3339(),
                            reason: entry.reason,
                        });
                    }
                }
            }
        }

        // Walk parent links.
        for window in chain.windows(2) {
            let child = &window[0];
            let parent = &window[1];
            let expected = child.issuer_serial.as_deref().unwrap_or("");
            if expected != parent.serial {
                return Ok(ValidationResult::Invalid(format!(
                    "{} issuer reference {} does not match parent {}",
                    child.serial, expected, parent.serial,
                )));
            }
            if !legal_issuer(parent.kind, child.kind) {
                return Ok(ValidationResult::Invalid(format!(
                    "{:?} cannot issue {:?}", parent.kind, child.kind,
                )));
            }
        }

        // Trust anchor: chain must end with a known Root.
        let anchor = chain.last().unwrap();
        if anchor.kind != CaKind::Root {
            return Ok(ValidationResult::Invalid(format!(
                "trust anchor {} is not a Root CA", anchor.serial,
            )));
        }

        Ok(ValidationResult::Valid {
            trust_anchor: anchor.serial.clone(),
            depth: chain.len(),
        })
    }
}

fn legal_issuer(parent: CaKind, child: CaKind) -> bool {
    matches!(
        (parent, child),
        (CaKind::Root, CaKind::PlatformIntermediate)
        | (CaKind::PlatformIntermediate, CaKind::TenantIntermediate)
        // tenant intermediates may issue leaves (modelled as `TenantIntermediate`
        // for now; cave will add a Leaf kind in a follow-up batch).
        | (CaKind::TenantIntermediate, CaKind::TenantIntermediate)
    )
}

#[allow(dead_code)]
fn _silence_unused(c: &CertHandle) -> &str { &c.serial }
