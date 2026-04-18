//! Knative Serving — service lifecycle, revision management, scale-to-zero.

use crate::error::{KnativeError, KnativeResult};
use crate::models::{
    CreateServiceRequest, KnRoute, KnService, Revision, RevisionSpec, RevisionStatus,
    RouteStatus, ServiceSpec, ServiceStatus, TrafficTarget, UpdateServiceRequest,
};
use chrono::Utc;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

pub struct ServingStore {
    services: DashMap<String, KnService>,
    revisions: DashMap<String, Revision>,
    routes: DashMap<String, KnRoute>,
    revision_counter: Arc<AtomicU64>,
}

impl ServingStore {
    pub fn new() -> Self {
        Self {
            services: DashMap::new(),
            revisions: DashMap::new(),
            routes: DashMap::new(),
            revision_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    fn ns_key(namespace: &str, name: &str) -> String {
        format!("{namespace}/{name}")
    }

    fn next_revision_name(service_name: &str, counter: u64) -> String {
        format!("{service_name}-{counter:05}")
    }

    pub fn create_service(&self, req: CreateServiceRequest) -> KnativeResult<KnService> {
        let key = Self::ns_key(&req.namespace, &req.name);
        if self.services.contains_key(&key) {
            return Err(KnativeError::Validation(format!("Service {key} already exists")));
        }
        if req.template.container.image.is_empty() {
            return Err(KnativeError::Validation("container image is required".into()));
        }
        let rev_num = self.revision_counter.fetch_add(1, Ordering::Relaxed);
        let revision_name = Self::next_revision_name(&req.name, rev_num);
        let traffic = req.traffic.clone().unwrap_or_else(|| {
            vec![TrafficTarget {
                revision_name: None,
                latest_revision: Some(true),
                percent: 100,
                tag: None,
                url: None,
            }]
        });
        let svc_url = format!("https://{}.{}.svc.cluster.local", req.name, req.namespace);
        let svc = KnService {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            spec: ServiceSpec {
                template: req.template.clone(),
                traffic: traffic.clone(),
            },
            status: ServiceStatus::Ready,
            latest_ready_revision: Some(revision_name.clone()),
            latest_created_revision: Some(revision_name.clone()),
            url: Some(svc_url.clone()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let revision = Revision {
            id: Uuid::new_v4(),
            name: revision_name.clone(),
            service_name: req.name.clone(),
            namespace: req.namespace.clone(),
            spec: RevisionSpec {
                container: req.template.container.clone(),
                scale: req.template.scale.clone(),
                service_account: req.template.service_account.clone(),
                timeout_seconds: req.template.timeout_seconds,
            },
            status: if req.template.scale.min_scale == 0 {
                RevisionStatus::Reserve
            } else {
                RevisionStatus::Active
            },
            current_replicas: req.template.scale.min_scale,
            desired_replicas: req.template.scale.min_scale,
            created_at: Utc::now(),
        };
        let route = KnRoute {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            spec_traffic: traffic.clone(),
            status_traffic: traffic,
            url: svc_url,
            status: RouteStatus::Ready,
            created_at: Utc::now(),
        };
        self.revisions.insert(Self::ns_key(&req.namespace, &revision_name), revision);
        self.routes.insert(Self::ns_key(&req.namespace, &req.name), route);
        self.services.insert(key, svc.clone());
        info!(service = %req.name, namespace = %req.namespace, revision = %revision_name, "knative service created");
        Ok(svc)
    }

    pub fn get_service(&self, namespace: &str, name: &str) -> KnativeResult<KnService> {
        let key = Self::ns_key(namespace, name);
        self.services.get(&key).map(|r| r.clone()).ok_or_else(|| KnativeError::ServiceNotFound(key))
    }

    pub fn list_services(&self, namespace: &str) -> Vec<KnService> {
        self.services.iter().filter(|r| r.value().namespace == namespace).map(|r| r.value().clone()).collect()
    }

    pub fn update_service(&self, namespace: &str, name: &str, req: UpdateServiceRequest) -> KnativeResult<KnService> {
        let key = Self::ns_key(namespace, name);
        let mut svc = self.services.get(&key).map(|r| r.clone()).ok_or_else(|| KnativeError::ServiceNotFound(key.clone()))?;
        let rev_num = self.revision_counter.fetch_add(1, Ordering::Relaxed);
        let revision_name = Self::next_revision_name(name, rev_num);
        if let Some(tmpl) = req.template {
            let new_rev = Revision {
                id: Uuid::new_v4(),
                name: revision_name.clone(),
                service_name: name.to_owned(),
                namespace: namespace.to_owned(),
                spec: RevisionSpec {
                    container: tmpl.container.clone(),
                    scale: tmpl.scale.clone(),
                    service_account: tmpl.service_account.clone(),
                    timeout_seconds: tmpl.timeout_seconds,
                },
                status: RevisionStatus::Active,
                current_replicas: 1,
                desired_replicas: 1,
                created_at: Utc::now(),
            };
            self.revisions.insert(Self::ns_key(namespace, &revision_name), new_rev);
            svc.spec.template = tmpl;
            svc.latest_created_revision = Some(revision_name.clone());
            svc.latest_ready_revision = Some(revision_name);
        }
        if let Some(traffic) = req.traffic {
            svc.spec.traffic = traffic;
        }
        svc.updated_at = Utc::now();
        self.services.insert(key, svc.clone());
        Ok(svc)
    }

    pub fn delete_service(&self, namespace: &str, name: &str) -> KnativeResult<()> {
        let key = Self::ns_key(namespace, name);
        self.services.remove(&key).ok_or_else(|| KnativeError::ServiceNotFound(key))?;
        self.revisions.retain(|_, rev| !(rev.service_name == name && rev.namespace == namespace));
        self.routes.remove(&Self::ns_key(namespace, name));
        Ok(())
    }

    pub fn get_revision(&self, namespace: &str, revision_name: &str) -> KnativeResult<Revision> {
        let key = Self::ns_key(namespace, revision_name);
        self.revisions.get(&key).map(|r| r.clone()).ok_or_else(|| KnativeError::RevisionNotFound {
            service: "unknown".into(),
            revision: revision_name.to_owned(),
        })
    }

    pub fn list_revisions_for_service(&self, namespace: &str, service_name: &str) -> Vec<Revision> {
        self.revisions.iter()
            .filter(|r| r.value().service_name == service_name && r.value().namespace == namespace)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn delete_revision(&self, namespace: &str, revision_name: &str) -> KnativeResult<()> {
        let key = Self::ns_key(namespace, revision_name);
        self.revisions.remove(&key).ok_or_else(|| KnativeError::RevisionNotFound {
            service: "unknown".into(),
            revision: revision_name.to_owned(),
        })?;
        Ok(())
    }

    pub fn scale_revision(&self, namespace: &str, revision_name: &str, replicas: u32) -> KnativeResult<Revision> {
        let key = Self::ns_key(namespace, revision_name);
        let mut rev = self.revisions.get(&key).map(|r| r.clone()).ok_or_else(|| KnativeError::RevisionNotFound {
            service: "unknown".into(),
            revision: revision_name.to_owned(),
        })?;
        rev.desired_replicas = replicas;
        rev.current_replicas = replicas;
        rev.status = if replicas == 0 { RevisionStatus::Reserve } else { RevisionStatus::Active };
        self.revisions.insert(key, rev.clone());
        info!(namespace, revision = revision_name, replicas, "revision scaled");
        Ok(rev)
    }

    pub fn get_route(&self, namespace: &str, name: &str) -> KnativeResult<KnRoute> {
        let key = Self::ns_key(namespace, name);
        self.routes.get(&key).map(|r| r.clone()).ok_or_else(|| KnativeError::RouteNotFound(key))
    }

    pub fn list_routes(&self, namespace: &str) -> Vec<KnRoute> {
        self.routes.iter().filter(|r| r.value().namespace == namespace).map(|r| r.value().clone()).collect()
    }
}

impl Default for ServingStore {
    fn default() -> Self {
        Self::new()
    }
}
