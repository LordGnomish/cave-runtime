// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory store for VirtualMachine / VirtualMachineInstance / DataVolume.
//!
//! Placeholder backing for the KubeVirt scaffold. Persistence layer
//! (cave-rdbms-operator / cave-etcd) will be wired once the lifecycle controller
//! reaches parity with upstream VMController reconcile.

use std::collections::HashMap;
use std::sync::RwLock;

use crate::models::{DataVolume, VirtualMachine, VirtualMachineInstance};

#[derive(Default)]
pub struct Store {
    vms: RwLock<HashMap<String, VirtualMachine>>,
    vmis: RwLock<HashMap<String, VirtualMachineInstance>>,
    dvs: RwLock<HashMap<String, DataVolume>>,
}

fn ns_key(ns: &Option<String>, name: &str) -> String {
    format!("{}/{}", ns.as_deref().unwrap_or("default"), name)
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put_vm(&self, vm: VirtualMachine) {
        let key = ns_key(&vm.namespace, &vm.name);
        self.vms.write().unwrap().insert(key, vm);
    }

    pub fn get_vm(&self, namespace: &str, name: &str) -> Option<VirtualMachine> {
        let key = format!("{namespace}/{name}");
        self.vms.read().unwrap().get(&key).cloned()
    }

    pub fn list_vms(&self) -> Vec<VirtualMachine> {
        self.vms.read().unwrap().values().cloned().collect()
    }

    pub fn delete_vm(&self, namespace: &str, name: &str) -> bool {
        let key = format!("{namespace}/{name}");
        self.vms.write().unwrap().remove(&key).is_some()
    }

    pub fn put_vmi(&self, vmi: VirtualMachineInstance) {
        let key = ns_key(&vmi.namespace, &vmi.name);
        self.vmis.write().unwrap().insert(key, vmi);
    }

    pub fn get_vmi(&self, namespace: &str, name: &str) -> Option<VirtualMachineInstance> {
        let key = format!("{namespace}/{name}");
        self.vmis.read().unwrap().get(&key).cloned()
    }

    pub fn list_vmis(&self) -> Vec<VirtualMachineInstance> {
        self.vmis.read().unwrap().values().cloned().collect()
    }

    pub fn delete_vmi(&self, namespace: &str, name: &str) -> bool {
        let key = format!("{namespace}/{name}");
        self.vmis.write().unwrap().remove(&key).is_some()
    }

    pub fn put_data_volume(&self, dv: DataVolume) {
        let key = ns_key(&dv.namespace, &dv.name);
        self.dvs.write().unwrap().insert(key, dv);
    }

    pub fn get_data_volume(&self, namespace: &str, name: &str) -> Option<DataVolume> {
        let key = format!("{namespace}/{name}");
        self.dvs.read().unwrap().get(&key).cloned()
    }

    pub fn list_data_volumes(&self) -> Vec<DataVolume> {
        self.dvs.read().unwrap().values().cloned().collect()
    }

    pub fn delete_data_volume(&self, namespace: &str, name: &str) -> bool {
        let key = format!("{namespace}/{name}");
        self.dvs.write().unwrap().remove(&key).is_some()
    }
}
