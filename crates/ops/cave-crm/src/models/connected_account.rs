// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM ConnectedAccount — `packages/twenty-server/src/modules/connected-account/standard-objects/connected-account.workspace-entity.ts`
//!
//! A workspace-member's linked mailbox / calendar provider. The message
//! and calendar sync workers (themselves scope-cut to a later mail-bridge
//! ray) read OAuth credentials from here; this module ports the control-
//! plane shape + the credential-health state machine (`authFailedAt` set
//! on failure, cleared on a successful refresh). The provider enum mirrors
//! `twenty-shared/src/types/ConnectedAccountProvider.ts`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Linked-account provider — exact lowercase wire values per
/// `ConnectedAccountProvider`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectedAccountProvider {
    Google,
    Microsoft,
    ImapSmtpCaldav,
    Oidc,
    Saml,
    EmailGroup,
    App,
}

/// ConnectedAccount workspace-entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectedAccount {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Twenty `accountOwnerId` — the owning WorkspaceMember (non-null).
    pub account_owner_id: Uuid,
    pub provider: ConnectedAccountProvider,
    /// Twenty `handle` (primary address, TEXT nullable).
    pub handle: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    /// Twenty `handleAliases` — additional addresses this account speaks for.
    pub handle_aliases: Vec<String>,
    /// Twenty `scopes` — granted OAuth scopes.
    pub scopes: Vec<String>,
    pub last_sync_history_id: Option<String>,
    pub last_credentials_refreshed_at: Option<DateTime<Utc>>,
    /// Twenty `authFailedAt` — set when a sync hits an auth error.
    pub auth_failed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ConnectedAccount {
    /// Link a freshly-authorized account. Tokens / scopes are filled in by
    /// the OAuth callback; a new account starts healthy (no `authFailedAt`).
    pub fn new(
        workspace_id: Uuid,
        account_owner_id: Uuid,
        provider: ConnectedAccountProvider,
        handle: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            account_owner_id,
            provider,
            handle: Some(handle.into()),
            access_token: None,
            refresh_token: None,
            handle_aliases: Vec::new(),
            scopes: Vec::new(),
            last_sync_history_id: None,
            last_credentials_refreshed_at: None,
            auth_failed_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Credentials are usable iff no auth failure is outstanding.
    pub fn is_healthy(&self) -> bool {
        self.auth_failed_at.is_none()
    }

    /// Record an auth failure observed at `at` — the sync worker stops
    /// polling until the member re-grants and a refresh clears the flag.
    pub fn mark_auth_failed(&mut self, at: DateTime<Utc>) {
        self.auth_failed_at = Some(at);
        self.updated_at = at;
    }

    /// Record a successful credential refresh at `at`, clearing any
    /// outstanding auth failure.
    pub fn record_refresh(&mut self, at: DateTime<Utc>) {
        self.last_credentials_refreshed_at = Some(at);
        self.auth_failed_at = None;
        self.updated_at = at;
    }

    /// Primary handle followed by aliases — every address this account
    /// can send/receive as.
    pub fn all_handles(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(h) = &self.handle {
            out.push(h.clone());
        }
        out.extend(self.handle_aliases.iter().cloned());
        out
    }

    /// Case-insensitive membership check across the primary handle and
    /// every alias.
    pub fn owns_handle(&self, candidate: &str) -> bool {
        let candidate = candidate.trim().to_ascii_lowercase();
        self.all_handles()
            .iter()
            .any(|h| h.trim().to_ascii_lowercase() == candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct() -> ConnectedAccount {
        ConnectedAccount::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            ConnectedAccountProvider::Google,
            "ada@acme.test",
        )
    }

    #[test]
    fn new_is_healthy_with_no_tokens() {
        let a = acct();
        assert_eq!(a.provider, ConnectedAccountProvider::Google);
        assert_eq!(a.handle.as_deref(), Some("ada@acme.test"));
        assert!(a.is_healthy());
        assert!(a.access_token.is_none());
    }

    #[test]
    fn provider_serializes_lowercase_wire_values() {
        let to = |p| serde_json::to_value(&p).unwrap();
        assert_eq!(to(ConnectedAccountProvider::Google), "google");
        assert_eq!(to(ConnectedAccountProvider::Microsoft), "microsoft");
        assert_eq!(to(ConnectedAccountProvider::ImapSmtpCaldav), "imap_smtp_caldav");
        assert_eq!(to(ConnectedAccountProvider::Oidc), "oidc");
        assert_eq!(to(ConnectedAccountProvider::Saml), "saml");
        assert_eq!(to(ConnectedAccountProvider::EmailGroup), "email_group");
        assert_eq!(to(ConnectedAccountProvider::App), "app");
    }

    #[test]
    fn mark_auth_failed_then_refresh_recovers_health() {
        let mut a = acct();
        let t0 = Utc::now();
        a.mark_auth_failed(t0);
        assert!(!a.is_healthy());
        assert_eq!(a.auth_failed_at, Some(t0));

        let t1 = t0 + chrono::Duration::seconds(30);
        a.record_refresh(t1);
        assert!(a.is_healthy());
        assert_eq!(a.auth_failed_at, None);
        assert_eq!(a.last_credentials_refreshed_at, Some(t1));
    }

    #[test]
    fn owns_handle_matches_primary_and_aliases_case_insensitively() {
        let mut a = acct();
        a.handle_aliases = vec!["ada.lovelace@acme.test".into()];
        assert!(a.owns_handle("ADA@ACME.TEST"));
        assert!(a.owns_handle("Ada.Lovelace@Acme.Test"));
        assert!(!a.owns_handle("someone@else.test"));
    }

    #[test]
    fn all_handles_includes_primary_then_aliases() {
        let mut a = acct();
        a.handle_aliases = vec!["alt@acme.test".into()];
        assert_eq!(a.all_handles(), vec!["ada@acme.test", "alt@acme.test"]);
    }
}
