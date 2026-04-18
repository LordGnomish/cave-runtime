use crate::models::{
    Component, ComponentGroup, ComponentStatus, Maintenance, OverallPageStatus, StatusIncident,
};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Default)]
pub struct StatusStore {
    pub components: RwLock<HashMap<Uuid, Component>>,
    pub groups: RwLock<HashMap<Uuid, ComponentGroup>>,
    pub incidents: RwLock<HashMap<Uuid, StatusIncident>>,
    pub maintenance: RwLock<HashMap<Uuid, Maintenance>>,
}

impl StatusStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    // -----------------------------------------------------------------------
    // Components
    // -----------------------------------------------------------------------

    pub fn insert_component(&self, c: Component) {
        self.components.write().unwrap().insert(c.id, c);
    }

    pub fn get_component(&self, id: Uuid) -> Option<Component> {
        self.components.read().unwrap().get(&id).cloned()
    }

    pub fn list_components(&self) -> Vec<Component> {
        let mut v: Vec<Component> = self.components.read().unwrap().values().cloned().collect();
        v.sort_by_key(|c| c.order);
        v
    }

    pub fn update_component(&self, c: Component) -> bool {
        let mut map = self.components.write().unwrap();
        if map.contains_key(&c.id) {
            map.insert(c.id, c);
            true
        } else {
            false
        }
    }

    pub fn delete_component(&self, id: Uuid) -> bool {
        self.components.write().unwrap().remove(&id).is_some()
    }

    // -----------------------------------------------------------------------
    // Groups
    // -----------------------------------------------------------------------

    pub fn insert_group(&self, g: ComponentGroup) {
        self.groups.write().unwrap().insert(g.id, g);
    }

    pub fn list_groups(&self) -> Vec<ComponentGroup> {
        let components = self.components.read().unwrap();
        let groups = self.groups.read().unwrap();

        let mut result: Vec<ComponentGroup> = groups
            .values()
            .map(|g| {
                let mut group = g.clone();
                group.components = components
                    .values()
                    .filter(|c| c.group_id == Some(g.id))
                    .cloned()
                    .collect();
                group.components.sort_by_key(|c| c.order);
                group
            })
            .collect();
        result.sort_by_key(|g| g.order);
        result
    }

    // -----------------------------------------------------------------------
    // Incidents
    // -----------------------------------------------------------------------

    pub fn insert_incident(&self, inc: StatusIncident) {
        self.incidents.write().unwrap().insert(inc.id, inc);
    }

    pub fn get_incident(&self, id: Uuid) -> Option<StatusIncident> {
        self.incidents.read().unwrap().get(&id).cloned()
    }

    pub fn update_incident(&self, inc: StatusIncident) -> bool {
        let mut map = self.incidents.write().unwrap();
        if map.contains_key(&inc.id) {
            map.insert(inc.id, inc);
            true
        } else {
            false
        }
    }

    pub fn list_incidents(&self) -> Vec<StatusIncident> {
        self.incidents.read().unwrap().values().cloned().collect()
    }

    pub fn list_active_incidents(&self) -> Vec<StatusIncident> {
        use crate::models::IncidentStatus;
        self.incidents
            .read()
            .unwrap()
            .values()
            .filter(|i| i.status != IncidentStatus::Resolved)
            .cloned()
            .collect()
    }

    // -----------------------------------------------------------------------
    // Maintenance
    // -----------------------------------------------------------------------

    pub fn insert_maintenance(&self, m: Maintenance) {
        self.maintenance.write().unwrap().insert(m.id, m);
    }

    pub fn list_maintenance(&self) -> Vec<Maintenance> {
        self.maintenance.read().unwrap().values().cloned().collect()
    }

    pub fn list_scheduled_maintenance(&self) -> Vec<Maintenance> {
        use crate::models::MaintenanceStatus;
        self.maintenance
            .read()
            .unwrap()
            .values()
            .filter(|m| m.status == MaintenanceStatus::Scheduled)
            .cloned()
            .collect()
    }

    // -----------------------------------------------------------------------
    // Overall status
    // -----------------------------------------------------------------------

    pub fn compute_overall_status(&self) -> OverallPageStatus {
        let components = self.components.read().unwrap();
        if components.is_empty() {
            return OverallPageStatus::AllOperational;
        }

        let worst = components
            .values()
            .map(|c| status_rank(&c.status))
            .max()
            .unwrap_or(0);

        match worst {
            0 => OverallPageStatus::AllOperational,
            1 => OverallPageStatus::DegradedPerformance,
            2 => OverallPageStatus::PartialOutage,
            3 => OverallPageStatus::UnderMaintenance,
            _ => OverallPageStatus::MajorOutage,
        }
    }
}

fn status_rank(s: &ComponentStatus) -> u8 {
    match s {
        ComponentStatus::Operational => 0,
        ComponentStatus::DegradedPerformance => 1,
        ComponentStatus::PartialOutage => 2,
        ComponentStatus::UnderMaintenance => 3,
        ComponentStatus::MajorOutage => 4,
    }
}
