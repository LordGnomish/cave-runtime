//! cave-blob — hybrid object-storage runtime.
//!
//! Provides a single object model (`Object`) addressable through two API
//! surfaces:
//!
//! * **S3** — bucket / object / multipart / SigV4 presigned URLs / lifecycle
//!   policies (mirrored from `aws-sdk-s3` and the AWS S3 API reference).
//! * **Azure Blob** — container / blob / block-blob / SAS / immutability
//!   policies (mirrored from `azure-storage-blobs`).
//!
//! Each module is sans-io and unit-testable: the in-memory store under
//! [`store`] backs both APIs, while [`s3::sigv4`] and [`azure::sas`] expose
//! pure signing helpers.

pub mod azure;
pub mod error;
pub mod lifecycle;
pub mod multipart;
pub mod object;
pub mod s3;
pub mod store;
pub mod tenant;
