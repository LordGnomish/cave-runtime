// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gatekeeper generator-resource expansion engine.
//!
//! Faithful line-port of the *pure* core of Gatekeeper's expansion subsystem
//! (gatekeeper v3.22.2, source_sha eda110bdaf2510288dccd73a1be4dd0c6442a4aa):
//!
//!   - `pkg/expansion/system.go`  — [`ExpansionSystem::expand`] /
//!     [`expand_resource`] / [`ExpansionSystem::upsert_template`] (ValidateTemplate)
//!     / [`mock_name_for_resource`] / [`ensure_owner_reference`] /
//!     [`ExpansionSystem::expand_recursive`] with `MAX_RECURSION_DEPTH`.
//!   - `pkg/expansion/db.go`      — matchers/generators adjacency maps,
//!     [`TemplateDb::templates_for_gvk`], and the cycle-detection graph. The
//!     upstream delegates cycle/SCC detection to `dominikbraun/graph`; we port
//!     the standard primitive (DFS-based directed-cycle detection) in-crate per
//!     ADR-RUNTIME-SANDBOX-NO-FFI-001 — no new workspace dependency.
//!   - `pkg/mutation/match/apply_to.go` — [`ApplyTo::matches`] / [`ApplyTo::flatten`].
//!
//! Scope note: the *controller* that drives this (watch/reconcile, readiness,
//! periodic audit) genuinely belongs to the Phase-2 k8s-controller-runtime port
//! and remains skipped. The mutation-system hook (`if s.mutationSystem == nil`)
//! is honoured here as the documented nil path — expanded resultants are
//! returned without mutator application, exactly as upstream does when no
//! mutation system is wired.
//!
//! All resources are represented as `serde_json::Value`, the Rust analogue of
//! upstream `unstructured.Unstructured`.

use crate::error::{PolicyError, PolicyResult};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

/// `maxRecursionDepth` (system.go) — safeguard against template cycles slipping
/// past the DB-level cycle check.
const MAX_RECURSION_DEPTH: usize = 30;

/// Group/Version/Kind tuple (k8s.io/apimachinery schema.GroupVersionKind).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct GroupVersionKind {
    pub group: String,
    pub version: String,
    pub kind: String,
}

impl GroupVersionKind {
    /// Equivalent to Go's `gvk == (schema.GroupVersionKind{})`.
    fn is_empty(&self) -> bool {
        self.group.is_empty() && self.version.is_empty() && self.kind.is_empty()
    }

    /// Reconstruct the `apiVersion` string ("group/version", or just "version"
    /// for the core group).
    fn api_version(&self) -> String {
        if self.group.is_empty() {
            self.version.clone()
        } else {
            format!("{}/{}", self.group, self.version)
        }
    }
}

/// `match.ApplyTo` (apply_to.go) — the set of GVKs a template applies to.
#[derive(Debug, Clone, Default)]
pub struct ApplyTo {
    pub groups: Vec<String>,
    pub versions: Vec<String>,
    pub kinds: Vec<String>,
}

impl ApplyTo {
    /// `Matches` (apply_to.go:45) — true iff group, version and kind are each
    /// contained in the corresponding list.
    pub fn matches(&self, gvk: &GroupVersionKind) -> bool {
        self.groups.iter().any(|g| g == &gvk.group)
            && self.versions.iter().any(|v| v == &gvk.version)
            && self.kinds.iter().any(|k| k == &gvk.kind)
    }

    /// `Flatten` (apply_to.go:26) — cartesian product groups × versions × kinds.
    pub fn flatten(&self) -> Vec<GroupVersionKind> {
        let mut out = Vec::new();
        for g in &self.groups {
            for v in &self.versions {
                for k in &self.kinds {
                    out.push(GroupVersionKind {
                        group: g.clone(),
                        version: v.clone(),
                        kind: k.clone(),
                    });
                }
            }
        }
        out
    }
}

/// `expansionunversioned.GeneratedGVK` (expansiontemplate_types.go).
#[derive(Debug, Clone, Default)]
pub struct GeneratedGvk {
    pub group: String,
    pub version: String,
    pub kind: String,
}

impl GeneratedGvk {
    fn is_empty(&self) -> bool {
        self.group.is_empty() && self.version.is_empty() && self.kind.is_empty()
    }

    /// `genGVKToSchemaGVK` (system.go:126).
    fn to_gvk(&self) -> GroupVersionKind {
        GroupVersionKind {
            group: self.group.clone(),
            version: self.version.clone(),
            kind: self.kind.clone(),
        }
    }
}

/// `expansionunversioned.ExpansionTemplateSpec`.
#[derive(Debug, Clone, Default)]
pub struct ExpansionTemplateSpec {
    pub apply_to: Vec<ApplyTo>,
    pub template_source: String,
    pub generated_gvk: GeneratedGvk,
    pub enforcement_action: String,
}

/// `expansionunversioned.ExpansionTemplate`.
#[derive(Debug, Clone, Default)]
pub struct ExpansionTemplate {
    pub name: String,
    pub spec: ExpansionTemplateSpec,
}

impl ExpansionTemplate {
    /// `applyToGVKs` (db.go:278) — flatten every ApplyTo on the template.
    fn apply_to_gvks(&self) -> Vec<GroupVersionKind> {
        self.spec.apply_to.iter().flat_map(|a| a.flatten()).collect()
    }
}

/// One resultant produced by expanding a generator resource.
/// (`Resultant`, system.go:42).
#[derive(Debug, Clone)]
pub struct Resultant {
    pub obj: Value,
    pub template_name: String,
    pub enforcement_action: String,
}

/// `ValidateTemplate` (system.go:85) — pure structural validation, including
/// the self-edge check (a template may not generate a GVK it also applies to).
fn validate_template(t: &ExpansionTemplate) -> PolicyResult<()> {
    if t.name.is_empty() {
        return Err(PolicyError::Validation(
            "ExpansionTemplate has empty name field".into(),
        ));
    }
    if t.name.len() >= 64 {
        return Err(PolicyError::Validation(
            "ExpansionTemplate name must be less than 64 characters".into(),
        ));
    }
    if t.spec.template_source.is_empty() {
        return Err(PolicyError::Validation(format!(
            "ExpansionTemplate {} has empty source field",
            t.name
        )));
    }
    if t.spec.generated_gvk.is_empty() {
        return Err(PolicyError::Validation(format!(
            "ExpansionTemplate {} has empty generatedGVK field",
            t.name
        )));
    }
    if t.spec.apply_to.is_empty() {
        return Err(PolicyError::Validation(format!(
            "ExpansionTemplate {} must specify ApplyTo",
            t.name
        )));
    }
    // Self-edge: generated GVK must not also be matched by ApplyTo.
    let gen_gvk = t.spec.generated_gvk.to_gvk();
    for apply in &t.spec.apply_to {
        if apply.matches(&gen_gvk) {
            return Err(PolicyError::Validation(format!(
                "ExpansionTemplate {} generates GVK {:?}, but also applies to that same GVK",
                t.name, gen_gvk
            )));
        }
    }
    Ok(())
}

/// `sourcePath` (system.go:114) — dotted path split.
fn source_path(source: &str) -> Vec<&str> {
    source.split('.').collect()
}

/// Equivalent of `unstructured.NestedMap` for the dotted source path. Returns
/// the nested object (an `Ok(Some(map))`), `Ok(None)` if absent, or `Err` if a
/// path segment is present but not an object.
fn nested_map(obj: &Value, path: &[&str]) -> PolicyResult<Option<Value>> {
    let mut cur = obj;
    for seg in path {
        match cur {
            Value::Object(m) => match m.get(*seg) {
                Some(v) => cur = v,
                None => return Ok(None),
            },
            _ => {
                return Err(PolicyError::Validation(format!(
                    "could not extract source field: {seg} is not an object"
                )))
            }
        }
    }
    match cur {
        Value::Object(_) => Ok(Some(cur.clone())),
        _ => Err(PolicyError::Validation(
            "source field is not a map".into(),
        )),
    }
}

/// `gvk := base.Object.GroupVersionKind()` — derive the GVK of an unstructured
/// object from its `apiVersion`/`kind`.
fn gvk_of(obj: &Value) -> GroupVersionKind {
    let api_version = obj.get("apiVersion").and_then(Value::as_str).unwrap_or("");
    let kind = obj
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let (group, version) = match api_version.split_once('/') {
        Some((g, v)) => (g.to_string(), v.to_string()),
        None => (String::new(), api_version.to_string()),
    };
    GroupVersionKind {
        group,
        version,
        kind,
    }
}

/// `mockNameForResource` (system.go:290) — "<generator name>-<kind>",
/// lowercased.
fn mock_name_for_resource(generator: &Value, gvk: &GroupVersionKind) -> String {
    let mut name = generator
        .pointer("/metadata/name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !gvk.kind.is_empty() {
        name.push('-');
    }
    name.push_str(&gvk.kind);
    name.to_lowercase()
}

/// `ensureOwnerReference` (system.go:258) — append an OwnerReference for the
/// parent if one is not already present (matched on apiVersion+kind+name).
fn ensure_owner_reference(resultant: &mut Value, parent: &Value) {
    let parent_api_version = parent.get("apiVersion").and_then(Value::as_str).unwrap_or("");
    let parent_kind = parent.get("kind").and_then(Value::as_str).unwrap_or("");
    let parent_name = parent
        .pointer("/metadata/name")
        .and_then(Value::as_str)
        .unwrap_or("");
    if parent_api_version.is_empty() || parent_kind.is_empty() || parent_name.is_empty() {
        return;
    }

    let metadata = resultant
        .as_object_mut()
        .expect("resultant must be an object")
        .entry("metadata")
        .or_insert_with(|| Value::Object(Map::new()));
    let meta_map = metadata
        .as_object_mut()
        .expect("metadata must be an object");
    let owners = meta_map
        .entry("ownerReferences")
        .or_insert_with(|| Value::Array(Vec::new()));
    let arr = match owners {
        Value::Array(a) => a,
        _ => return,
    };
    for r in arr.iter() {
        if r.get("apiVersion").and_then(Value::as_str) == Some(parent_api_version)
            && r.get("kind").and_then(Value::as_str) == Some(parent_kind)
            && r.get("name").and_then(Value::as_str) == Some(parent_name)
        {
            return;
        }
    }
    let mut new_ref = Map::new();
    new_ref.insert("apiVersion".into(), Value::String(parent_api_version.into()));
    new_ref.insert("kind".into(), Value::String(parent_kind.into()));
    new_ref.insert("name".into(), Value::String(parent_name.into()));
    arr.push(Value::Object(new_ref));
}

/// `expandResource` (system.go:215) — extract the source field, re-stamp the
/// resultant GVK, inherit namespace, mock the name and ensure the parent owner
/// reference.
fn expand_resource(obj: &Value, template: &ExpansionTemplate) -> PolicyResult<Value> {
    let src_path = &template.spec.template_source;
    if src_path.is_empty() {
        return Err(PolicyError::Validation(
            "cannot expand resource using a template with no source".into(),
        ));
    }
    let resultant_gvk = template.spec.generated_gvk.to_gvk();
    if resultant_gvk.is_empty() {
        return Err(PolicyError::Validation(
            "cannot expand resource using template with empty generatedGVK".into(),
        ));
    }

    let src = nested_map(obj, &source_path(src_path))?.ok_or_else(|| {
        PolicyError::Validation(format!(
            "could not find source field {src_path:?} in resource"
        ))
    })?;

    let mut resource = src; // SetUnstructuredContent(src)
    let map = resource
        .as_object_mut()
        .expect("source map is an object by nested_map contract");

    // SetGroupVersionKind(resultantGVK)
    map.insert(
        "apiVersion".into(),
        Value::String(resultant_gvk.api_version()),
    );
    map.insert("kind".into(), Value::String(resultant_gvk.kind.clone()));

    // Namespace: no explicit Namespace override here (the controller path would
    // pass one); inherit from parent metadata.namespace if present.
    if let Some(ns) = obj
        .pointer("/metadata/namespace")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        let meta = map
            .entry("metadata")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(mm) = meta.as_object_mut() {
            mm.insert("namespace".into(), Value::String(ns));
        }
    }

    // SetName(mockNameForResource(...))
    let name = mock_name_for_resource(obj, &resultant_gvk);
    {
        let meta = map
            .entry("metadata")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(mm) = meta.as_object_mut() {
            mm.insert("name".into(), Value::String(name));
        }
    }

    ensure_owner_reference(&mut resource, obj);
    Ok(resource)
}

// ─── Template DB (db.go) ────────────────────────────────────────────────────

#[derive(Default)]
struct TemplateState {
    template: ExpansionTemplate,
    has_conflicts: bool,
}

/// In-memory template DB: maintains the matchers/generators adjacency maps and a
/// directed graph (edges template→template) used for expansion-cycle detection.
#[derive(Default)]
struct TemplateDb {
    store: HashMap<String, TemplateState>,
    /// GVK → templates whose ApplyTo matches that GVK.
    matchers: HashMap<GroupVersionKind, HashSet<String>>,
    /// GVK → templates that generate that GVK.
    generators: HashMap<GroupVersionKind, HashSet<String>>,
    /// Adjacency list of the directed template graph (id → out-neighbours).
    /// An edge A→B means A's generatedGVK matches B's ApplyTo.
    adj: BTreeMap<String, HashSet<String>>,
}

impl TemplateDb {
    /// `edgesForTemplate` (db.go:134) — out-bound (genGVK→matchers) and in-bound
    /// (generators→applyTo) edges for the given template.
    fn edges_for_template(&self, template: &ExpansionTemplate) -> Vec<(String, String)> {
        let id = template.name.clone();
        let gen_gvk = template.spec.generated_gvk.to_gvk();
        let mut edges = Vec::new();
        if let Some(set) = self.matchers.get(&gen_gvk) {
            for t in set {
                edges.push((id.clone(), t.clone()));
            }
        }
        for gvk in template.apply_to_gvks() {
            if let Some(set) = self.generators.get(&gvk) {
                for t in set {
                    edges.push((t.clone(), id.clone()));
                }
            }
        }
        edges
    }

    /// `handleAdd` (db.go:74) — insert the template + adjacency, add graph edges,
    /// returning true if any new edge closes a directed cycle. The template is
    /// added even when a cycle is found (mirrors upstream).
    fn handle_add(&mut self, template: &ExpansionTemplate) -> bool {
        let id = template.name.clone();
        self.store.insert(
            id.clone(),
            TemplateState {
                template: template.clone(),
                has_conflicts: false,
            },
        );

        let gen_gvk = template.spec.generated_gvk.to_gvk();
        self.generators.entry(gen_gvk).or_default().insert(id.clone());
        for m in template.apply_to_gvks() {
            self.matchers.entry(m).or_default().insert(id.clone());
        }

        self.adj.entry(id.clone()).or_default();

        let edges = self.edges_for_template(template);
        let mut cycle = false;
        for (from, to) in edges {
            // CreatesCycle: adding from→to closes a cycle iff `from` is already
            // reachable from `to`.
            if self.reachable(&to, &from) {
                cycle = true;
            }
            self.adj.entry(to.clone()).or_default();
            self.adj.entry(from).or_default().insert(to);
        }
        cycle
    }

    /// `handleRemove` (db.go:156) — drop the template from store + adjacency maps
    /// + graph edges.
    fn handle_remove(&mut self, id: &str) {
        let template = match self.store.remove(id) {
            Some(s) => s.template,
            None => return,
        };

        let gen_gvk = template.spec.generated_gvk.to_gvk();
        if let Some(set) = self.generators.get_mut(&gen_gvk) {
            set.remove(id);
            if set.is_empty() {
                self.generators.remove(&gen_gvk);
            }
        }
        for m in template.apply_to_gvks() {
            if let Some(set) = self.matchers.get_mut(&m) {
                set.remove(id);
                if set.is_empty() {
                    self.matchers.remove(&m);
                }
            }
        }
        for (from, to) in self.edges_for_template(&template) {
            if let Some(set) = self.adj.get_mut(&from) {
                set.remove(&to);
            }
        }
        self.adj.remove(id);
    }

    /// DFS reachability: is `target` reachable from `start` following directed
    /// edges? Used both by CreatesCycle and the SCC-conflict scan.
    fn reachable(&self, start: &str, target: &str) -> bool {
        let mut stack = vec![start.to_string()];
        let mut seen = HashSet::new();
        while let Some(node) = stack.pop() {
            if node == target {
                return true;
            }
            if !seen.insert(node.clone()) {
                continue;
            }
            if let Some(neigh) = self.adj.get(&node) {
                for n in neigh {
                    stack.push(n.clone());
                }
            }
        }
        false
    }

    /// `updateCycles` (db.go:201) — recompute `has_conflicts` for every node.
    /// A node is in a cycle iff it can reach itself through ≥1 edge. This is the
    /// "SCC of size > 1 (or self-loop)" predicate the upstream graph library
    /// computes via StronglyConnectedComponents.
    fn update_cycles(&mut self) {
        let ids: Vec<String> = self.store.keys().cloned().collect();
        for id in &ids {
            let in_cycle = self
                .adj
                .get(id)
                .map(|neigh| neigh.iter().any(|n| self.reachable(n, id)))
                .unwrap_or(false);
            if let Some(state) = self.store.get_mut(id) {
                state.has_conflicts = in_cycle;
            }
        }
    }

    /// `upsert` (db.go:222).
    fn upsert(&mut self, template: ExpansionTemplate) -> PolicyResult<()> {
        let id = template.name.clone();
        let had_old = self.store.contains_key(&id);
        let old_conflicts = self
            .store
            .get(&id)
            .map(|s| s.has_conflicts)
            .unwrap_or(false);
        if had_old {
            self.handle_remove(&id);
        }
        let new_cycle = self.handle_add(&template);
        if new_cycle || (had_old && old_conflicts) {
            self.update_cycles();
        }
        if new_cycle {
            return Err(PolicyError::Validation(
                "template forms expansion cycle".into(),
            ));
        }
        Ok(())
    }

    /// `remove` (db.go:245).
    #[allow(dead_code)]
    fn remove(&mut self, id: &str) {
        let old_conflicts = match self.store.get(id) {
            Some(s) => s.has_conflicts,
            None => return,
        };
        self.handle_remove(id);
        if old_conflicts {
            self.update_cycles();
        }
    }

    /// `templatesForGVK` (db.go:260) — non-conflicting templates matching `gvk`.
    fn templates_for_gvk(&self, gvk: &GroupVersionKind) -> Vec<ExpansionTemplate> {
        let mut out = Vec::new();
        if let Some(ids) = self.matchers.get(gvk) {
            // Deterministic order for stable resultant ordering.
            let mut ids: Vec<&String> = ids.iter().collect();
            ids.sort();
            for id in ids {
                if let Some(state) = self.store.get(id) {
                    if !state.has_conflicts {
                        out.push(state.template.clone());
                    }
                }
            }
        }
        out
    }

    /// `getConflicts` (db.go:62).
    #[allow(dead_code)]
    fn get_conflicts(&self) -> Vec<String> {
        self.store
            .iter()
            .filter(|(_, s)| s.has_conflicts)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

// ─── System (system.go) ─────────────────────────────────────────────────────

/// `expansion.System` (system.go:36) — the pure expansion engine. The mutation
/// system hook is intentionally absent (the upstream nil path), so expanded
/// resultants are returned without mutator application.
#[derive(Default)]
pub struct ExpansionSystem {
    db: TemplateDb,
}

impl ExpansionSystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// `UpsertTemplate` (system.go:56) — validate then store the template.
    pub fn upsert_template(&mut self, template: ExpansionTemplate) -> PolicyResult<()> {
        validate_template(&template)?;
        self.db.upsert(template)
    }

    /// `RemoveTemplate` (system.go:68).
    pub fn remove_template(&mut self, name: &str) -> PolicyResult<()> {
        if name.is_empty() {
            return Err(PolicyError::Validation(
                "cannot remove template with empty name".into(),
            ));
        }
        self.db.remove(name);
        Ok(())
    }

    /// Templates currently flagged as forming a cycle.
    pub fn conflicts(&self) -> Vec<String> {
        self.db.get_conflicts()
    }

    /// `Expand` (system.go:137) — expand `base` into resultant resources,
    /// recursing through chained templates.
    pub fn expand(&self, base: &Value) -> PolicyResult<Vec<Resultant>> {
        let mut res = Vec::new();
        self.expand_recursive(base, &mut res, 0)?;
        Ok(res)
    }

    /// `expandRecursive` (system.go:148).
    fn expand_recursive(
        &self,
        base: &Value,
        resultants: &mut Vec<Resultant>,
        depth: usize,
    ) -> PolicyResult<()> {
        if depth >= MAX_RECURSION_DEPTH {
            return Err(PolicyError::Validation(format!(
                "maximum recursion depth of {MAX_RECURSION_DEPTH} reached"
            )));
        }
        let res = self.expand_once(base)?;
        for r in &res {
            self.expand_recursive(&r.obj, resultants, depth + 1)?;
        }
        resultants.extend(res);
        Ok(())
    }

    /// `expand` (system.go:174) — single-level expansion (no mutation system).
    fn expand_once(&self, base: &Value) -> PolicyResult<Vec<Resultant>> {
        let gvk = gvk_of(base);
        if gvk.is_empty() {
            return Err(PolicyError::Validation(
                "cannot expand resource with empty GVK".into(),
            ));
        }
        let mut resultants = Vec::new();
        for te in self.db.templates_for_gvk(&gvk) {
            let obj = expand_resource(base, &te)?;
            resultants.push(Resultant {
                obj,
                template_name: te.name.clone(),
                enforcement_action: te.spec.enforcement_action.clone(),
            });
        }
        Ok(resultants)
    }
}
