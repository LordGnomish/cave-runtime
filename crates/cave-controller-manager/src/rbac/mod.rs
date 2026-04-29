//! RBAC controllers — `pkg/controller/clusterroleaggregation`.
//!
//! Currently implemented:
//!
//! * [`cluster_role_aggregation`] — composes a parent ClusterRole's
//!   `rules[]` from the rules of every ClusterRole matching its
//!   `aggregationRule.clusterRoleSelectors[]` selector.

pub mod cluster_role_aggregation;
pub mod match_expressions;
