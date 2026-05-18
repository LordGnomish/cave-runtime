// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::{extract::{Request, State}, middleware::Next, response::Response};
use std::sync::Arc;
use crate::VaultState;
use crate::error::VaultError;

pub const VAULT_TOKEN_HEADER: &str = "X-Vault-Token";
pub const VAULT_NAMESPACE_HEADER: &str = "X-Vault-Namespace";

pub async fn require_token(
    State(state): State<Arc<VaultState>>,
    mut req: Request,
    next: Next,
) -> Result<Response, VaultError> {
    {
        let seal = state.seal_state.read().await;
        if seal.is_sealed() {
            return Err(VaultError::Sealed);
        }
    }

    let token_id = req.headers()
        .get(VAULT_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or(VaultError::BadToken)?;

    let ts = state.token_store.read().await;
    if ts.lookup(&token_id).is_none() {
        return Err(VaultError::TokenNotFound);
    }
    drop(ts);

    req.extensions_mut().insert(token_id);
    Ok(next.run(req).await)
}
