// SPDX-License-Identifier: AGPL-3.0-or-later
//! kubectl output-format mapping (`-o yaml`/`-o json`/`-o name`/`-o wide`).
//!
//! Translates the kubectl `-o` flag values into Cave's canonical
//! `OutputFormat` enum. The native renderer covers the same shapes
//! plus an extra `Table` default — kubectl's default is `Wide` so we
//! map accordingly.

use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KubectlOutput {
    /// Wide table — kubectl default (no `-o`).
    Wide,
    /// `-o json`
    Json,
    /// `-o yaml`
    Yaml,
    /// `-o name`
    Name,
    /// `-o jsonpath=...` (raw expression carried through to server).
    Jsonpath(String),
    /// `-o custom-columns=KEY:.path[,KEY:.path]`
    CustomColumns(String),
    /// `-o go-template=...`
    GoTemplate(String),
}

pub fn parse(s: Option<&str>) -> Result<KubectlOutput> {
    match s {
        None => Ok(KubectlOutput::Wide),
        Some(raw) => {
            let lower = raw.to_lowercase();
            if let Some(rest) = lower.strip_prefix("jsonpath=") {
                let _ = rest;
                let payload = raw["jsonpath=".len()..].to_string();
                if payload.is_empty() {
                    bail!("empty jsonpath expression");
                }
                return Ok(KubectlOutput::Jsonpath(payload));
            }
            if let Some(rest) = lower.strip_prefix("custom-columns=") {
                let _ = rest;
                let payload = raw["custom-columns=".len()..].to_string();
                if payload.is_empty() {
                    bail!("empty custom-columns spec");
                }
                return Ok(KubectlOutput::CustomColumns(payload));
            }
            if let Some(rest) = lower.strip_prefix("go-template=") {
                let _ = rest;
                let payload = raw["go-template=".len()..].to_string();
                if payload.is_empty() {
                    bail!("empty go-template expression");
                }
                return Ok(KubectlOutput::GoTemplate(payload));
            }
            match lower.as_str() {
                "wide" => Ok(KubectlOutput::Wide),
                "json" => Ok(KubectlOutput::Json),
                "yaml" => Ok(KubectlOutput::Yaml),
                "name" => Ok(KubectlOutput::Name),
                _ => bail!("unknown -o value `{}`", raw),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_flag_is_wide() {
        assert_eq!(parse(None).unwrap(), KubectlOutput::Wide);
    }

    #[test]
    fn json() {
        assert_eq!(parse(Some("json")).unwrap(), KubectlOutput::Json);
    }

    #[test]
    fn yaml() {
        assert_eq!(parse(Some("yaml")).unwrap(), KubectlOutput::Yaml);
    }

    #[test]
    fn name() {
        assert_eq!(parse(Some("name")).unwrap(), KubectlOutput::Name);
    }

    #[test]
    fn wide_explicit() {
        assert_eq!(parse(Some("wide")).unwrap(), KubectlOutput::Wide);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(parse(Some("JSON")).unwrap(), KubectlOutput::Json);
        assert_eq!(parse(Some("Yaml")).unwrap(), KubectlOutput::Yaml);
    }

    #[test]
    fn jsonpath_expression() {
        let out = parse(Some("jsonpath={.items[0].metadata.name}")).unwrap();
        assert_eq!(
            out,
            KubectlOutput::Jsonpath("{.items[0].metadata.name}".into())
        );
    }

    #[test]
    fn jsonpath_empty_rejected() {
        assert!(parse(Some("jsonpath=")).is_err());
    }

    #[test]
    fn custom_columns_expression() {
        let out = parse(Some("custom-columns=NAME:.metadata.name,STATUS:.status.phase")).unwrap();
        match out {
            KubectlOutput::CustomColumns(s) => assert!(s.contains("NAME")),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn custom_columns_empty_rejected() {
        assert!(parse(Some("custom-columns=")).is_err());
    }

    #[test]
    fn go_template_expression() {
        let out = parse(Some("go-template={{.metadata.name}}")).unwrap();
        match out {
            KubectlOutput::GoTemplate(s) => assert!(s.contains("metadata.name")),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn go_template_empty_rejected() {
        assert!(parse(Some("go-template=")).is_err());
    }

    #[test]
    fn unknown_rejected() {
        assert!(parse(Some("xml")).is_err());
    }
}
