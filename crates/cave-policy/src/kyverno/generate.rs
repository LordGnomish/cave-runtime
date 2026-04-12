//! Kyverno generation engine.
//!
//! Supports: create resources from data, clone, sync, cloneList.

use super::jmespath::substitute_variables_json;
use super::models::*;
use crate::error::PolicyError;
use serde_json::Value;

/// Evaluate a Kyverno generate rule and return the resources to create.
pub fn generate_rule(
    rule: &KyvernoRule,
    trigger_resource: &Value,
    context: &Value,
) -> Result<Vec<GeneratedResource>, PolicyError> {
    let generate = match &rule.generate {
        Some(g) => g,
        None => return Ok(vec![]),
    };

    // Evaluate preconditions
    if let Some(preconditions) = &rule.preconditions {
        if !super::validate::eval_conditions(preconditions, trigger_resource, context)? {
            return Ok(vec![]);
        }
    }

    let resource = build_generated_resource(generate, trigger_resource, context, &rule.name)?;
    Ok(vec![GeneratedResource {
        policy: String::new(),
        rule: rule.name.clone(),
        resource,
    }])
}

fn build_generated_resource(
    generate: &Generation,
    trigger: &Value,
    context: &Value,
    rule_name: &str,
) -> Result<Value, PolicyError> {
    let mut resource = serde_json::json!({
        "apiVersion": generate.api_version,
        "kind": generate.kind,
        "metadata": {
            "name": substitute_name(&generate.name, trigger, context)?,
        }
    });

    // Set namespace if provided
    if let Some(ns) = &generate.namespace {
        let substituted_ns = super::jmespath::substitute_variables(ns, context)?;
        resource["metadata"]["namespace"] = serde_json::json!(substituted_ns);
    }

    if let Some(data) = &generate.data {
        // Substitute variables in the data template
        let substituted = substitute_variables_json(data, context)?;
        merge_into(&mut resource, &substituted);
    } else if let Some(clone) = &generate.clone {
        // Clone from existing resource (reference only — actual clone happens at admission)
        resource["metadata"]["annotations"] = serde_json::json!({
            "kyverno.io/clone-source-namespace": clone.namespace,
            "kyverno.io/clone-source-name": clone.name,
        });
    } else if let Some(clone_list) = &generate.clone_list {
        tracing::debug!(
            target: "kyverno.generate",
            rule = rule_name,
            clone_namespace = clone_list.namespace,
            "cloneList generation (requires API access)"
        );
    }

    // Add synchronize annotation
    if generate.synchronize {
        let annotations = resource["metadata"]["annotations"].as_object_mut();
        if let Some(a) = annotations {
            a.insert("policies.kyverno.io/sync".into(), serde_json::json!("true"));
        } else {
            resource["metadata"]["annotations"] = serde_json::json!({
                "policies.kyverno.io/sync": "true"
            });
        }
    }

    Ok(resource)
}

fn substitute_name(name: &str, trigger: &Value, context: &Value) -> Result<String, PolicyError> {
    // Support {{request.object.metadata.name}} in name field
    super::jmespath::substitute_variables(name, context)
}

fn merge_into(target: &mut Value, source: &Value) {
    if let (Value::Object(t), Value::Object(s)) = (target, source) {
        for (k, v) in s {
            match (t.get_mut(k), v) {
                (Some(tv @ Value::Object(_)), v @ Value::Object(_)) => merge_into(tv, v),
                (Some(entry), v) => *entry = v.clone(),
                (None, v) => { t.insert(k.clone(), v.clone()); }
            }
        }
    }
}
