// SPDX-License-Identifier: AGPL-3.0-or-later
/// Kubernetes plugin — service discovery from the Kubernetes API.
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};
use k8s_openapi::api::core::v1::{Endpoints, Service};
use kube::{Api, Client};
use tracing::{info, warn};

use crate::{
    config::{KubernetesConfig, PodMode},
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
};

/// In-memory DNS records synthesized from k8s resources.
type RecordStore = HashMap<(Name, RecordType), Vec<Record>>;

pub struct KubernetesPlugin {
    config: KubernetesConfig,
    store: Arc<ArcSwap<RecordStore>>,
}

impl KubernetesPlugin {
    pub fn new(config: KubernetesConfig) -> Self {
        Self {
            config,
            store: Arc::new(ArcSwap::new(Arc::new(HashMap::new()))),
        }
    }

    async fn sync(&self) -> DnsResult<()> {
        let client = Client::try_default().await.map_err(|e| {
            DnsError::Kubernetes(format!("cannot connect to kube API: {e}"))
        })?;

        let mut store: RecordStore = HashMap::new();

        for zone in &self.config.zones {
            let ns_list: Vec<String> = if self.config.namespaces.is_empty() {
                vec!["default".into()]
            } else {
                self.config.namespaces.clone()
            };

            for ns in &ns_list {
                self.sync_namespace(&client, ns, zone, &mut store).await?;
            }
        }

        self.store.store(Arc::new(store));
        Ok(())
    }

    async fn sync_namespace(
        &self,
        client: &Client,
        namespace: &str,
        zone: &str,
        store: &mut RecordStore,
    ) -> DnsResult<()> {
        let svc_api: Api<Service> = Api::namespaced(client.clone(), namespace);
        let ep_api: Api<Endpoints> = Api::namespaced(client.clone(), namespace);

        let services = svc_api.list(&Default::default()).await.map_err(|e| {
            DnsError::Kubernetes(format!("list services in {namespace}: {e}"))
        })?;

        for svc in services.items {
            let svc_name = svc
                .metadata
                .name
                .as_deref()
                .unwrap_or("unknown");
            let dns_name = format!("{svc_name}.{namespace}.svc.{zone}");
            let fqdn: Name = match dns_name.parse() {
                Ok(n) => n,
                Err(_) => continue,
            };

            if let Some(spec) = &svc.spec {
                // ExternalName → CNAME
                if let Some(ext) = &spec.external_name {
                    if spec.type_.as_deref() == Some("ExternalName") {
                        let target: Name = match format!("{ext}.").parse() {
                            Ok(n) => n,
                            Err(_) => continue,
                        };
                        let mut r = Record::new();
                        r.set_name(fqdn.clone());
                        r.set_ttl(self.config.ttl);
                        r.set_record_type(RecordType::CNAME);
                        r.set_dns_class(DNSClass::IN);
                        r.set_data(Some(RData::CNAME(hickory_proto::rr::rdata::CNAME(target))));
                        store
                            .entry((fqdn.clone(), RecordType::CNAME))
                            .or_default()
                            .push(r);
                        continue;
                    }
                }

                // ClusterIP service
                if let Some(cluster_ip) = &spec.cluster_ip {
                    if cluster_ip != "None" && !cluster_ip.is_empty() {
                        if let Ok(addr) = cluster_ip.parse::<IpAddr>() {
                            let rdata = match addr {
                                IpAddr::V4(v4) => {
                                    RData::A(hickory_proto::rr::rdata::A(v4))
                                }
                                IpAddr::V6(v6) => {
                                    RData::AAAA(hickory_proto::rr::rdata::AAAA(v6))
                                }
                            };
                            let rtype = if matches!(addr, IpAddr::V4(_)) {
                                RecordType::A
                            } else {
                                RecordType::AAAA
                            };
                            let mut r = Record::new();
                            r.set_name(fqdn.clone());
                            r.set_ttl(self.config.ttl);
                            r.set_record_type(rtype);
                            r.set_dns_class(DNSClass::IN);
                            r.set_data(Some(rdata));
                            store
                                .entry((fqdn.clone(), rtype))
                                .or_default()
                                .push(r);
                        }
                    }
                }

                // Headless service — emit endpoint IPs
                if spec.cluster_ip.as_deref() == Some("None") {
                    if let Ok(eps) = ep_api.get(svc_name).await {
                        for subset in eps.subsets.unwrap_or_default() {
                            for addr in subset.addresses.unwrap_or_default() {
                                if let Ok(ip) = addr.ip.parse::<IpAddr>() {
                                    let (rdata, rtype) = match ip {
                                        IpAddr::V4(v4) => (
                                            RData::A(hickory_proto::rr::rdata::A(v4)),
                                            RecordType::A,
                                        ),
                                        IpAddr::V6(v6) => (
                                            RData::AAAA(hickory_proto::rr::rdata::AAAA(v6)),
                                            RecordType::AAAA,
                                        ),
                                    };
                                    let mut r = Record::new();
                                    r.set_name(fqdn.clone());
                                    r.set_ttl(self.config.ttl);
                                    r.set_record_type(rtype);
                                    r.set_dns_class(DNSClass::IN);
                                    r.set_data(Some(rdata));
                                    store
                                        .entry((fqdn.clone(), rtype))
                                        .or_default()
                                        .push(r);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Plugin for KubernetesPlugin {
    fn name(&self) -> &str {
        "kubernetes"
    }

    async fn ready(&self) -> DnsResult<()> {
        match self.sync().await {
            Ok(()) => info!("kubernetes plugin synced"),
            Err(e) => warn!(error = %e, "kubernetes plugin: initial sync failed (continuing)"),
        }
        Ok(())
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let q = match ctx.request.queries().first() {
            Some(q) => q.clone(),
            None => return next.run(ctx).await,
        };

        let store = self.store.load();
        if let Some(records) = store.get(&(q.name().clone(), q.query_type())) {
            for r in records {
                ctx.response.add_answer(r.clone());
            }
            return Ok(());
        }

        next.run(ctx).await
    }
}
