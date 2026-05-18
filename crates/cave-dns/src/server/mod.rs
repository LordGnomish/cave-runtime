// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod doh;
pub mod dot;
pub mod tcp;
pub mod udp;

use std::sync::Arc;

use tracing::info;

use crate::{
    config::DnsConfig,
    error::{DnsError, DnsResult},
    plugins::PluginChain,
    zone::ZoneManager,
};

/// Top-level DNS server — owns all listeners and the plugin chain.
pub struct DnsServer {
    pub config: Arc<DnsConfig>,
    pub plugins: Arc<PluginChain>,
    pub zones: Arc<ZoneManager>,
}

impl DnsServer {
    pub async fn new(config: DnsConfig, plugins: PluginChain) -> DnsResult<Self> {
        let zones = Arc::new(ZoneManager::new());

        // Load zones from config
        for zone_cfg in &config.zones {
            zones.load_zone(zone_cfg).await?;
        }

        Ok(Self {
            config: Arc::new(config),
            plugins: Arc::new(plugins),
            zones,
        })
    }

    /// Start all listeners and block forever.
    pub async fn run(self) -> DnsResult<()> {
        // Ready all plugins
        self.plugins.ready_all().await?;

        let config = Arc::clone(&self.config);
        let plugins = Arc::clone(&self.plugins);

        let mut handles = Vec::new();

        // UDP listeners
        for addr in &config.listen_udp {
            let a = addr.clone();
            let p = Arc::clone(&plugins);
            info!(addr = %a, "starting UDP listener");
            handles.push(tokio::spawn(async move {
                if let Err(e) = udp::serve(a.clone(), p).await {
                    tracing::error!(addr = %a, error = %e, "UDP listener error");
                }
            }));
        }

        // TCP listeners
        for addr in &config.listen_tcp {
            let a = addr.clone();
            let p = Arc::clone(&plugins);
            info!(addr = %a, "starting TCP listener");
            handles.push(tokio::spawn(async move {
                if let Err(e) = tcp::serve(a.clone(), p).await {
                    tracing::error!(addr = %a, error = %e, "TCP listener error");
                }
            }));
        }

        // DoT listeners
        if !config.dot_listen.is_empty() {
            let cert = config
                .tls_cert_path
                .clone()
                .ok_or_else(|| DnsError::Config("DoT requires tls_cert_path".into()))?;
            let key = config
                .tls_key_path
                .clone()
                .ok_or_else(|| DnsError::Config("DoT requires tls_key_path".into()))?;

            for addr in &config.dot_listen {
                let a = addr.clone();
                let p = Arc::clone(&plugins);
                let c = cert.clone();
                let k = key.clone();
                info!(addr = %a, "starting DoT listener");
                handles.push(tokio::spawn(async move {
                    if let Err(e) = dot::serve(a.clone(), p, c, k).await {
                        tracing::error!(addr = %a, error = %e, "DoT listener error");
                    }
                }));
            }
        }

        // DoH listeners
        for addr in &config.doh_listen {
            let a = addr.clone();
            let p = Arc::clone(&plugins);
            info!(addr = %a, "starting DoH listener");
            handles.push(tokio::spawn(async move {
                if let Err(e) = doh::serve(a.clone(), p).await {
                    tracing::error!(addr = %a, error = %e, "DoH listener error");
                }
            }));
        }

        // Wait for all listeners (they run forever)
        futures::future::join_all(handles).await;
        Ok(())
    }
}
