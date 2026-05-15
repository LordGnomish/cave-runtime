//! `/admin/auth` view — user list + role assignment + RBAC matrix.
//!
//! Mirrors the user-management surface Backstage exposes through
//! `permission-backend`. The matrix view shows which user holds which role,
//! with the editor (assign / revoke) gated on `IamWrite`.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, IamRoleAssignment, IamUser};
use crate::admin::types::Cite;
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IamViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("user {0} not found in this tenant")]
    UserNotFound(String),
    #[error("role assignment ({user},{role}) already exists")]
    DuplicateAssignment { user: String, role: String },
    #[error("role assignment ({user},{role}) does not exist")]
    NoSuchAssignment { user: String, role: String },
}

pub fn list_users(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<IamUser>, IamViewError> {
    ctx.authorise(Permission::IamRead)?;
    Ok(scope(&state.iam_users.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

/// `username -> Vec<role>`, alphabetically sorted on both axes.
pub fn rbac_matrix(state: &AdminState, ctx: &RequestCtx) -> Result<BTreeMap<String, Vec<String>>, IamViewError> {
    ctx.authorise(Permission::IamRead)?;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let users = state.iam_users.read().unwrap();
    for u in scope(&users, &ctx.tenant, |r| &r.tenant) {
        out.entry(u.username.clone()).or_default();
    }
    let assigns = state.iam_assignments.read().unwrap();
    for a in scope(&assigns, &ctx.tenant, |r| &r.tenant) {
        out.entry(a.username.clone()).or_default().push(a.role.clone());
    }
    for roles in out.values_mut() {
        roles.sort();
        roles.dedup();
    }
    Ok(out)
}

/// Assign a role. Idempotency is enforced — duplicates are an error.
pub fn assign_role(
    state: &AdminState,
    ctx: &RequestCtx,
    username: &str,
    role: &str,
) -> Result<(), IamViewError> {
    ctx.authorise(Permission::IamWrite)?;
    {
        let users = state.iam_users.read().unwrap();
        if !scope(&users, &ctx.tenant, |r| &r.tenant)
            .iter()
            .any(|u| u.username == username)
        {
            return Err(IamViewError::UserNotFound(username.into()));
        }
    }
    let mut assigns = state.iam_assignments.write().unwrap();
    if assigns
        .iter()
        .any(|a| a.tenant == ctx.tenant && a.username == username && a.role == role)
    {
        return Err(IamViewError::DuplicateAssignment {
            user: username.into(),
            role: role.into(),
        });
    }
    assigns.push(IamRoleAssignment {
        tenant: ctx.tenant.clone(),
        username: username.into(),
        role: role.into(),
    });
    Ok(())
}

pub fn revoke_role(
    state: &AdminState,
    ctx: &RequestCtx,
    username: &str,
    role: &str,
) -> Result<(), IamViewError> {
    ctx.authorise(Permission::IamWrite)?;
    let mut assigns = state.iam_assignments.write().unwrap();
    let before = assigns.len();
    assigns.retain(|a| !(a.tenant == ctx.tenant && a.username == username && a.role == role));
    if assigns.len() == before {
        return Err(IamViewError::NoSuchAssignment {
            user: username.into(),
            role: role.into(),
        });
    }
    Ok(())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, IamViewError> {
    let users = list_users(state, ctx)?;
    let matrix = rbac_matrix(state, ctx)?;
    let user_rows: Vec<Vec<String>> =
        users.iter().map(|u| vec![u.username.clone(), u.email.clone()]).collect();
    let matrix_rows: Vec<Vec<String>> = matrix
        .iter()
        .map(|(u, roles)| vec![u.clone(), roles.join(", ")])
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Users ({n_user})</h2>{u_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">RBAC matrix</h2>{m_tbl}</section>"#,
        n_user = users.len(),
        u_tbl = table(&["username", "email"], &user_rows),
        m_tbl = table(&["user", "roles"], &matrix_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth",
        &format!("auth · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/permission-backend/src/index.ts", "PermissionPolicy");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_users_returns_only_owner_users() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "listUsers",
            "acme"
        );
        let state = AdminState::seeded();
        let users = list_users(&state, &ctx(&[Permission::IamRead])).unwrap();
        assert_eq!(users.len(), 2);
        assert!(!users.iter().any(|u| u.username == "mallory"));
    }

    #[test]
    fn rbac_matrix_includes_users_with_no_roles() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/components/PermissionMatrix.tsx",
            "PermissionMatrix",
            "acme"
        );
        let state = AdminState::empty();
        state.iam_users.write().unwrap().push(IamUser {
            tenant: ctx(&[]).tenant,
            username: "carol".into(),
            email: "c@acme".into(),
        });
        let m = rbac_matrix(&state, &ctx(&[Permission::IamRead])).unwrap();
        assert_eq!(m.get("carol"), Some(&vec![]));
    }

    #[test]
    fn assign_role_appends_and_is_visible_in_matrix() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "addRoleAssignment",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::IamRead, Permission::IamWrite]);
        assign_role(&state, &c, "alice", "auditor").unwrap();
        let m = rbac_matrix(&state, &c).unwrap();
        let alice_roles = m.get("alice").unwrap();
        assert!(alice_roles.contains(&"admin".to_string()));
        assert!(alice_roles.contains(&"auditor".to_string()));
    }

    #[test]
    fn assign_role_rejects_unknown_user() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "validateUser",
            "acme"
        );
        let state = AdminState::seeded();
        let err = assign_role(
            &state,
            &ctx(&[Permission::IamRead, Permission::IamWrite]),
            "ghost",
            "viewer",
        )
        .unwrap_err();
        assert!(matches!(err, IamViewError::UserNotFound(_)));
    }

    #[test]
    fn assign_role_rejects_duplicate() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "duplicateGuard",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::IamRead, Permission::IamWrite]);
        let err = assign_role(&state, &c, "alice", "admin").unwrap_err();
        assert!(matches!(err, IamViewError::DuplicateAssignment { .. }));
    }

    #[test]
    fn revoke_role_removes_existing_pair_and_errors_on_missing() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "removeRoleAssignment",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::IamRead, Permission::IamWrite]);
        revoke_role(&state, &c, "bob", "viewer").unwrap();
        let err = revoke_role(&state, &c, "bob", "viewer").unwrap_err();
        assert!(matches!(err, IamViewError::NoSuchAssignment { .. }));
    }
}
