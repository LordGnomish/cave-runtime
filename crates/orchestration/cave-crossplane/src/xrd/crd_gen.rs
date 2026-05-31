// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! XRD → CRD generation.
//!
//! Upstream: crossplane/crossplane-runtime pkg/xcrd
//!   - ForCompositeResource / ForCompositeResourceClaim (crd.go)
//!   - CompositeResourceSpecProps / CompositeResourceStatusProps (schemas.go)
//!
//! An XRD (`CompositeResourceDefinition`) is rendered into a Kubernetes
//! `CustomResourceDefinition`. The XR CRD takes the XRD's `names` + `group` +
//! `scope`, appends the `composite` category, and injects the Crossplane
//! *system fields* into every version's `openAPIV3Schema`:
//!   * spec (all scopes): `compositionRef`, `compositionSelector`,
//!     `compositionRevisionRef`, `compositionRevisionSelector`,
//!     `compositionUpdatePolicy`, plus `resourceRefs`,
//!   * spec (cluster scope only): `claimRef`, `writeConnectionSecretToRef`,
//!   * status (all): `conditions`,
//!   * status (cluster scope only): `connectionDetails.lastPublishedTime`,
//!     `claimConditionTypes`.
//!
//! The claim CRD ([`for_composite_resource_claim`]) is generated from the XRD's
//! `claimNames`, is always namespaced, and appends the `claim` category.
//!
//! `XrdScope::Cluster` is treated as the legacy-cluster shape (the one that
//! offers claims, hence `claimRef`/`connectionDetails`); `XrdScope::Namespaced`
//! is the modern namespaced XR (no claim plumbing). This is a pure in-crate
//! JSON transform — no apiserver coupling.

use crate::models::XrdScope;
use crate::xrd::spec::XrdSpec;
use serde_json::{json, Map, Value};

/// Category appended to a composite resource CRD's `names.categories`.
pub const CATEGORY_COMPOSITE: &str = "composite";
/// Category appended to a claim CRD's `names.categories`.
pub const CATEGORY_CLAIM: &str = "claim";

/// Composition-selection system fields injected into every XR/claim spec
/// (upstream `CompositeResourceSpecProps`, all scopes).
pub const COMPOSITION_SPEC_FIELDS: &[&str] = &[
    "compositionRef",
    "compositionSelector",
    "compositionRevisionRef",
    "compositionRevisionSelector",
    "compositionUpdatePolicy",
];

fn object_ref_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "name": {"type": "string"} }
    })
}

fn label_selector_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "matchLabels": {
                "type": "object",
                "additionalProperties": {"type": "string"}
            }
        }
    })
}

/// The composition-selection spec properties common to every scope.
fn composition_spec_props(out: &mut Map<String, Value>) {
    out.insert("compositionRef".into(), object_ref_schema());
    out.insert("compositionSelector".into(), label_selector_schema());
    out.insert("compositionRevisionRef".into(), object_ref_schema());
    out.insert("compositionRevisionSelector".into(), label_selector_schema());
    out.insert(
        "compositionUpdatePolicy".into(),
        json!({"type": "string", "enum": ["Automatic", "Manual"]}),
    );
    out.insert(
        "resourceRefs".into(),
        json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "apiVersion": {"type": "string"},
                    "kind": {"type": "string"},
                    "name": {"type": "string"}
                }
            }
        }),
    );
}

/// Cluster-scope (claim-offering) extra spec properties.
fn cluster_spec_props(out: &mut Map<String, Value>) {
    out.insert("claimRef".into(), object_ref_schema());
    out.insert(
        "writeConnectionSecretToRef".into(),
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "namespace": {"type": "string"}
            }
        }),
    );
}

/// Status `conditions` array, shared by every scope.
fn conditions_schema() -> Value {
    json!({
        "type": "array",
        "items": {
            "type": "object",
            "required": ["lastTransitionTime", "reason", "status", "type"],
            "properties": {
                "lastTransitionTime": {"type": "string", "format": "date-time"},
                "message": {"type": "string"},
                "reason": {"type": "string"},
                "status": {"type": "string"},
                "type": {"type": "string"},
                "observedGeneration": {"type": "integer", "format": "int64"}
            }
        }
    })
}

/// Cluster-scope status extras: `connectionDetails` + `claimConditionTypes`.
fn cluster_status_props(out: &mut Map<String, Value>) {
    out.insert(
        "connectionDetails".into(),
        json!({
            "type": "object",
            "properties": {
                "lastPublishedTime": {"type": "string", "format": "date-time"}
            }
        }),
    );
    out.insert(
        "claimConditionTypes".into(),
        json!({"type": "array", "items": {"type": "string"}}),
    );
}

/// Deep-merge the system fields into a single version's user schema, returning
/// the full `openAPIV3Schema` for the generated CRD version.
fn inject_system_fields(user_schema: Option<&Value>, scope: XrdScope, is_claim: bool) -> Value {
    // Start from the user-authored openAPIV3Schema (or a minimal object).
    let mut root = match user_schema {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    root.entry("type").or_insert_with(|| json!("object"));

    // properties.{spec,status} — preserve any user-authored children.
    let props = root
        .entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("properties is an object");

    // --- spec ---
    let spec = props
        .entry("spec")
        .or_insert_with(|| json!({"type": "object"}))
        .as_object_mut()
        .expect("spec is an object");
    spec.entry("type").or_insert_with(|| json!("object"));
    let spec_props = spec
        .entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("spec.properties is an object");
    composition_spec_props(spec_props);
    if is_claim {
        // Claims bind to a single XR + may publish a connection secret.
        spec_props.insert("resourceRef".into(), object_ref_schema());
        spec_props.insert(
            "writeConnectionSecretToRef".into(),
            json!({
                "type": "object",
                "properties": { "name": {"type": "string"} }
            }),
        );
    } else if scope == XrdScope::Cluster {
        cluster_spec_props(spec_props);
    }

    // --- status ---
    let status = props
        .entry("status")
        .or_insert_with(|| json!({"type": "object"}))
        .as_object_mut()
        .expect("status is an object");
    status.entry("type").or_insert_with(|| json!("object"));
    let status_props = status
        .entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("status.properties is an object");
    status_props.insert("conditions".into(), conditions_schema());
    if is_claim || scope == XrdScope::Cluster {
        cluster_status_props(status_props);
    }

    Value::Object(root)
}

fn render_versions(xrd: &XrdSpec, scope: XrdScope, is_claim: bool) -> Vec<Value> {
    xrd.versions
        .iter()
        .map(|v| {
            json!({
                "name": v.name,
                "served": v.served,
                // CRD storage version mirrors the XRD's referenceable version.
                "storage": v.referenceable,
                "schema": {
                    "openAPIV3Schema": inject_system_fields(v.schema.as_ref(), scope, is_claim)
                }
            })
        })
        .collect()
}

fn scope_str(scope: XrdScope) -> &'static str {
    match scope {
        XrdScope::Cluster => "Cluster",
        XrdScope::Namespaced => "Namespaced",
    }
}

/// Render the composite resource (XR) CRD from an XRD
/// (upstream `xcrd.ForCompositeResource`).
pub fn for_composite_resource(xrd: &XrdSpec) -> Value {
    let n = &xrd.names;
    let mut categories = vec![CATEGORY_COMPOSITE.to_string()];
    // Preserve any user-declared categories (none in our model today), then
    // ensure `composite` is present.
    categories.dedup();
    json!({
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": { "name": format!("{}.{}", n.plural, xrd.group) },
        "spec": {
            "group": xrd.group,
            "scope": scope_str(xrd.scope),
            "names": {
                "kind": n.kind,
                "listKind": n.list_kind,
                "plural": n.plural,
                "singular": n.singular,
                "categories": categories,
            },
            "versions": render_versions(xrd, xrd.scope, false),
        }
    })
}

/// Render the claim CRD from an XRD's `claimNames`
/// (upstream `xcrd.ForCompositeResourceClaim`). Returns `None` when the XRD
/// offers no claim.
pub fn for_composite_resource_claim(xrd: &XrdSpec) -> Option<Value> {
    let cn = xrd.claim_names.as_ref()?;
    Some(json!({
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": { "name": format!("{}.{}", cn.plural, xrd.group) },
        "spec": {
            "group": xrd.group,
            // Claims are always namespaced.
            "scope": "Namespaced",
            "names": {
                "kind": cn.kind,
                "listKind": cn.list_kind,
                "plural": cn.plural,
                "singular": cn.singular,
                "categories": [CATEGORY_CLAIM],
            },
            // Claims use the XR's versions, with claim-shaped system fields.
            "versions": render_versions(xrd, XrdScope::Namespaced, true),
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xrd::spec::XrdSpecVersion;

    fn xrd() -> XrdSpec {
        let mut s = XrdSpec::new("ex.io", "XThing", XrdScope::Cluster);
        s.versions = vec![XrdSpecVersion {
            name: "v1".into(),
            served: true,
            referenceable: true,
            schema: None,
        }];
        s
    }

    #[test]
    fn minimal_schema_when_user_schema_absent() {
        let crd = for_composite_resource(&xrd());
        let oas = &crd["spec"]["versions"][0]["schema"]["openAPIV3Schema"];
        assert_eq!(oas["type"], json!("object"));
        assert!(oas["properties"]["spec"]["properties"]["compositionRef"].is_object());
    }

    #[test]
    fn storage_mirrors_referenceable() {
        let crd = for_composite_resource(&xrd());
        assert_eq!(crd["spec"]["versions"][0]["storage"], json!(true));
    }
}
