//! Azure Blob storage API surface — request/response types + SAS helper.

pub mod sas;

pub use sas::{SasToken, SasPermissions, build_account_sas, build_blob_sas};
