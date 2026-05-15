//! PostgreSQL wire protocol (v3) message handling.

pub mod auth;
pub mod codec;
pub mod error;
pub mod messages;
pub mod startup;

pub use auth::AuthMethod;
pub use codec::{
    CANCEL_REQUEST_CODE, DEFAULT_MAX_FRAME_SIZE, PgFrame, PgPhase, PgWireCodec, SSL_REQUEST_CODE,
    StartupKind, classify_startup,
};
pub use error::ErrorResponse;
pub use messages::{BackendMessage, FrontendMessage};
pub use startup::{StartupMessage, SSLRequest};
