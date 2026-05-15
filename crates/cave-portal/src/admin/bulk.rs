//! Bulk-operation helper.
//!
//! Lets the dashboard tick checkboxes next to N resources and submit
//! one POST that pauses / resumes / deletes them all. The endpoint
//! returns a typed [`BulkOpResult`] per target so the page can render
//! per-row success / error chips.
//!
//! Two modes:
//! * `all_or_nothing = true` — the first failure aborts; nothing
//!   else is touched, partial state is rolled back via the caller
//!   `rollback` callback. Useful for paired ops like
//!   "pause+notify".
//! * `all_or_nothing = false` (default) — best-effort: failures are
//!   recorded but the rest of the batch proceeds.

use crate::admin::permission::{Permission, RequestCtx};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BulkOpError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("bulk op {kind:?} aborted at target {target:?}: {detail}")]
    Aborted {
        kind: BulkOpKind,
        target: String,
        detail: String,
    },
    #[error("empty target list")]
    EmptyTargets,
    #[error("target count {0} exceeds bulk limit {1}")]
    LimitExceeded(usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BulkOpKind {
    PauseScaledObject,
    ResumeScaledObject,
    DeleteScaledObject,
    ExportVaultSecret,
    ImportVaultSecret,
    AckAlert,
    RotateCertificate,
}

impl BulkOpKind {
    pub const fn required_permission(self) -> Permission {
        match self {
            BulkOpKind::PauseScaledObject
            | BulkOpKind::ResumeScaledObject
            | BulkOpKind::DeleteScaledObject => Permission::KedaWrite,
            BulkOpKind::ExportVaultSecret => Permission::VaultRead,
            BulkOpKind::ImportVaultSecret => Permission::VaultRead, // upgraded in real handler
            BulkOpKind::AckAlert => Permission::AlertsAck,
            BulkOpKind::RotateCertificate => Permission::CertsRead,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            BulkOpKind::PauseScaledObject => "pause_scaledobject",
            BulkOpKind::ResumeScaledObject => "resume_scaledobject",
            BulkOpKind::DeleteScaledObject => "delete_scaledobject",
            BulkOpKind::ExportVaultSecret => "export_vault_secret",
            BulkOpKind::ImportVaultSecret => "import_vault_secret",
            BulkOpKind::AckAlert => "ack_alert",
            BulkOpKind::RotateCertificate => "rotate_certificate",
        }
    }
}

pub const MAX_BULK_TARGETS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkOpRequest {
    pub kind: BulkOpKind,
    pub targets: Vec<String>,
    pub all_or_nothing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BulkTargetResult {
    Ok,
    Failed(String),
    /// Set when the batch was aborted before this target was reached
    /// (`all_or_nothing` mode).
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkOpResult {
    pub kind: BulkOpKind,
    pub all_or_nothing: bool,
    /// Per-target outcome, in submission order.
    pub results: Vec<(String, BulkTargetResult)>,
    pub ok_count: usize,
    pub fail_count: usize,
    pub aborted_count: usize,
}

impl BulkOpResult {
    pub fn is_full_success(&self) -> bool {
        self.fail_count == 0 && self.aborted_count == 0
    }
}

/// Per-target executor closure. Returns `Ok(())` on success or a
/// failure detail string. Used by handlers as an injection point so
/// tests can drive deterministic outcomes.
pub trait BulkExecutor: Send + Sync {
    fn execute(&self, kind: BulkOpKind, target: &str) -> Result<(), String>;
}

/// Vec-of-failures stub executor for tests.
pub struct FixedExecutor {
    pub failures: std::collections::HashSet<String>,
}

impl FixedExecutor {
    pub fn new() -> Self {
        Self {
            failures: Default::default(),
        }
    }

    pub fn with_failures(targets: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            failures: targets.into_iter().map(|t| t.into()).collect(),
        }
    }
}

impl BulkExecutor for FixedExecutor {
    fn execute(&self, _kind: BulkOpKind, target: &str) -> Result<(), String> {
        if self.failures.contains(target) {
            Err(format!("fixture failure for {target}"))
        } else {
            Ok(())
        }
    }
}

/// Submit a bulk op. The executor is called once per target. With
/// `all_or_nothing = true`, the first failure halts execution and
/// the remaining targets are flagged `Aborted`. With
/// `all_or_nothing = false`, every target is attempted.
pub fn submit(
    ctx: &RequestCtx,
    req: &BulkOpRequest,
    executor: &dyn BulkExecutor,
) -> Result<BulkOpResult, BulkOpError> {
    ctx.authorise(Permission::BulkOpsSubmit)?;
    ctx.authorise(req.kind.required_permission())?;
    if req.targets.is_empty() {
        return Err(BulkOpError::EmptyTargets);
    }
    if req.targets.len() > MAX_BULK_TARGETS {
        return Err(BulkOpError::LimitExceeded(req.targets.len(), MAX_BULK_TARGETS));
    }
    let mut results = Vec::with_capacity(req.targets.len());
    let mut ok = 0;
    let mut fail = 0;
    let mut aborted = 0;
    let mut halt_after = None;
    for (i, t) in req.targets.iter().enumerate() {
        if halt_after.is_some_and(|idx| i > idx) {
            results.push((t.clone(), BulkTargetResult::Aborted));
            aborted += 1;
            continue;
        }
        match executor.execute(req.kind, t) {
            Ok(()) => {
                results.push((t.clone(), BulkTargetResult::Ok));
                ok += 1;
            }
            Err(detail) => {
                results.push((t.clone(), BulkTargetResult::Failed(detail)));
                fail += 1;
                if req.all_or_nothing {
                    halt_after = Some(i);
                }
            }
        }
    }
    Ok(BulkOpResult {
        kind: req.kind,
        all_or_nothing: req.all_or_nothing,
        results,
        ok_count: ok,
        fail_count: fail,
        aborted_count: aborted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_full() -> RequestCtx {
        RequestCtx::developer(
            "acme",
            &[Permission::BulkOpsSubmit, Permission::KedaWrite],
        )
    }

    fn req(kind: BulkOpKind, targets: &[&str], aon: bool) -> BulkOpRequest {
        BulkOpRequest {
            kind,
            targets: targets.iter().map(|s| s.to_string()).collect(),
            all_or_nothing: aon,
        }
    }

    #[test]
    fn submit_refuses_without_bulk_permission() {
        let ctx = RequestCtx::developer("acme", &[Permission::KedaWrite]);
        let r = req(BulkOpKind::PauseScaledObject, &["a", "b"], false);
        assert!(matches!(
            submit(&ctx, &r, &FixedExecutor::new()).unwrap_err(),
            BulkOpError::Auth(_)
        ));
    }

    #[test]
    fn submit_refuses_without_kind_permission() {
        let ctx = RequestCtx::developer("acme", &[Permission::BulkOpsSubmit]);
        let r = req(BulkOpKind::PauseScaledObject, &["a"], false);
        assert!(matches!(
            submit(&ctx, &r, &FixedExecutor::new()).unwrap_err(),
            BulkOpError::Auth(_)
        ));
    }

    #[test]
    fn empty_targets_errors() {
        let r = req(BulkOpKind::PauseScaledObject, &[], false);
        assert!(matches!(
            submit(&ctx_full(), &r, &FixedExecutor::new()).unwrap_err(),
            BulkOpError::EmptyTargets
        ));
    }

    #[test]
    fn over_limit_errors() {
        let targets: Vec<String> = (0..MAX_BULK_TARGETS + 1).map(|i| format!("t{i}")).collect();
        let targets_ref: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
        let r = req(BulkOpKind::PauseScaledObject, &targets_ref, false);
        assert!(matches!(
            submit(&ctx_full(), &r, &FixedExecutor::new()).unwrap_err(),
            BulkOpError::LimitExceeded(_, _)
        ));
    }

    #[test]
    fn best_effort_continues_past_failures() {
        let exec = FixedExecutor::with_failures(["b"]);
        let r = req(BulkOpKind::PauseScaledObject, &["a", "b", "c"], false);
        let result = submit(&ctx_full(), &r, &exec).unwrap();
        assert_eq!(result.ok_count, 2);
        assert_eq!(result.fail_count, 1);
        assert_eq!(result.aborted_count, 0);
        assert!(matches!(result.results[1].1, BulkTargetResult::Failed(_)));
    }

    #[test]
    fn all_or_nothing_aborts_after_first_failure() {
        let exec = FixedExecutor::with_failures(["b"]);
        let r = req(BulkOpKind::PauseScaledObject, &["a", "b", "c", "d"], true);
        let result = submit(&ctx_full(), &r, &exec).unwrap();
        assert_eq!(result.ok_count, 1); // "a"
        assert_eq!(result.fail_count, 1); // "b"
        assert_eq!(result.aborted_count, 2); // "c", "d"
        assert!(matches!(result.results[0].1, BulkTargetResult::Ok));
        assert!(matches!(result.results[2].1, BulkTargetResult::Aborted));
    }

    #[test]
    fn is_full_success_only_when_zero_failures() {
        let exec = FixedExecutor::new();
        let r = req(BulkOpKind::PauseScaledObject, &["a", "b"], false);
        let result = submit(&ctx_full(), &r, &exec).unwrap();
        assert!(result.is_full_success());
        let exec2 = FixedExecutor::with_failures(["a"]);
        let result2 = submit(&ctx_full(), &r, &exec2).unwrap();
        assert!(!result2.is_full_success());
    }

    #[test]
    fn result_preserves_submission_order() {
        let exec = FixedExecutor::new();
        let r = req(BulkOpKind::PauseScaledObject, &["a", "b", "c"], false);
        let result = submit(&ctx_full(), &r, &exec).unwrap();
        let labels: Vec<&str> = result.results.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(labels, vec!["a", "b", "c"]);
    }

    #[test]
    fn kind_required_permission_maps_correctly() {
        assert_eq!(BulkOpKind::PauseScaledObject.required_permission(), Permission::KedaWrite);
        assert_eq!(BulkOpKind::AckAlert.required_permission(), Permission::AlertsAck);
    }

    #[test]
    fn kind_as_str_stable() {
        assert_eq!(BulkOpKind::PauseScaledObject.as_str(), "pause_scaledobject");
        assert_eq!(BulkOpKind::DeleteScaledObject.as_str(), "delete_scaledobject");
    }
}
