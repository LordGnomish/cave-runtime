// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl infra {xr,xrd,composition,provider,function,package,claim}` dispatcher.
//!
//! This module exposes a typed dispatcher so cave-cli can wire it without
//! pulling in clap features. The dispatcher returns a JSON Value as the
//! command result; cave-cli formats it for human/JSON output.

use crate::CrossplaneState;
use crate::error::{CrossplaneError, CrossplaneResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum InfraSubcommand {
    Xr,
    Xrd,
    Composition,
    Provider,
    Function,
    Package,
    Claim,
}

impl InfraSubcommand {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "xr" => Some(Self::Xr),
            "xrd" => Some(Self::Xrd),
            "composition" | "comp" => Some(Self::Composition),
            "provider" => Some(Self::Provider),
            "function" => Some(Self::Function),
            "package" | "pkg" => Some(Self::Package),
            "claim" => Some(Self::Claim),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum InfraAction {
    List,
    Get(String),
    Health,
    Catalog,
    /// Garbage-collect revisions: `gc <name> [revisionHistoryLimit]`.
    Gc(String, Option<i64>),
}

impl InfraAction {
    pub fn from_args(args: &[String]) -> CrossplaneResult<Self> {
        match args.first().map(|s| s.as_str()) {
            Some("list") | None => Ok(Self::List),
            Some("get") => {
                let name = args.get(1).cloned().ok_or_else(|| {
                    CrossplaneError::Internal("get requires a name".to_string())
                })?;
                Ok(Self::Get(name))
            }
            Some("gc") => {
                let name = args.get(1).cloned().ok_or_else(|| {
                    CrossplaneError::Internal("gc requires a name".to_string())
                })?;
                let limit = match args.get(2) {
                    Some(l) => Some(l.parse::<i64>().map_err(|_| {
                        CrossplaneError::Internal(format!("gc limit must be an integer: {}", l))
                    })?),
                    None => None,
                };
                Ok(Self::Gc(name, limit))
            }
            Some("health") => Ok(Self::Health),
            Some("catalog") => Ok(Self::Catalog),
            Some(other) => Err(CrossplaneError::Internal(format!(
                "unknown action: {}",
                other
            ))),
        }
    }
}

/// Dispatch `cavectl infra <subcommand> <action> [name]` to the live state.
pub fn dispatch(
    state: &Arc<CrossplaneState>,
    sub: InfraSubcommand,
    action: InfraAction,
) -> CrossplaneResult<Value> {
    match (sub, action) {
        (InfraSubcommand::Xrd, InfraAction::List) => Ok(json!({
            "items": state.xrd_store.list().iter().map(|x| json!({
                "name": x.name, "group": x.group, "kind": x.kind,
            })).collect::<Vec<_>>(),
        })),
        (InfraSubcommand::Xrd, InfraAction::Get(name)) => {
            let xrd = state.xrd_store.get_by_name(&name)?;
            Ok(serde_json::to_value(xrd).unwrap_or(Value::Null))
        }
        (InfraSubcommand::Composition, InfraAction::List) => Ok(json!({
            "items": state.composition_store.list().iter().map(|c| json!({
                "name": c.name,
                "kind": c.composite_type_ref.kind,
            })).collect::<Vec<_>>(),
        })),
        (InfraSubcommand::Composition, InfraAction::Get(name)) => {
            let c = state.composition_store.get(&name)?;
            Ok(serde_json::to_value(c).unwrap_or(Value::Null))
        }
        (InfraSubcommand::Composition, InfraAction::Gc(name, limit)) => {
            let collected = state.composition_store.gc_revisions(&name, limit)?;
            Ok(json!({
                "name": name,
                "limit": limit,
                "collected": collected,
                "remaining": state
                    .composition_store
                    .get_revisions(&name)
                    .map(|r| r.len())
                    .unwrap_or(0),
            }))
        }
        (InfraSubcommand::Provider, InfraAction::List) => Ok(json!({
            "items": state.provider_store.list().iter().map(|p| json!({
                "name": p.name,
                "package": p.package,
                "status": format!("{:?}", p.status),
            })).collect::<Vec<_>>(),
        })),
        (InfraSubcommand::Provider, InfraAction::Catalog) => Ok(json!({
            "items": state.provider_store.catalog(),
        })),
        (InfraSubcommand::Provider, InfraAction::Get(name)) => {
            let p = state.provider_store.get(&name)?;
            Ok(serde_json::to_value(p).unwrap_or(Value::Null))
        }
        (InfraSubcommand::Function, InfraAction::List) => Ok(json!({
            "items": state.function_store.list(),
        })),
        (InfraSubcommand::Function, InfraAction::Get(name)) => {
            let f = state
                .function_store
                .get(&name)
                .ok_or_else(|| CrossplaneError::Internal(format!("function not found: {}", name)))?;
            Ok(serde_json::to_value(f).unwrap_or(Value::Null))
        }
        (InfraSubcommand::Xr, InfraAction::List) => Ok(json!({
            "items": state.claim_store.list_composites(),
        })),
        (InfraSubcommand::Xr, InfraAction::Get(_)) => Err(CrossplaneError::Internal(
            "xr get requires kind/name; use HTTP API for now".into(),
        )),
        (InfraSubcommand::Claim, InfraAction::List) => Ok(json!({
            "note": "claim list requires a namespace; use /api/crossplane/namespaces/{ns}/claims/{kind}",
        })),
        (InfraSubcommand::Claim, InfraAction::Get(_)) => Err(CrossplaneError::Internal(
            "claim get requires ns/name/kind; use HTTP API for now".into(),
        )),
        (InfraSubcommand::Package, InfraAction::List) => Ok(json!({
            "note": "Package list is in-memory; use install_package to register",
        })),
        (InfraSubcommand::Package, InfraAction::Get(_name)) => Ok(json!({
            "note": "Package introspection not yet wired in cavectl",
        })),
        (_, InfraAction::Health) => Ok(json!({
            "module": "cave-crossplane",
            "status": "ok",
            "xrds": state.xrd_store.len(),
            "compositions": state.composition_store.len(),
            "providers": state.provider_store.list().len(),
            "functions": state.function_store.len(),
        })),
        (sub, InfraAction::Catalog) => Err(CrossplaneError::Internal(format!(
            "catalog action is only valid for `provider`; got `{:?}`",
            sub
        ))),
        (sub, InfraAction::Gc(..)) => Err(CrossplaneError::Internal(format!(
            "gc action is only valid for `composition`; got `{:?}`",
            sub
        ))),
    }
}

/// Top-level entrypoint for `cavectl infra ...`.
pub fn run_cli(
    state: &Arc<CrossplaneState>,
    args: &[String],
) -> CrossplaneResult<Value> {
    let sub_str = args
        .first()
        .ok_or_else(|| CrossplaneError::Internal("usage: cavectl infra <sub> [action] [name]".into()))?;
    let sub = InfraSubcommand::from_str(sub_str)
        .ok_or_else(|| CrossplaneError::Internal(format!("unknown subcommand: {}", sub_str)))?;
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();
    let action = InfraAction::from_args(&rest)?;
    dispatch(state, sub, action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Arc<CrossplaneState> {
        Arc::new(CrossplaneState::default())
    }

    #[test]
    fn subcommand_parse() {
        assert_eq!(
            InfraSubcommand::from_str("xrd").unwrap(),
            InfraSubcommand::Xrd
        );
        assert_eq!(
            InfraSubcommand::from_str("comp").unwrap(),
            InfraSubcommand::Composition
        );
        assert_eq!(
            InfraSubcommand::from_str("pkg").unwrap(),
            InfraSubcommand::Package
        );
        assert!(InfraSubcommand::from_str("nope").is_none());
    }

    #[test]
    fn action_from_args_default_list() {
        assert!(matches!(
            InfraAction::from_args(&[]).unwrap(),
            InfraAction::List
        ));
    }

    #[test]
    fn action_get_requires_name() {
        assert!(InfraAction::from_args(&["get".into()]).is_err());
    }

    #[test]
    fn action_get_parses_name() {
        let a = InfraAction::from_args(&["get".into(), "n".into()]).unwrap();
        assert_eq!(a, InfraAction::Get("n".into()));
    }

    #[test]
    fn dispatch_provider_catalog() {
        let v = dispatch(&s(), InfraSubcommand::Provider, InfraAction::Catalog).unwrap();
        assert!(v["items"].is_array());
    }

    #[test]
    fn dispatch_function_list_empty() {
        let v = dispatch(&s(), InfraSubcommand::Function, InfraAction::List).unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn dispatch_xrd_get_missing_errors() {
        let r = dispatch(
            &s(),
            InfraSubcommand::Xrd,
            InfraAction::Get("nope".into()),
        );
        assert!(r.is_err());
    }

    #[test]
    fn run_cli_health() {
        let v = run_cli(&s(), &["xrd".into(), "health".into()]).unwrap();
        assert_eq!(v["module"], json!("cave-crossplane"));
    }

    #[test]
    fn run_cli_no_args_errors() {
        assert!(run_cli(&s(), &[]).is_err());
    }

    #[test]
    fn action_gc_parses_name_and_limit() {
        let a = InfraAction::from_args(&["gc".into(), "c1".into(), "3".into()]).unwrap();
        assert_eq!(a, InfraAction::Gc("c1".into(), Some(3)));
        let a2 = InfraAction::from_args(&["gc".into(), "c1".into()]).unwrap();
        assert_eq!(a2, InfraAction::Gc("c1".into(), None));
    }

    #[test]
    fn action_gc_requires_name() {
        assert!(InfraAction::from_args(&["gc".into()]).is_err());
    }

    #[test]
    fn action_gc_rejects_bad_limit() {
        assert!(InfraAction::from_args(&["gc".into(), "c1".into(), "x".into()]).is_err());
    }

    #[test]
    fn dispatch_composition_gc_missing_errors() {
        let r = dispatch(
            &s(),
            InfraSubcommand::Composition,
            InfraAction::Gc("nope".into(), Some(1)),
        );
        assert!(r.is_err());
    }

    #[test]
    fn dispatch_gc_only_valid_for_composition() {
        let r = dispatch(
            &s(),
            InfraSubcommand::Provider,
            InfraAction::Gc("p".into(), None),
        );
        assert!(r.is_err());
    }

    #[test]
    fn run_cli_unknown_subcommand_errors() {
        assert!(run_cli(&s(), &["bogus".into()]).is_err());
    }
}
