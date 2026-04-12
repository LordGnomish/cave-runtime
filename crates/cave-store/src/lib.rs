//! **cave-store** — embedded etcd-compatible KV store + S3-compatible object storage.
//!
//! ## Architecture
//!
//! ```text
//!   ┌─────────────────────────────────────────────────────────┐
//!   │                    CaveStoreServer                      │
//!   │   ┌─────────────────────┐  ┌───────────────────────┐   │
//!   │   │  etcd v3 gRPC       │  │  S3-compatible HTTP   │   │
//!   │   │  (port 2379)        │  │  (port 9000)          │   │
//!   │   │  KV, Watch, Lease   │  │  Buckets, Objects     │   │
//!   │   │  Auth, Cluster      │  │  Multipart, Versions  │   │
//!   │   └────────┬────────────┘  └──────────┬────────────┘   │
//!   │            │  StorageEngine            │ S3Store        │
//!   │   ┌────────▼────────────┐  ┌──────────▼────────────┐   │
//!   │   │  MVCC + WAL         │  │  File-backed objects  │   │
//!   │   │  (crash-safe)       │  │  + in-memory index    │   │
//!   │   └─────────────────────┘  └───────────────────────┘   │
//!   └─────────────────────────────────────────────────────────┘
//! ```
//!
//! ## etcd API (gRPC)
//! - KV: Put, Range, DeleteRange, Txn, Compact
//! - Watch: streaming watch with filters
//! - Lease: Grant, Revoke, KeepAlive, TimeToLive, Leases
//! - Auth: users, roles, permissions
//! - Cluster: member management
//! - Maintenance: status, defragment, hash
//!
//! ## S3 API (HTTP)
//! - Bucket: Create, Delete, List, Head, Versioning, Lifecycle, Policy, Notifications, ACL
//! - Object: Put, Get, Delete, Head, Copy, List (prefix/delimiter/pagination)
//! - Multipart: Create, UploadPart, Complete, Abort
//! - Presigned URLs
//! - SSE-S3 and SSE-C encryption

pub mod config;
pub mod engine;
pub mod error;
pub mod etcd;
pub mod s3;
pub mod server;

pub use config::StoreConfig;
pub use engine::StorageEngine;
pub use error::{Result, StoreError};
pub use s3::S3Store;
pub use server::CaveStoreServer;
