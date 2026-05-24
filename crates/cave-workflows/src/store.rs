// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory CRUD store for Argo Workflows CRD objects.

use crate::workflow_crd::Workflow;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Default)]
pub struct WorkflowStore {
    by_id: Arc<RwLock<HashMap<Uuid, Workflow>>>,
    by_name: Arc<RwLock<HashMap<(String, String), Uuid>>>,
}

impl WorkflowStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&self, wf: Workflow) -> Result<Workflow, String> {
        let key = (wf.namespace.clone(), wf.name.clone());
        let mut idx = self.by_name.write().unwrap();
        if idx.contains_key(&key) {
            return Err(format!("workflow {}/{} already exists", key.0, key.1));
        }
        idx.insert(key, wf.id);
        self.by_id.write().unwrap().insert(wf.id, wf.clone());
        Ok(wf)
    }

    pub fn get(&self, namespace: &str, name: &str) -> Option<Workflow> {
        let key = (namespace.to_string(), name.to_string());
        let id = *self.by_name.read().unwrap().get(&key)?;
        self.by_id.read().unwrap().get(&id).cloned()
    }

    pub fn list(&self, namespace: Option<&str>) -> Vec<Workflow> {
        let all = self.by_id.read().unwrap();
        match namespace {
            Some(ns) => all.values().filter(|w| w.namespace == ns).cloned().collect(),
            None => all.values().cloned().collect(),
        }
    }

    pub fn delete(&self, namespace: &str, name: &str) -> Result<(), String> {
        let key = (namespace.to_string(), name.to_string());
        let id = self
            .by_name
            .write()
            .unwrap()
            .remove(&key)
            .ok_or_else(|| format!("workflow {namespace}/{name} not found"))?;
        self.by_id.write().unwrap().remove(&id);
        Ok(())
    }

    pub fn update<F: FnOnce(&mut Workflow)>(
        &self,
        namespace: &str,
        name: &str,
        f: F,
    ) -> Result<Workflow, String> {
        let key = (namespace.to_string(), name.to_string());
        let id = *self
            .by_name
            .read()
            .unwrap()
            .get(&key)
            .ok_or_else(|| format!("workflow {namespace}/{name} not found"))?;
        let mut all = self.by_id.write().unwrap();
        let wf = all.get_mut(&id).ok_or("index drift")?;
        f(wf);
        Ok(wf.clone())
    }

    pub fn len(&self) -> usize {
        self.by_id.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_crd::{
        Arguments, ContainerTemplate, Inputs, Outputs, Template, TemplateBody, WorkflowSpec,
    };
    use std::collections::HashMap as M;

    fn wf(name: &str) -> Workflow {
        Workflow::new(
            name,
            "argo",
            WorkflowSpec {
                entrypoint: "main".into(),
                templates: vec![Template {
                    name: "main".into(),
                    inputs: Inputs::default(),
                    outputs: Outputs::default(),
                    body: TemplateBody::Container(ContainerTemplate {
                        image: "alpine".into(),
                        command: vec![],
                        args: vec![],
                        env: M::new(),
                        working_dir: None,
                    }),
                    retry_strategy: None,
                    timeout: None,
                }],
                arguments: Arguments::default(),
                service_account_name: None,
                on_exit: None,
                parallelism: None,
                workflow_template_ref: None,
            },
        )
    }

    #[test]
    fn create_get_list_delete_round_trip() {
        let s = WorkflowStore::new();
        s.create(wf("a")).unwrap();
        s.create(wf("b")).unwrap();
        assert_eq!(s.len(), 2);
        assert!(s.get("argo", "a").is_some());
        assert_eq!(s.list(Some("argo")).len(), 2);
        s.delete("argo", "a").unwrap();
        assert_eq!(s.len(), 1);
        assert!(s.get("argo", "a").is_none());
    }

    #[test]
    fn duplicate_create_fails() {
        let s = WorkflowStore::new();
        s.create(wf("a")).unwrap();
        assert!(s.create(wf("a")).is_err());
    }

    #[test]
    fn delete_missing_errors() {
        let s = WorkflowStore::new();
        assert!(s.delete("argo", "ghost").is_err());
    }

    #[test]
    fn update_mutates_in_place() {
        let s = WorkflowStore::new();
        s.create(wf("a")).unwrap();
        let updated = s
            .update("argo", "a", |w| w.spec.parallelism = Some(4))
            .unwrap();
        assert_eq!(updated.spec.parallelism, Some(4));
        assert_eq!(s.get("argo", "a").unwrap().spec.parallelism, Some(4));
    }
}
