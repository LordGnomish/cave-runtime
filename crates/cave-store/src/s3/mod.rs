// SPDX-License-Identifier: AGPL-3.0-or-later
//! S3/MinIO compatible object storage implementation.

pub mod encryption;
pub mod lifecycle;
pub mod notification;
pub mod policy;
pub mod presigned;
pub mod routes;
pub mod store;
pub mod types;
pub mod xml;

pub use routes::s3_router;
pub use store::ObjectStore;
pub use types::S3Event;
