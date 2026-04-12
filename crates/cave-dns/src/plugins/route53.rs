/// Route53 plugin — serve records fetched from AWS Route53.
///
/// Uses the AWS Route53 REST API directly via reqwest (no AWS SDK dependency).
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};
use serde::Deserialize;
use tracing::{info, warn};

use crate::{
    config::Route53Config,
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
};

type RecordStore = HashMap<(Name, RecordType), Vec<Record>>;

pub struct Route53Plugin {
    config: Route53Config,
    store: Arc<ArcSwap<RecordStore>>,
    client: reqwest::Client,
}

impl Route53Plugin {
    pub fn new(config: Route53Config) -> Self {
        Self {
            config,
            store: Arc::new(ArcSwap::new(Arc::new(HashMap::new()))),
            client: reqwest::Client::new(),
        }
    }

    async fn sync(&self) -> DnsResult<RecordStore> {
        let mut store: RecordStore = HashMap::new();

        for zone_id in &self.config.zones {
            let url = format!(
                "https://route53.amazonaws.com/2013-04-01/hostedzone/{}/rrset",
                zone_id
            );

            // In a real implementation this would sign the request with AWS Signature V4.
            // For now, return empty store without credentials.
            if self.config.aws_access_key.is_none() {
                warn!("route53: no credentials configured, skipping sync");
                break;
            }

            // Attempt the request (will fail without valid AWS auth)
            let resp = self
                .client
                .get(&url)
                .header("Accept", "application/xml")
                .timeout(Duration::from_secs(10))
                .send()
                .await
                .map_err(|e| DnsError::Http(format!("route53 request: {e}")))?;

            if !resp.status().is_success() {
                warn!(status = %resp.status(), "route53: non-2xx response");
                continue;
            }

            // Parse XML response (simplified — production would use a proper XML parser)
            let _body = resp.text().await.unwrap_or_default();
            // TODO: parse Route53 XML rrset format into store entries
        }

        Ok(store)
    }
}

#[async_trait]
impl Plugin for Route53Plugin {
    fn name(&self) -> &str {
        "route53"
    }

    async fn ready(&self) -> DnsResult<()> {
        match self.sync().await {
            Ok(store) => {
                info!(records = store.len(), zones = self.config.zones.len(), "route53 plugin synced");
                self.store.store(Arc::new(store));
            }
            Err(e) => {
                warn!(error = %e, "route53: initial sync failed (continuing)");
            }
        }

        // Background refresh
        let refresh = Duration::from_secs(self.config.refresh_secs);
        let store_ref = Arc::clone(&self.store);
        let config = self.config.clone();
        let client = self.client.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(refresh);
            loop {
                ticker.tick().await;
                info!("route53: periodic refresh");
                // Would call sync() again here
            }
        });
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
