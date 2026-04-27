//! S3 API surface — request/response types + SigV4 helper.

pub mod presign;
pub mod sigv4;

pub use sigv4::{SigV4Credentials, SigV4Request, sign};
