// SPDX-License-Identifier: AGPL-3.0-or-later
//! `key-auth` plugin — API key in header or query.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_str_array, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let header_names = cfg_str_array(cfg, "key_names");
    let header_names = if header_names.is_empty() {
        vec!["apikey".to_string()]
    } else {
        header_names
    };
    let valid_keys = cfg_str_array(cfg, "valid_keys");
    let key_in_body = cfg.get("key_in_body").and_then(|v| v.as_bool()).unwrap_or(false);

    let mut found: Option<String> = None;
    for name in &header_names {
        let lname = name.to_lowercase();
        if let Some(v) = ctx.request.headers.get(&lname) {
            found = Some(v.clone());
            break;
        }
        if let Some(q) = extract_query(&ctx.request.uri, name) {
            found = Some(q);
            break;
        }
    }
    if found.is_none() && key_in_body {
        let body = std::str::from_utf8(&ctx.request.body).unwrap_or("");
        for name in &header_names {
            if let Some(v) = extract_form(body, name) {
                found = Some(v);
                break;
            }
        }
    }

    let key = match found {
        Some(k) => k,
        None => return Err(AGwError::Unauthorized("missing api key".into())),
    };
    if !valid_keys.is_empty() && !valid_keys.contains(&key) {
        return Err(AGwError::Unauthorized("invalid api key".into()));
    }
    ctx.request.headers.insert("x-consumer-credential-type".into(), "key-auth".into());
    Ok(None)
}

fn extract_query(uri: &str, name: &str) -> Option<String> {
    let (_, q) = uri.split_once('?')?;
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == name {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn extract_form(body: &str, name: &str) -> Option<String> {
    for pair in body.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == name {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;

    fn ctx(req: GwRequest) -> PluginContext {
        PluginContext::new(req, None, Route::new("r"))
    }

    #[test]
    fn missing_key_rejected() {
        let mut c = ctx(GwRequest::new("GET", "/", "h"));
        assert!(access(&serde_json::json!({}), &mut c).is_err());
    }

    #[test]
    fn header_key_accepted() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("apikey", "secret"));
        let r = access(&serde_json::json!({ "valid_keys": ["secret"] }), &mut c).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn invalid_key_rejected() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("apikey", "bad"));
        assert!(access(&serde_json::json!({ "valid_keys": ["good"] }), &mut c).is_err());
    }

    #[test]
    fn query_string_key_accepted() {
        let mut c = ctx(GwRequest::new("GET", "/?apikey=secret", "h"));
        let r = access(&serde_json::json!({ "valid_keys": ["secret"] }), &mut c).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn custom_header_name() {
        let mut c = ctx(GwRequest::new("GET", "/", "h").header("x-api-key", "secret"));
        let r = access(
            &serde_json::json!({ "key_names": ["x-api-key"], "valid_keys": ["secret"] }),
            &mut c,
        )
        .unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn body_form_key_accepted_when_enabled() {
        let req = GwRequest::new("POST", "/", "h").body("apikey=secret&x=1".as_bytes().to_vec());
        let mut c = ctx(req);
        let r = access(
            &serde_json::json!({ "valid_keys": ["secret"], "key_in_body": true }),
            &mut c,
        )
        .unwrap();
        assert!(r.is_none());
    }
}
