// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/kerberos/src/main/java/org/keycloak/federation/kerberos/impl/KerberosUsernamePasswordAuthenticator.java
// Source: RFC 4120 §5.2.2  PrincipalName
//
// Kerberos principal type — `service/host@REALM`.  The realm is
// part of the principal in MIT format; we keep them separated so
// downstream code can match against `FederationConfig.kerberos_realm`
// without re-parsing.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// `["HTTP", "portal.acme.corp"]` for `HTTP/portal.acme.corp`.
    pub components: Vec<String>,
    /// `ACME.CORP`.
    pub realm: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PrincipalError {
    #[error("principal is empty")]
    Empty,
    #[error("missing realm in `{0}` — expected name@REALM")]
    MissingRealm(String),
    #[error("unbalanced escape in `{0}`")]
    BadEscape(String),
}

impl Principal {
    /// Parse the MIT-style flat form.  Escapes: `\\@`, `\\/`, `\\\\`.
    pub fn parse(s: &str) -> Result<Self, PrincipalError> {
        if s.is_empty() {
            return Err(PrincipalError::Empty);
        }
        let mut components: Vec<String> = vec![String::new()];
        let mut realm = String::new();
        let mut in_realm = false;
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, in_realm) {
                ('\\', _) => {
                    let next = chars.next().ok_or_else(|| PrincipalError::BadEscape(s.to_string()))?;
                    if in_realm {
                        realm.push(next);
                    } else {
                        components.last_mut().unwrap().push(next);
                    }
                }
                ('/', false) => components.push(String::new()),
                ('@', false) => {
                    in_realm = true;
                }
                (c, false) => components.last_mut().unwrap().push(c),
                (c, true) => realm.push(c),
            }
        }
        if !in_realm {
            return Err(PrincipalError::MissingRealm(s.to_string()));
        }
        Ok(Principal { components, realm })
    }

    /// Serialize back to MIT form.
    pub fn to_string_mit(&self) -> String {
        let joined: Vec<String> = self
            .components
            .iter()
            .map(|c| {
                c.chars()
                    .map(|ch| match ch {
                        '@' | '/' | '\\' => format!("\\{ch}"),
                        _ => ch.to_string(),
                    })
                    .collect()
            })
            .collect();
        format!("{}@{}", joined.join("/"), self.realm)
    }

    /// True if this is a service principal (`service/host@REALM`).
    pub fn is_service(&self) -> bool {
        self.components.len() >= 2
    }

    /// Convenience: the typical SPN form, `HTTP/portal.acme.corp`.
    pub fn spn(&self) -> Option<String> {
        if !self.is_service() {
            return None;
        }
        Some(self.components.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_user_principal() {
        let p = Principal::parse("alice@ACME.CORP").unwrap();
        assert_eq!(p.components, vec!["alice"]);
        assert_eq!(p.realm, "ACME.CORP");
        assert!(!p.is_service());
    }

    #[test]
    fn parse_service_principal() {
        let p = Principal::parse("HTTP/portal.acme.corp@ACME.CORP").unwrap();
        assert_eq!(p.components, vec!["HTTP", "portal.acme.corp"]);
        assert_eq!(p.spn().as_deref(), Some("HTTP/portal.acme.corp"));
    }

    #[test]
    fn parse_rejects_missing_realm() {
        assert!(matches!(Principal::parse("alice"), Err(PrincipalError::MissingRealm(_))));
    }

    #[test]
    fn parse_handles_escape() {
        let p = Principal::parse(r"alice\@example@ACME.CORP").unwrap();
        assert_eq!(p.components, vec!["alice@example"]);
    }

    #[test]
    fn to_string_round_trip_escapes_special() {
        let p = Principal { components: vec!["with/slash".into()], realm: "ACME".into() };
        let s = p.to_string_mit();
        let p2 = Principal::parse(&s).unwrap();
        assert_eq!(p2.components, p.components);
    }

    #[test]
    fn empty_rejected() {
        assert_eq!(Principal::parse(""), Err(PrincipalError::Empty));
    }

    #[test]
    fn unbalanced_escape_rejected() {
        assert!(matches!(Principal::parse("alice\\"), Err(PrincipalError::BadEscape(_))));
    }
}
