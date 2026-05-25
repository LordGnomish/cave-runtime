// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Composition engine — renders composed resources from a Composition + claim spec.

use crate::error::{CrossplaneError, CrossplaneResult};
use crate::models::{
    Composition, ConvertTransform, MatchTransform, MatchType, MathTransform, Patch, PatchType,
    StringTransform, StringTransformType, Transform, TransformType,
};
use std::collections::HashMap;

pub struct CompositionEngine;

impl CompositionEngine {
    pub fn new() -> Self {
        Self
    }

    /// Render all composed resources from a composition + claim spec.
    pub fn render(
        &self,
        composition: &Composition,
        claim_spec: &serde_json::Value,
    ) -> CrossplaneResult<Vec<serde_json::Value>> {
        let mut results = Vec::new();

        for resource in &composition.resources {
            let mut base = resource.base.clone();

            for patch in &resource.patches {
                self.apply_patch(&mut base, patch, claim_spec, &composition.patch_sets)?;
            }

            results.push(base);
        }

        Ok(results)
    }

    /// Apply a single patch to a resource base object.
    pub fn apply_patch(
        &self,
        base: &mut serde_json::Value,
        patch: &Patch,
        claim_spec: &serde_json::Value,
        patch_sets: &[crate::models::PatchSet],
    ) -> CrossplaneResult<()> {
        match patch.patch_type {
            PatchType::FromCompositeFieldPath => {
                if let (Some(from), Some(to)) = (&patch.from_field_path, &patch.to_field_path) {
                    if let Some(value) = get_field_path(claim_spec, from) {
                        let transformed = Self::apply_transforms(value, &patch.transforms)?;
                        set_field_path(base, to, transformed);
                    }
                }
            }
            PatchType::ToCompositeFieldPath => {
                if let (Some(from), Some(_to)) = (&patch.from_field_path, &patch.to_field_path) {
                    // Read from composed resource back to composite — no-op in render phase
                    let _ = get_field_path(base, from);
                }
            }
            PatchType::CombineFromComposite => {
                if let (Some(combine), Some(to)) = (&patch.combine, &patch.to_field_path) {
                    let mut parts: Vec<String> = Vec::new();
                    for var in &combine.variables {
                        if let Some(v) = get_field_path(claim_spec, &var.from_field_path) {
                            parts.push(value_to_string(&v));
                        }
                    }
                    let combined = if let Some(string_spec) = &combine.string {
                        // Simple format substitution: replace {} placeholders
                        let mut result = string_spec.format.clone();
                        for part in &parts {
                            if let Some(pos) = result.find("{}") {
                                result.replace_range(pos..pos + 2, part);
                            }
                        }
                        result
                    } else {
                        parts.join("")
                    };
                    let value = serde_json::Value::String(combined);
                    let transformed = Self::apply_transforms(value, &patch.transforms)?;
                    set_field_path(base, to, transformed);
                }
            }
            PatchType::PatchSet => {
                if let Some(ps_name) = &patch.patch_set_name {
                    if let Some(ps) = patch_sets.iter().find(|ps| &ps.name == ps_name) {
                        for ps_patch in &ps.patches {
                            self.apply_patch(base, ps_patch, claim_spec, patch_sets)?;
                        }
                    }
                }
            }
            PatchType::CombineToComposite
            | PatchType::FromEnvironmentFieldPath
            | PatchType::ToEnvironmentFieldPath => {
                // No-op in render phase (environment not modelled here)
            }
        }
        Ok(())
    }

    /// Apply a chain of transforms to a value.
    pub fn apply_transforms(
        value: serde_json::Value,
        transforms: &[Transform],
    ) -> CrossplaneResult<serde_json::Value> {
        let mut v = value;
        for transform in transforms {
            v = match transform.transform_type {
                TransformType::Map => {
                    if let Some(map) = &transform.map {
                        Self::apply_map_transform(v, map)
                    } else {
                        v
                    }
                }
                TransformType::Math => {
                    if let Some(math) = &transform.math {
                        Self::apply_math_transform(v, math)
                    } else {
                        v
                    }
                }
                TransformType::String => {
                    if let Some(st) = &transform.string {
                        Self::apply_string_transform(v, st)?
                    } else {
                        v
                    }
                }
                TransformType::Convert => {
                    if let Some(ct) = &transform.convert {
                        Self::apply_convert_transform(v, ct)
                    } else {
                        v
                    }
                }
                TransformType::Match => {
                    if let Some(mt) = &transform.match_tf {
                        Self::apply_match_transform(v, mt)
                    } else {
                        v
                    }
                }
            };
        }
        Ok(v)
    }

    pub fn apply_map_transform(
        v: serde_json::Value,
        map: &HashMap<String, String>,
    ) -> serde_json::Value {
        let key = value_to_string(&v);
        if let Some(mapped) = map.get(&key) {
            serde_json::Value::String(mapped.clone())
        } else {
            v
        }
    }

    pub fn apply_math_transform(v: serde_json::Value, math: &MathTransform) -> serde_json::Value {
        let num = match &v {
            serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
            serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
            _ => return v,
        };

        let mut result = num;

        if let Some(multiplier) = math.multiply {
            result *= multiplier;
        }
        if let Some(min) = math.clamp_min {
            if result < min {
                result = min;
            }
        }
        if let Some(max) = math.clamp_max {
            if result > max {
                result = max;
            }
        }

        serde_json::json!(result)
    }

    pub fn apply_string_transform(
        v: serde_json::Value,
        st: &StringTransform,
    ) -> CrossplaneResult<serde_json::Value> {
        let s = value_to_string(&v);
        let result = match st.kind {
            StringTransformType::Format => {
                if let Some(fmt) = &st.format {
                    fmt.replace("{}", &s)
                } else {
                    s
                }
            }
            StringTransformType::Convert => {
                if let Some(fmt) = &st.format {
                    match fmt.as_str() {
                        "ToUpper" | "to_upper" => s.to_uppercase(),
                        "ToLower" | "to_lower" => s.to_lowercase(),
                        "ToBase64" | "to_base64" => {
                            use std::fmt::Write as _;
                            // Simple base64-like encoding — use hex fallback without external dep
                            let mut out = String::new();
                            for b in s.as_bytes() {
                                let _ = write!(out, "{:02x}", b);
                            }
                            out
                        }
                        _ => s,
                    }
                } else {
                    s
                }
            }
            StringTransformType::TrimPrefix => {
                if let Some(prefix) = &st.format {
                    s.strip_prefix(prefix.as_str()).unwrap_or(&s).to_owned()
                } else {
                    s
                }
            }
            StringTransformType::TrimSuffix => {
                if let Some(suffix) = &st.format {
                    s.strip_suffix(suffix.as_str()).unwrap_or(&s).to_owned()
                } else {
                    s
                }
            }
            StringTransformType::Regexp => {
                if let Some(re_cfg) = &st.regexp {
                    let re =
                        regex::Regex::new(&re_cfg.match_pattern).map_err(|e: regex::Error| {
                            CrossplaneError::PatchTransform(e.to_string())
                        })?;
                    if let Some(caps) = re.captures(&s) {
                        let group_idx = re_cfg.group.unwrap_or(0) as usize;
                        caps.get(group_idx)
                            .map(|m: regex::Match| m.as_str().to_owned())
                            .unwrap_or_default()
                    } else {
                        s
                    }
                } else {
                    s
                }
            }
        };
        Ok(serde_json::Value::String(result))
    }

    pub fn apply_convert_transform(
        v: serde_json::Value,
        ct: &ConvertTransform,
    ) -> serde_json::Value {
        match ct.to_type.as_str() {
            "string" => serde_json::Value::String(value_to_string(&v)),
            "int" | "integer" => {
                let n = match &v {
                    serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
                    serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(0),
                    serde_json::Value::Bool(b) => {
                        if *b {
                            1
                        } else {
                            0
                        }
                    }
                    _ => 0,
                };
                serde_json::json!(n)
            }
            "float" | "number" => {
                let n = match &v {
                    serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
                    serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
                    serde_json::Value::Bool(b) => {
                        if *b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    _ => 0.0,
                };
                serde_json::json!(n)
            }
            "bool" | "boolean" => {
                let b = match &v {
                    serde_json::Value::Bool(b) => *b,
                    serde_json::Value::String(s) => {
                        matches!(s.to_lowercase().as_str(), "true" | "1" | "yes")
                    }
                    serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
                    _ => false,
                };
                serde_json::Value::Bool(b)
            }
            _ => v,
        }
    }

    pub fn apply_match_transform(v: serde_json::Value, mt: &MatchTransform) -> serde_json::Value {
        let s = value_to_string(&v);

        for pattern in &mt.patterns {
            let matched = match pattern.match_type {
                MatchType::Literal => pattern.literal.as_ref().map(|l| l == &s).unwrap_or(false),
                MatchType::Regexp => {
                    if let Some(re_str) = &pattern.regexp {
                        regex::Regex::new(re_str)
                            .map(|re| re.is_match(&s))
                            .unwrap_or(false)
                    } else {
                        false
                    }
                }
            };

            if matched {
                return pattern.result.clone();
            }
        }

        mt.fallback_value.clone().unwrap_or(v)
    }
}

impl Default for CompositionEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ── Field path helpers ────────────────────────────────────────────────────────

/// Walk a dot-separated path in a JSON value.
pub fn get_field_path(obj: &serde_json::Value, path: &str) -> Option<serde_json::Value> {
    let mut current = obj;
    for segment in path.split('.') {
        match current {
            serde_json::Value::Object(map) => {
                current = map.get(segment)?;
            }
            serde_json::Value::Array(arr) => {
                let idx: usize = segment.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(current.clone())
}

/// Write a value at a dot-separated path, creating intermediate objects as needed.
pub fn set_field_path(obj: &mut serde_json::Value, path: &str, value: serde_json::Value) {
    let segments: Vec<&str> = path.split('.').collect();
    let mut current = obj;

    for (i, segment) in segments.iter().enumerate() {
        if i == segments.len() - 1 {
            if let serde_json::Value::Object(map) = current {
                map.insert(segment.to_string(), value);
                return;
            }
        } else {
            if !current.is_object() {
                *current = serde_json::Value::Object(serde_json::Map::new());
            }
            if let serde_json::Value::Object(map) = current {
                let entry = map
                    .entry(segment.to_string())
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                current = entry;
            }
        }
    }
}

fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_owned(),
        other => other.to_string(),
    }
}
