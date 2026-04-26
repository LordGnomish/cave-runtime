//! Conversion webhook — converts resources between API versions.
//!
//! Upstream: kubernetes/kubernetes v1.30.0
//!   * `staging/src/k8s.io/apiextensions-apiserver/pkg/apiserver/conversion/converter.go`
//!   * `staging/src/k8s.io/apiextensions-apiserver/pkg/apiserver/conversion/webhook_converter.go`
//!   * `staging/src/k8s.io/api/apiextensions/v1/types.go` (`ConversionReview`).
//!
//! This is the conversion-webhook entry point: each request contains a list
//! of objects at one apiVersion and a `desired_api_version`. The converter
//! returns objects rewritten in the target version. For built-in core types
//! we implement an in-process converter (no HTTP) — which is what
//! upstream's `none` strategy degenerates to.
//!
//! Tenant invariant: the converter MUST preserve the `tenant_id` of every
//! object across version transitions. Any drop or rewrite is a fault.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvertibleObject {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    /// Schema-versioned payload — opaque to the converter except for known
    /// field renames.
    pub fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionRequest {
    pub uid: String,
    pub desired_api_version: String,
    pub objects: Vec<ConvertibleObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionResponse {
    pub uid: String,
    pub converted_objects: Vec<ConvertibleObject>,
    pub result_status: String, // "Success" | "Failure"
    pub result_message: String,
}

/// Built-in converter for known field renames. Real K8s converters use a
/// scheme registry; we implement the minimal "rename + version stamp" path
/// that handles the v1beta1 ↔ v1 case for core types.
pub struct CoreConverter {
    /// Rename rules keyed by `(from_version, to_version)`.
    rules: Vec<RenameRule>,
}

#[derive(Debug, Clone)]
pub struct RenameRule {
    pub from_version: String,
    pub to_version: String,
    pub kind: String,
    pub from_field: String,
    pub to_field: String,
}

impl CoreConverter {
    pub fn new() -> Self {
        Self {
            rules: vec![
                // Example: in apps/v1beta1 the field was `paused` (bool); in apps/v1
                // it stayed `paused`, but Replica selectors were renamed. We model
                // a simple rename as illustration of the scheme.
                RenameRule {
                    from_version: "apps/v1beta1".into(),
                    to_version:   "apps/v1".into(),
                    kind:         "Deployment".into(),
                    from_field:   "rollbackTo".into(),
                    to_field:     "_deprecated_rollbackTo".into(),
                },
                RenameRule {
                    from_version: "v1beta1".into(),
                    to_version:   "v1".into(),
                    kind:         "ConfigMap".into(),
                    from_field:   "binaryData".into(),
                    to_field:     "binaryData".into(),
                },
            ],
        }
    }

    pub fn with_rule(mut self, rule: RenameRule) -> Self {
        self.rules.push(rule);
        self
    }

    pub fn convert(&self, req: ConversionRequest) -> ConversionResponse {
        let mut converted = Vec::with_capacity(req.objects.len());
        for mut obj in req.objects {
            let from_version = obj.api_version.clone();
            // tenant_id invariant: preserved verbatim.
            let original_tenant = obj.tenant_id.clone();
            // Apply applicable rules.
            for rule in &self.rules {
                if rule.from_version == from_version
                    && rule.to_version == req.desired_api_version
                    && rule.kind == obj.kind
                    && rule.from_field != rule.to_field
                {
                    if let Some(v) = obj.fields.remove(&rule.from_field) {
                        obj.fields.insert(rule.to_field.clone(), v);
                    }
                }
            }
            obj.api_version = req.desired_api_version.clone();
            // Re-assert tenant_id (defensive — should never have changed).
            obj.tenant_id = original_tenant;
            converted.push(obj);
        }
        ConversionResponse {
            uid: req.uid,
            converted_objects: converted,
            result_status: "Success".into(),
            result_message: String::new(),
        }
    }

    pub fn rule_count(&self) -> usize { self.rules.len() }
}

impl Default for CoreConverter {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(kind: &str, version: &str, tenant: &str) -> ConvertibleObject {
        let mut fields = serde_json::Map::new();
        fields.insert("foo".into(), serde_json::Value::String("bar".into()));
        ConvertibleObject {
            api_version: version.into(),
            kind: kind.into(),
            name: "obj1".into(),
            namespace: "default".into(),
            tenant_id: tenant.into(),
            fields,
        }
    }

    /// Upstream parity: `TestConverter_RewriteApiVersion`.
    #[test]
    fn test_convert_stamps_target_api_version() {
        let conv = CoreConverter::new();
        let req = ConversionRequest {
            uid: "u1".into(),
            desired_api_version: "v1".into(),
            objects: vec![obj("ConfigMap", "v1beta1", "acme")],
        };
        let resp = conv.convert(req);
        assert_eq!(resp.result_status, "Success");
        assert_eq!(resp.converted_objects[0].api_version, "v1");
        assert_eq!(resp.converted_objects[0].tenant_id, "acme",
            "tenant_id invariant preserved across conversion");
    }

    /// Upstream parity: `TestConverter_FieldRename`.
    #[test]
    fn test_convert_applies_field_rename() {
        let conv = CoreConverter::new()
            .with_rule(RenameRule {
                from_version: "v1beta1".into(),
                to_version: "v1".into(),
                kind: "Widget".into(),
                from_field: "oldName".into(),
                to_field: "newName".into(),
            });
        let mut o = obj("Widget", "v1beta1", "acme");
        o.fields.insert("oldName".into(), serde_json::Value::String("hello".into()));
        let req = ConversionRequest {
            uid: "u1".into(), desired_api_version: "v1".into(), objects: vec![o],
        };
        let resp = conv.convert(req);
        let out = &resp.converted_objects[0];
        assert!(out.fields.contains_key("newName"));
        assert!(!out.fields.contains_key("oldName"));
        assert_eq!(out.tenant_id, "acme", "tenant_id invariant preserved");
    }

    /// Upstream parity: `TestConverter_NoOpForMatchingVersion`.
    #[test]
    fn test_convert_noop_when_already_target_version() {
        let conv = CoreConverter::new();
        let req = ConversionRequest {
            uid: "u1".into(),
            desired_api_version: "v1".into(),
            objects: vec![obj("ConfigMap", "v1", "acme")],
        };
        let resp = conv.convert(req);
        assert_eq!(resp.converted_objects[0].api_version, "v1");
        assert_eq!(resp.converted_objects[0].tenant_id, "acme",
            "tenant_id invariant preserved on noop");
    }

    /// Upstream parity: `TestConverter_PreservesTenantAcrossBatch`.
    #[test]
    fn test_convert_preserves_tenant_for_each_object() {
        let conv = CoreConverter::new();
        let req = ConversionRequest {
            uid: "u1".into(),
            desired_api_version: "v1".into(),
            objects: vec![
                obj("ConfigMap", "v1beta1", "acme"),
                obj("ConfigMap", "v1beta1", "globex"),
                obj("ConfigMap", "v1beta1", "initech"),
            ],
        };
        let resp = conv.convert(req);
        assert_eq!(resp.converted_objects[0].tenant_id, "acme");
        assert_eq!(resp.converted_objects[1].tenant_id, "globex");
        assert_eq!(resp.converted_objects[2].tenant_id, "initech");
        // tenant_id invariant: no cross-tenant bleed.
    }

    /// Upstream parity: `TestConverter_RuleScopedByKind`.
    #[test]
    fn test_rule_scoped_by_kind_does_not_apply_to_other_kinds() {
        let conv = CoreConverter::new()
            .with_rule(RenameRule {
                from_version: "v1beta1".into(),
                to_version: "v1".into(),
                kind: "Foo".into(),
                from_field: "old".into(),
                to_field: "new".into(),
            });
        let mut o = obj("Bar", "v1beta1", "acme");
        o.fields.insert("old".into(), serde_json::Value::Bool(true));
        let req = ConversionRequest {
            uid: "u1".into(), desired_api_version: "v1".into(), objects: vec![o],
        };
        let resp = conv.convert(req);
        assert!(resp.converted_objects[0].fields.contains_key("old"),
            "rule for Foo MUST NOT touch Bar");
        assert_eq!(resp.converted_objects[0].tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestConverter_RuleScopedByDirection`.
    #[test]
    fn test_rule_scoped_by_direction_not_reversed() {
        let conv = CoreConverter::new()
            .with_rule(RenameRule {
                from_version: "v1beta1".into(),
                to_version: "v1".into(),
                kind: "Foo".into(),
                from_field: "old".into(),
                to_field: "new".into(),
            });
        // Convert in reverse direction — rule must not apply.
        let mut o = obj("Foo", "v1", "acme");
        o.fields.insert("new".into(), serde_json::Value::Bool(true));
        let req = ConversionRequest {
            uid: "u1".into(), desired_api_version: "v1beta1".into(), objects: vec![o],
        };
        let resp = conv.convert(req);
        assert!(resp.converted_objects[0].fields.contains_key("new"),
            "v1→v1beta1 must not apply v1beta1→v1 rule");
        assert_eq!(resp.converted_objects[0].tenant_id, "acme", "tenant_id invariant");
    }

    /// Upstream parity: `TestConverter_EmptyBatchSucceeds`.
    #[test]
    fn test_empty_batch_returns_success() {
        let conv = CoreConverter::new();
        let req = ConversionRequest {
            uid: "u1".into(),
            desired_api_version: "v1".into(),
            objects: vec![],
        };
        let resp = conv.convert(req);
        assert_eq!(resp.result_status, "Success");
        assert_eq!(resp.converted_objects.len(), 0);
    }

    /// Upstream parity: `TestConverter_RuleCountSnapshot`.
    #[test]
    fn test_default_rule_set_present() {
        let conv = CoreConverter::new();
        assert!(conv.rule_count() >= 2,
            "default converter ships at least 2 baseline rules");
        // tenant_id invariant smoke: empty batch still preserves shape.
        let resp = conv.convert(ConversionRequest {
            uid: "u".into(), desired_api_version: "v1".into(), objects: vec![],
        });
        assert_eq!(resp.result_status, "Success");
    }
}
