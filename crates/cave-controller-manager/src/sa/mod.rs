//! ServiceAccount controllers — `pkg/controller/serviceaccount`.
//!
//! Two controllers in upstream:
//!
//! * [`sa_controller`] — creates the `default` ServiceAccount per namespace
//!   (`serviceaccounts_controller.go`).
//! * [`token_controller`] — materializes
//!   `kubernetes.io/service-account-token` secrets for ServiceAccounts and
//!   manages bound projected tokens (`tokens_controller.go`).

pub mod sa_controller;
pub mod token_controller;
