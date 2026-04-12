//! CAVE Docs Site — GitBook replacement.

pub mod error;
pub mod types;
pub mod renderer;
pub mod store;
pub mod toc;
pub mod search;
pub mod versioning;
pub mod openapi;
pub mod routes;

pub use store::DocsStore;
pub use error::{DocsError, DocsResult};
pub const MODULE_NAME: &str = "docs-site";
