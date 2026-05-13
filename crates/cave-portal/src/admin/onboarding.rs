//! Persona-tailored onboarding tour.
//!
//! On first login a new persona sees a 4-5 step wizard with the
//! "where do I start" routes for their role. Progress is held in a
//! per-principal map keyed by `(principal, persona)` so two personas
//! sharing an account each get their own progress.

use crate::admin::permission::{Permission, Persona, RequestCtx};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OnboardError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("step {0} does not belong to this persona's tour")]
    UnknownStep(String),
    #[error("step {0} already complete")]
    AlreadyComplete(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TourStep {
    pub id: String,
    pub title: String,
    pub href: String,
    pub description: String,
}

/// One persona's tour script. Read-only — the same for every principal.
pub fn tour_for(persona: Persona) -> Vec<TourStep> {
    match persona {
        Persona::PlatformAdmin => vec![
            TourStep {
                id: "compliance".into(),
                title: "Charter compliance".into(),
                href: "/admin/compliance".into(),
                description:
                    "Per-crate dual-grade matrix. Start here to see what's at Grade A vs F."
                        .into(),
            },
            TourStep {
                id: "upstream".into(),
                title: "Upstream projects".into(),
                href: "/admin/upstream".into(),
                description: "Pinned upstream versions and last-check timestamps.".into(),
            },
            TourStep {
                id: "adr".into(),
                title: "ADR Browser".into(),
                href: "/admin/adr".into(),
                description: "Cave Charter + every accepted Architecture Decision Record.".into(),
            },
            TourStep {
                id: "audit".into(),
                title: "Audit log".into(),
                href: "/admin/audit".into(),
                description: "Activity feed across all personas. Filter / export CSV.".into(),
            },
            TourStep {
                id: "cluster".into(),
                title: "Cluster live".into(),
                href: "/admin/cluster".into(),
                description: "Live Raft state — term, leader, WAL apply lag.".into(),
            },
        ],
        Persona::TenantAdmin => vec![
            TourStep {
                id: "keda".into(),
                title: "KEDA scaled workloads".into(),
                href: "/admin/keda".into(),
                description: "Your tenant's ScaledObjects and scaling events.".into(),
            },
            TourStep {
                id: "vault".into(),
                title: "Vault secrets".into(),
                href: "/admin/vault".into(),
                description:
                    "Secret metadata + access audit (NOT secret values — those live in cave-vault)."
                        .into(),
            },
            TourStep {
                id: "kubelet".into(),
                title: "Pods".into(),
                href: "/admin/kubelet".into(),
                description: "Live pod status across your tenant namespaces.".into(),
            },
            TourStep {
                id: "cri".into(),
                title: "Container runtime".into(),
                href: "/admin/cri".into(),
                description: "Per-sandbox and per-container state — exec, logs, attach.".into(),
            },
        ],
        Persona::Anonymous => vec![
            TourStep {
                id: "login".into(),
                title: "Sign in".into(),
                href: "/login".into(),
                description: "Sign in to begin. WebAuthn required for admin views.".into(),
            },
        ],
    }
}

/// Per-principal progress.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TourProgress {
    pub principal: String,
    pub persona: String,
    /// Step ids completed, in completion order.
    pub completed: Vec<String>,
    pub dismissed: bool,
}

impl TourProgress {
    pub fn is_complete(&self, persona: Persona) -> bool {
        let total = tour_for(persona).len();
        self.completed.len() >= total
    }

    pub fn percent_complete(&self, persona: Persona) -> u8 {
        let total = tour_for(persona).len();
        if total == 0 {
            return 100;
        }
        ((self.completed.len() * 100) / total) as u8
    }
}

#[derive(Debug, Default)]
pub struct OnboardingState {
    progress: RwLock<HashMap<String, TourProgress>>,
}

impl OnboardingState {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(principal: &str, persona: Persona) -> String {
        format!("{}::{}", principal, persona.as_str())
    }

    /// Read the current tour progress for the caller.
    pub fn read(&self, ctx: &RequestCtx) -> Result<TourProgress, OnboardError> {
        ctx.authorise(Permission::OnboardRead)?;
        let k = Self::key(&ctx.principal, ctx.persona);
        Ok(self
            .progress
            .read()
            .unwrap()
            .get(&k)
            .cloned()
            .unwrap_or(TourProgress {
                principal: ctx.principal.clone(),
                persona: ctx.persona.as_str().to_string(),
                completed: Vec::new(),
                dismissed: false,
            }))
    }

    /// Mark a step complete. Errors if the step id is not part of
    /// the caller persona's tour or if already complete.
    pub fn complete_step(&self, ctx: &RequestCtx, step_id: &str) -> Result<TourProgress, OnboardError> {
        ctx.authorise(Permission::OnboardWrite)?;
        let tour = tour_for(ctx.persona);
        if !tour.iter().any(|s| s.id == step_id) {
            return Err(OnboardError::UnknownStep(step_id.into()));
        }
        let k = Self::key(&ctx.principal, ctx.persona);
        let mut g = self.progress.write().unwrap();
        let entry = g.entry(k).or_insert_with(|| TourProgress {
            principal: ctx.principal.clone(),
            persona: ctx.persona.as_str().to_string(),
            completed: Vec::new(),
            dismissed: false,
        });
        if entry.completed.iter().any(|c| c == step_id) {
            return Err(OnboardError::AlreadyComplete(step_id.into()));
        }
        entry.completed.push(step_id.into());
        Ok(entry.clone())
    }

    /// Dismiss the tour permanently for this principal+persona.
    pub fn dismiss(&self, ctx: &RequestCtx) -> Result<(), OnboardError> {
        ctx.authorise(Permission::OnboardWrite)?;
        let k = Self::key(&ctx.principal, ctx.persona);
        let mut g = self.progress.write().unwrap();
        let entry = g.entry(k).or_insert_with(|| TourProgress {
            principal: ctx.principal.clone(),
            persona: ctx.persona.as_str().to_string(),
            completed: Vec::new(),
            dismissed: false,
        });
        entry.dismissed = true;
        Ok(())
    }

    /// Next un-completed step, if any.
    pub fn next_step(&self, ctx: &RequestCtx) -> Result<Option<TourStep>, OnboardError> {
        let p = self.read(ctx)?;
        if p.dismissed {
            return Ok(None);
        }
        let tour = tour_for(ctx.persona);
        Ok(tour.into_iter().find(|s| !p.completed.contains(&s.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(persona: Persona, perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer_as("acme", perms, persona)
    }

    #[test]
    fn platform_tour_has_5_steps() {
        let tour = tour_for(Persona::PlatformAdmin);
        assert_eq!(tour.len(), 5);
        assert_eq!(tour[0].id, "compliance");
    }

    #[test]
    fn tenant_tour_has_4_steps() {
        let tour = tour_for(Persona::TenantAdmin);
        assert_eq!(tour.len(), 4);
    }

    #[test]
    fn anonymous_tour_directs_to_login() {
        let tour = tour_for(Persona::Anonymous);
        assert_eq!(tour.len(), 1);
        assert_eq!(tour[0].href, "/login");
    }

    #[test]
    fn read_returns_empty_for_new_principal() {
        let s = OnboardingState::new();
        let p = s
            .read(&ctx(Persona::PlatformAdmin, &[Permission::OnboardRead]))
            .unwrap();
        assert!(p.completed.is_empty());
    }

    #[test]
    fn complete_step_advances_progress() {
        let s = OnboardingState::new();
        let c = ctx(
            Persona::PlatformAdmin,
            &[Permission::OnboardRead, Permission::OnboardWrite],
        );
        s.complete_step(&c, "compliance").unwrap();
        let p = s.read(&c).unwrap();
        assert_eq!(p.completed, vec!["compliance".to_string()]);
        assert_eq!(p.percent_complete(Persona::PlatformAdmin), 20);
    }

    #[test]
    fn complete_unknown_step_errors() {
        let s = OnboardingState::new();
        let c = ctx(
            Persona::PlatformAdmin,
            &[Permission::OnboardRead, Permission::OnboardWrite],
        );
        assert!(matches!(
            s.complete_step(&c, "bogus").unwrap_err(),
            OnboardError::UnknownStep(_)
        ));
    }

    #[test]
    fn complete_twice_errors() {
        let s = OnboardingState::new();
        let c = ctx(
            Persona::PlatformAdmin,
            &[Permission::OnboardRead, Permission::OnboardWrite],
        );
        s.complete_step(&c, "compliance").unwrap();
        assert!(matches!(
            s.complete_step(&c, "compliance").unwrap_err(),
            OnboardError::AlreadyComplete(_)
        ));
    }

    #[test]
    fn next_step_returns_first_uncompleted() {
        let s = OnboardingState::new();
        let c = ctx(
            Persona::PlatformAdmin,
            &[Permission::OnboardRead, Permission::OnboardWrite],
        );
        s.complete_step(&c, "compliance").unwrap();
        let ns = s.next_step(&c).unwrap().unwrap();
        assert_eq!(ns.id, "upstream");
    }

    #[test]
    fn dismiss_blocks_next_step() {
        let s = OnboardingState::new();
        let c = ctx(
            Persona::PlatformAdmin,
            &[Permission::OnboardRead, Permission::OnboardWrite],
        );
        s.dismiss(&c).unwrap();
        assert!(s.next_step(&c).unwrap().is_none());
    }

    #[test]
    fn personas_have_separate_progress() {
        let s = OnboardingState::new();
        let pc = ctx(
            Persona::PlatformAdmin,
            &[Permission::OnboardRead, Permission::OnboardWrite],
        );
        let tc = ctx(
            Persona::TenantAdmin,
            &[Permission::OnboardRead, Permission::OnboardWrite],
        );
        s.complete_step(&pc, "compliance").unwrap();
        // Tenant persona uses different step ids; "compliance"
        // shouldn't appear in their tour.
        assert!(matches!(
            s.complete_step(&tc, "compliance").unwrap_err(),
            OnboardError::UnknownStep(_)
        ));
        // Independent progress.
        assert!(s.read(&tc).unwrap().completed.is_empty());
    }

    #[test]
    fn percent_complete_for_fully_done_tour() {
        let s = OnboardingState::new();
        let c = ctx(
            Persona::PlatformAdmin,
            &[Permission::OnboardRead, Permission::OnboardWrite],
        );
        for step in tour_for(Persona::PlatformAdmin) {
            s.complete_step(&c, &step.id).unwrap();
        }
        let p = s.read(&c).unwrap();
        assert!(p.is_complete(Persona::PlatformAdmin));
        assert_eq!(p.percent_complete(Persona::PlatformAdmin), 100);
    }

    #[test]
    fn write_refuses_without_permission() {
        let s = OnboardingState::new();
        let c = ctx(Persona::PlatformAdmin, &[Permission::OnboardRead]);
        assert!(matches!(
            s.complete_step(&c, "compliance").unwrap_err(),
            OnboardError::Auth(_)
        ));
    }
}
