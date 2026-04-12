//! S3-compatible object storage — store + HTTP router.

pub mod router;
pub mod store;
pub mod types;
#[cfg(test)]
mod tests;

pub use router::s3_router;
pub use store::S3Store;
pub use types::*;
