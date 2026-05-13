//! `/admin/audit` — append-only activity log.
//!
//! Every persona action that mutates Portal state (a KEDA pause, a
//! vault read, a compliance refresh, ...) appends a typed
//! [`AuditEntry`]. Platform admins can browse the log; the page
//! supports filter-by-date / persona / action / target and a CSV
//! export.
//!
//! The log is in-memory with a bounded ring. Production wires a
//! persistent backend in cave-runtime's bootstrap and replays
//! entries on restart; that's out of scope for this module.

pub mod entries;

use crate::admin::permission::{Permission, Persona, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use std::sync::Arc;

pub use entries::{AuditAction, AuditEntry, AuditResult, AuditStore};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuditViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("audit log requires Platform admin persona")]
    PersonaRequired,
}

/// Filter parameters parsed from `/admin/audit?...` query string.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditFilter {
    pub from_unix: Option<i64>,
    pub to_unix: Option<i64>,
    pub persona: Option<String>,
    pub action: Option<String>,
    pub target: Option<String>,
}

impl AuditFilter {
    pub fn matches(&self, e: &AuditEntry) -> bool {
        if let Some(from) = self.from_unix {
            if e.timestamp_unix < from {
                return false;
            }
        }
        if let Some(to) = self.to_unix {
            if e.timestamp_unix > to {
                return false;
            }
        }
        if let Some(p) = &self.persona {
            if !e.persona.eq_ignore_ascii_case(p) {
                return false;
            }
        }
        if let Some(a) = &self.action {
            if !e.action.as_str().eq_ignore_ascii_case(a) {
                return false;
            }
        }
        if let Some(t) = &self.target {
            if !e.target.contains(t) {
                return false;
            }
        }
        true
    }
}

pub fn list_entries(
    store: &AuditStore,
    ctx: &RequestCtx,
    filter: &AuditFilter,
) -> Result<Vec<AuditEntry>, AuditViewError> {
    if ctx.persona != Persona::PlatformAdmin {
        return Err(AuditViewError::PersonaRequired);
    }
    ctx.authorise(Permission::AuditRead)?;
    let entries = store.list();
    Ok(entries
        .into_iter()
        .filter(|e| filter.matches(e))
        .collect())
}

/// Export the matching rows as CSV. Header row is always emitted so
/// the download is valid even when no rows match.
pub fn export_csv(
    store: &AuditStore,
    ctx: &RequestCtx,
    filter: &AuditFilter,
) -> Result<String, AuditViewError> {
    let rows = list_entries(store, ctx, filter)?;
    let mut out = String::from("timestamp_unix,persona,action,target,result,detail\n");
    for r in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{}\n",
            r.timestamp_unix,
            csv_escape(&r.persona),
            csv_escape(r.action.as_str()),
            csv_escape(&r.target),
            csv_escape(r.result.as_str()),
            csv_escape(&r.detail),
        ));
    }
    Ok(out)
}

pub fn render(
    store: Arc<AuditStore>,
    ctx: &RequestCtx,
    filter: &AuditFilter,
) -> Result<String, AuditViewError> {
    let rows = list_entries(&store, ctx, filter)?;
    let counts = action_summary(&rows);
    let chips: String = counts
        .iter()
        .map(|(a, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{a} <strong>×{n}</strong></span>"#,
                a = escape(a),
                n = n
            )
        })
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.timestamp_unix.to_string(),
                escape(&r.persona),
                r.action.as_str().to_string(),
                escape(&r.target),
                r.result.as_str().to_string(),
                escape(&r.detail),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Append-only activity log. Platform admin only. <a class="text-blue-700 underline" href="/admin/audit.csv?tenant_id={tid}">Export CSV</a></p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> entries</span>
  </div>
  <div class="mb-4">{chips}</div>
  {tbl}
</section>"#,
        tid = escape(ctx.tenant.as_str()),
        n = rows.len(),
        chips = chips,
        tbl = table(
            &["ts", "persona", "action", "target", "result", "detail"],
            &table_rows
        ),
    );
    Ok(page_shell(&format!("audit · {}", escape(ctx.tenant.as_str())), &body))
}

fn action_summary(rows: &[AuditEntry]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.action.as_str().to_string()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(persona: &str, action: AuditAction, target: &str, ts: i64) -> AuditEntry {
        AuditEntry {
            timestamp_unix: ts,
            persona: persona.into(),
            action,
            target: target.into(),
            result: AuditResult::Ok,
            detail: String::new(),
        }
    }

    fn platform_ctx() -> RequestCtx {
        RequestCtx::developer("acme", &[Permission::AuditRead])
    }

    fn tenant_ctx() -> RequestCtx {
        RequestCtx::developer_as("acme", &[Permission::AuditRead], Persona::TenantAdmin)
    }

    #[test]
    fn list_requires_platform_admin() {
        let store = AuditStore::new(100);
        let err = list_entries(&store, &tenant_ctx(), &AuditFilter::default()).unwrap_err();
        assert!(matches!(err, AuditViewError::PersonaRequired));
    }

    #[test]
    fn list_refuses_without_permission() {
        let store = AuditStore::new(100);
        let ctx = RequestCtx::developer("acme", &[]);
        assert!(matches!(
            list_entries(&store, &ctx, &AuditFilter::default()).unwrap_err(),
            AuditViewError::Auth(_)
        ));
    }

    #[test]
    fn filter_by_date_range_inclusive() {
        let store = AuditStore::new(100);
        store.append(entry("a", AuditAction::Read, "x", 100));
        store.append(entry("a", AuditAction::Read, "x", 200));
        store.append(entry("a", AuditAction::Read, "x", 300));
        let f = AuditFilter {
            from_unix: Some(150),
            to_unix: Some(250),
            ..Default::default()
        };
        let rows = list_entries(&store, &platform_ctx(), &f).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].timestamp_unix, 200);
    }

    #[test]
    fn filter_by_persona_case_insensitive() {
        let store = AuditStore::new(100);
        store.append(entry("Alice", AuditAction::Read, "x", 1));
        store.append(entry("bob", AuditAction::Read, "x", 2));
        let f = AuditFilter {
            persona: Some("alice".into()),
            ..Default::default()
        };
        let rows = list_entries(&store, &platform_ctx(), &f).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn filter_by_action() {
        let store = AuditStore::new(100);
        store.append(entry("a", AuditAction::Read, "x", 1));
        store.append(entry("a", AuditAction::Write, "x", 2));
        let f = AuditFilter {
            action: Some("write".into()),
            ..Default::default()
        };
        let rows = list_entries(&store, &platform_ctx(), &f).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn filter_by_target_substring() {
        let store = AuditStore::new(100);
        store.append(entry("a", AuditAction::Read, "vault/secret/db-password", 1));
        store.append(entry("a", AuditAction::Read, "keda/scaledobject/echo", 2));
        let f = AuditFilter {
            target: Some("vault".into()),
            ..Default::default()
        };
        let rows = list_entries(&store, &platform_ctx(), &f).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn export_csv_emits_header_row_when_empty() {
        let store = AuditStore::new(100);
        let csv = export_csv(&store, &platform_ctx(), &AuditFilter::default()).unwrap();
        assert!(csv.starts_with("timestamp_unix,persona,action,target,result,detail\n"));
    }

    #[test]
    fn export_csv_escapes_commas_and_quotes() {
        let store = AuditStore::new(100);
        store.append(AuditEntry {
            detail: r#"key="value", more"#.into(),
            ..entry("a", AuditAction::Write, "x", 1)
        });
        let csv = export_csv(&store, &platform_ctx(), &AuditFilter::default()).unwrap();
        assert!(csv.contains(r#""key=""value"", more""#));
    }

    #[test]
    fn render_includes_filter_chip_and_count() {
        let store = Arc::new(AuditStore::new(100));
        store.append(entry("a", AuditAction::Read, "x", 1));
        let html = render(store, &platform_ctx(), &AuditFilter::default()).unwrap();
        assert!(html.contains("1</strong> entries"));
        assert!(html.contains("Export CSV"));
    }

    #[test]
    fn render_refuses_for_tenant_admin() {
        let store = Arc::new(AuditStore::new(100));
        assert!(matches!(
            render(store, &tenant_ctx(), &AuditFilter::default()).unwrap_err(),
            AuditViewError::PersonaRequired
        ));
    }
}
