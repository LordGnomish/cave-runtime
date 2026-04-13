//! External identity provider adapters.
//!
//! Each module implements [`crate::provider::AuthBackend`] for a specific
//! enterprise IdP. They are stubs today — enterprises fill in their API
//! credentials and the CAVE Runtime routes auth through their existing tooling.

pub mod auth0;
pub mod entra;
pub mod okta_adapter;
