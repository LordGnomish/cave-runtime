//! etcd v3 gRPC API services.

pub mod auth;
pub mod cluster;
pub mod kv;
pub mod lease;
pub mod watch;

/// Re-export generated proto modules.
pub mod proto {
    pub mod mvccpb {
        tonic::include_proto!("mvccpb");
    }
    pub mod authpb {
        tonic::include_proto!("authpb");
    }
    pub mod etcdserverpb {
        tonic::include_proto!("etcdserverpb");
    }
}

use std::sync::Arc;

use tonic::transport::server::Router;
use tonic::transport::Server;

use crate::engine::StorageEngine;

use proto::etcdserverpb::{
    auth_server::AuthServer as AuthSvc,
    cluster_server::ClusterServer as ClusterSvc,
    kv_server::KvServer as KvSvc,
    lease_server::LeaseServer as LeaseSvc,
    maintenance_server::MaintenanceServer as MaintenanceSvc,
    watch_server::WatchServer as WatchSvc,
};

pub fn build_grpc_router(engine: Arc<StorageEngine>) -> Router {
    Server::builder()
        .add_service(KvSvc::new(kv::KvServer::new(Arc::clone(&engine))))
        .add_service(WatchSvc::new(watch::WatchServer::new(Arc::clone(&engine))))
        .add_service(LeaseSvc::new(lease::LeaseServer::new(Arc::clone(&engine))))
        .add_service(AuthSvc::new(auth::AuthServer::new()))
        .add_service(ClusterSvc::new(cluster::ClusterServer::new(Arc::clone(&engine))))
        .add_service(MaintenanceSvc::new(cluster::MaintenanceServer::new(Arc::clone(&engine))))
}
