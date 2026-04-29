//! Realm seed bundling.
//!
//! Ships the canonical `cave` realm export as a baked-in JSON document so
//! ops never has to fetch a separate file. Used at first start to bootstrap
//! Keycloak via `kc.sh import` (or via `realm-export` Job in deploy/) — and
//! at runtime for tests that need a known-good realm shape.
//!
//! The seed file lives at `crates/cave-auth/seed/cave-realm.json` and is
//! template-substituted at runtime: `${VAR:-default}` is replaced with the
//! environment variable's value or the literal default.

use std::collections::BTreeMap;

/// Raw seed JSON, template-substituted at runtime by [`render_seed`].
const RAW_REALM_SEED: &str = include_str!("../seed/cave-realm.json");

/// Render the bundled realm seed with environment-variable substitution.
///
/// Substitutes `${NAME}` and `${NAME:-default}` placeholders against the
/// supplied env map. Variables that are absent and have no default fall
/// through unchanged so missing config surfaces in the JSON parser rather
/// than silently shipping empty values.
pub fn render_seed(env: &BTreeMap<String, String>) -> String {
    substitute(RAW_REALM_SEED, env)
}

/// Render with the process's real environment.
pub fn render_seed_from_env() -> String {
    let mut env = BTreeMap::new();
    for (k, v) in std::env::vars() {
        env.insert(k, v);
    }
    render_seed(&env)
}

/// `${NAME}` and `${NAME:-default}` substitution. Single pass, no recursion;
/// brace-balanced; matches a leading `\` as a literal-escape so JSON content
/// can include the literal sequence `\${X}` if needed.
pub fn substitute(input: &str, env: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Escape: `\$` emits a literal `$` and skips one byte.
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            out.push('$');
            i += 2;
            continue;
        }
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Find matching closing brace
            let mut j = i + 2;
            while j < bytes.len() && bytes[j] != b'}' {
                j += 1;
            }
            if j == bytes.len() {
                // unmatched, emit literally
                out.push(bytes[i] as char);
                i += 1;
                continue;
            }
            let body = &input[i + 2..j];
            // Parse `NAME` or `NAME:-default`
            let (name, default) = match body.find(":-") {
                Some(pos) => (&body[..pos], Some(&body[pos + 2..])),
                None => (body, None),
            };
            match env.get(name) {
                Some(v) => out.push_str(v),
                None => match default {
                    Some(d) => out.push_str(d),
                    None => out.push_str(&input[i..=j]),
                },
            }
            i = j + 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn substitute_replaces_named_var() {
        let mut env = BTreeMap::new();
        env.insert("X".into(), "hello".into());
        assert_eq!(substitute("a ${X} b", &env), "a hello b");
    }

    #[test]
    fn substitute_uses_default_when_unset() {
        let env = BTreeMap::new();
        assert_eq!(substitute("a ${X:-d} b", &env), "a d b");
    }

    #[test]
    fn substitute_prefers_env_over_default() {
        let mut env = BTreeMap::new();
        env.insert("X".into(), "real".into());
        assert_eq!(substitute("a ${X:-d} b", &env), "a real b");
    }

    #[test]
    fn substitute_keeps_literal_for_missing_no_default() {
        let env = BTreeMap::new();
        assert_eq!(substitute("a ${MISSING} b", &env), "a ${MISSING} b");
    }

    #[test]
    fn substitute_handles_escape() {
        let env = BTreeMap::new();
        assert_eq!(substitute("a \\${X} b", &env), "a ${X} b");
    }

    #[test]
    fn substitute_handles_unmatched_brace() {
        let env = BTreeMap::new();
        assert_eq!(substitute("a ${unclosed b", &env), "a ${unclosed b");
    }

    #[test]
    fn substitute_multiple_vars() {
        let mut env = BTreeMap::new();
        env.insert("A".into(), "1".into());
        env.insert("B".into(), "2".into());
        assert_eq!(substitute("${A}-${B}", &env), "1-2");
    }

    #[test]
    fn substitute_default_can_be_empty() {
        let env = BTreeMap::new();
        assert_eq!(substitute("a ${X:-} b", &env), "a  b");
    }

    #[test]
    fn substitute_default_can_contain_dashes() {
        let env = BTreeMap::new();
        assert_eq!(substitute("${X:-default-with-dash}", &env), "default-with-dash");
    }

    #[test]
    fn raw_seed_is_valid_json_after_substitution() {
        let env = BTreeMap::new();
        let s = substitute(RAW_REALM_SEED, &env);
        let v: Value = serde_json::from_str(&s).expect("realm seed must parse as JSON");
        assert_eq!(v["realm"], "cave");
    }

    #[test]
    fn rendered_seed_contains_admin_role() {
        let env = BTreeMap::new();
        let s = substitute(RAW_REALM_SEED, &env);
        let v: Value = serde_json::from_str(&s).unwrap();
        let roles = v["roles"]["realm"].as_array().unwrap();
        let names: Vec<&str> = roles.iter().filter_map(|r| r["name"].as_str()).collect();
        assert!(names.contains(&"admin"));
        assert!(names.contains(&"operator"));
        assert!(names.contains(&"tenant"));
    }

    #[test]
    fn rendered_seed_contains_cave_portal_client() {
        let env = BTreeMap::new();
        let s = substitute(RAW_REALM_SEED, &env);
        let v: Value = serde_json::from_str(&s).unwrap();
        let clients = v["clients"].as_array().unwrap();
        let portal = clients
            .iter()
            .find(|c| c["clientId"] == "cave-portal")
            .expect("cave-portal client must exist");
        assert_eq!(portal["protocol"], "openid-connect");
        assert_eq!(portal["standardFlowEnabled"], true);
        assert_eq!(portal["implicitFlowEnabled"], false);
        assert_eq!(portal["publicClient"], false);
        assert_eq!(portal["attributes"]["pkce.code.challenge.method"], "S256");
    }

    #[test]
    fn rendered_seed_redirect_uri_matches_callback() {
        let env = BTreeMap::new();
        let s = substitute(RAW_REALM_SEED, &env);
        let v: Value = serde_json::from_str(&s).unwrap();
        let redirects = v["clients"][0]["redirectUris"].as_array().unwrap();
        let strs: Vec<&str> = redirects.iter().filter_map(|r| r.as_str()).collect();
        assert!(strs.iter().any(|r| r.ends_with("/oauth/callback")),
                "redirect_uri must end with /oauth/callback, got {strs:?}");
    }

    #[test]
    fn rendered_seed_admin_user_has_admin_role() {
        let env = BTreeMap::new();
        let s = substitute(RAW_REALM_SEED, &env);
        let v: Value = serde_json::from_str(&s).unwrap();
        let users = v["users"].as_array().unwrap();
        let admin = users
            .iter()
            .find(|u| u["username"] == "admin")
            .expect("default admin user must exist");
        let roles: Vec<&str> = admin["realmRoles"].as_array().unwrap()
            .iter().filter_map(|r| r.as_str()).collect();
        assert!(roles.contains(&"admin"));
    }

    #[test]
    fn rendered_seed_admin_credential_uses_env_password() {
        let mut env = BTreeMap::new();
        env.insert("CAVE_ADMIN_PASSWORD".into(), "supersecret".into());
        let s = substitute(RAW_REALM_SEED, &env);
        let v: Value = serde_json::from_str(&s).unwrap();
        let creds = v["users"][0]["credentials"].as_array().unwrap();
        assert_eq!(creds[0]["value"], "supersecret");
    }

    #[test]
    fn rendered_seed_admin_username_uses_env() {
        let mut env = BTreeMap::new();
        env.insert("CAVE_ADMIN_USERNAME".into(), "btartan".into());
        let s = substitute(RAW_REALM_SEED, &env);
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["users"][0]["username"], "btartan");
    }

    #[test]
    fn rendered_seed_brute_force_protected() {
        let env = BTreeMap::new();
        let s = substitute(RAW_REALM_SEED, &env);
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["bruteForceProtected"], true);
    }

    #[test]
    fn rendered_seed_pkce_required() {
        let env = BTreeMap::new();
        let s = substitute(RAW_REALM_SEED, &env);
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(
            v["clients"][0]["attributes"]["pkce.code.challenge.method"],
            "S256"
        );
    }

    #[test]
    fn render_seed_from_env_works() {
        // doesn't blow up; result parses
        let s = render_seed_from_env();
        let _v: Value = serde_json::from_str(&s).unwrap();
    }

    #[test]
    fn substitute_idempotent_when_no_placeholders() {
        let env = BTreeMap::new();
        let s = "no placeholders here";
        assert_eq!(substitute(s, &env), s);
    }

    #[test]
    fn substitute_handles_dollar_without_brace() {
        let env = BTreeMap::new();
        assert_eq!(substitute("price $5", &env), "price $5");
    }
}
