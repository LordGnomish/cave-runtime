// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/device` — RFC 8628 device-flow inspector + user-verification view.
//!
//! Surfaces in-flight `device_code` records (pending / approved / denied /
//! expired). The user-facing verification page itself is served by
//! `cave_auth::keycloak::device_endpoint` (`GET /realms/{realm}/device`);
//! this admin page shows the inspector + the URL for that page.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFlowRow {
    pub device_code: String,
    pub user_code: String,
    pub realm: String,
    pub client_id: String,
    pub status: String,
}

pub fn seeded_flows() -> Vec<DeviceFlowRow> {
    vec![
        DeviceFlowRow {
            device_code: "dev-abc123".into(), user_code: "BCDF-GHJK".into(),
            realm: "acme-realm".into(), client_id: "cli-tv".into(),
            status: "pending".into(),
        },
        DeviceFlowRow {
            device_code: "dev-def456".into(), user_code: "LMNP-QRST".into(),
            realm: "acme-realm".into(), client_id: "cli-tv".into(),
            status: "approved".into(),
        },
    ]
}

pub fn render(_state: &AdminState, ctx: &RequestCtx) -> Result<String, super::AuthViewError> {
    ctx.authorise(Permission::AuthDeviceRead)?;
    let flows = seeded_flows();
    let rows: Vec<Vec<String>> = flows.iter().map(|f| vec![
        escape(&f.device_code),
        escape(&f.user_code),
        escape(&f.realm),
        escape(&f.client_id),
        escape(&f.status),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    RFC 8628 device-authorization-grant inspector.
    Backend: <code>cave_auth::keycloak::device_endpoint</code>.
  </p>
  <h2 class="text-lg font-semibold mb-2">In-flight device flows ({n})</h2>
  {tbl}
  <div class="mt-4 text-sm">
    <strong>User verification page:</strong>
    <code>/realms/&lt;realm&gt;/device</code> — operators can type their
    user_code there to approve or deny the device.
    <br>
    cavectl: <code>cavectl auth device poll &lt;device_code&gt;</code> for tests.
  </div>
</section>"#,
        n = flows.len(),
        tbl = table(&["device_code", "user_code", "realm", "client_id", "status"], &rows),
    );
    Ok(page_shell_full(ctx, "/admin/auth/device", &format!("auth/device · {}", escape(ctx.tenant.as_str())), &body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_permission() {
        assert!(render(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_emits_inspector_table() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthDeviceRead])).unwrap();
        assert!(html.contains("RFC 8628"));
        assert!(html.contains("BCDF-GHJK"));
        assert!(html.contains("LMNP-QRST"));
    }

    #[test]
    fn render_links_to_user_page() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthDeviceRead])).unwrap();
        assert!(html.contains("/realms/&lt;realm&gt;/device"));
    }

    #[test]
    fn seeded_has_two_flows_one_pending_one_approved() {
        let f = seeded_flows();
        assert_eq!(f.len(), 2);
        assert!(f.iter().any(|x| x.status == "pending"));
        assert!(f.iter().any(|x| x.status == "approved"));
    }
}
