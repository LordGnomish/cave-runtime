// SPDX-License-Identifier: AGPL-3.0-or-later
//! CaveStoreServer — starts both the etcd gRPC server and S3 HTTP server.

use std::{net::SocketAddr, sync::Arc};

use tokio::time::{interval, Duration};
use tracing::info;

use crate::{
    config::StoreConfig,
    engine::StorageEngine,
    etcd::build_grpc_router,
    s3::S3Store,
};

pub struct CaveStoreServer {
    pub engine: Arc<StorageEngine>,
    pub s3: Arc<S3Store>,
    pub config: StoreConfig,
}

impl CaveStoreServer {
    pub fn new(config: StoreConfig) -> std::io::Result<Self> {
        let engine = Arc::new(StorageEngine::open(&config.data_dir, config.wal_sync)?);
        let s3_dir = config.data_dir.join("s3");
        let s3 = Arc::new(S3Store::new(s3_dir, config.sse_master_key.clone())?);
        Ok(Self { engine, s3, config })
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let etcd_addr: SocketAddr = format!("{}:{}", "0.0.0.0", self.config.etcd_port).parse()?;
        let s3_addr: SocketAddr = format!("{}:{}", self.config.s3_host, self.config.s3_port).parse()?;

        let grpc_router = build_grpc_router(Arc::clone(&self.engine));
        let s3_router = crate::s3::s3_router(Arc::clone(&self.s3));

        // Lease expiry task
        let engine_expiry = Arc::clone(&self.engine);
        let expiry_interval = self.config.lease_check_interval_ms;
        tokio::spawn(async move {
            let mut tick = interval(Duration::from_millis(expiry_interval));
            loop {
                tick.tick().await;
                let expired = engine_expiry.mvcc.write().expire_leases();
                for id in expired {
                    info!("lease {id} expired");
                }
            }
        });

        info!("etcd gRPC listening on {etcd_addr}");
        info!("S3 HTTP listening on {s3_addr}");

        let s3_listener = tokio::net::TcpListener::bind(s3_addr).await?;

        let (grpc_res, s3_res) = tokio::join!(
            grpc_router.serve(etcd_addr),
            axum::serve(s3_listener, s3_router),
        );

        grpc_res?;
        s3_res?;
        Ok(())
    }
}
