// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PostgreSQL scaler — scales on the scalar result of a user SQL query.
//! upstream: kedacore/keda v2.16.1 — pkg/scalers/postgresql_scaler.go

use crate::scaler::ScalerTrait;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct PostgreSqlScaler {
    pub tenant_id: String,
    pub target_query_value: f64,
    pub activation_target_query_value: f64,
    pub query: String,
    /// Full libpq connection string — when set it wins over the part fields.
    pub connection: String,
    pub host: String,
    pub port: String,
    pub user_name: String,
    pub db_name: String,
    pub ssl_mode: String,
    pub password: String,
    current: f64,
}

impl PostgreSqlScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            target_query_value: 0.0,
            activation_target_query_value: 0.0,
            query: String::new(),
            connection: String::new(),
            host: String::new(),
            port: String::new(),
            user_name: String::new(),
            db_name: String::new(),
            ssl_mode: String::new(),
            password: String::new(),
            current: 0.0,
        }
    }

    /// Port of `buildConnArray` — the ordered libpq key=value parameters.
    /// Password is appended last so it mirrors the upstream array shape.
    pub fn build_conn_array(&self) -> Vec<String> {
        let mut params = Vec::new();
        params.push(format!("host={}", escape_connection_parameter(&self.host)));
        params.push(format!("port={}", escape_connection_parameter(&self.port)));
        params.push(format!("user={}", escape_connection_parameter(&self.user_name)));
        params.push(format!("dbname={}", escape_connection_parameter(&self.db_name)));
        params.push(format!("sslmode={}", escape_connection_parameter(&self.ssl_mode)));
        params.push(format!(
            "password={}",
            escape_connection_parameter(&self.password)
        ));
        params
    }

    /// The connection string KEDA hands to the driver — explicit `connection`
    /// when provided, otherwise the space-joined `buildConnArray`.
    pub fn connection_string(&self) -> String {
        if !self.connection.is_empty() {
            return self.connection.clone();
        }
        self.build_conn_array().join(" ")
    }

    /// Record the scalar the query returned (`getActiveNumber` result).
    pub fn observe(&mut self, query_result: f64) {
        self.current = query_result;
    }
}

/// Port of `escapePostgreConnectionParameter`: leave space-free values bare,
/// otherwise backslash-escape single quotes and wrap the value in quotes.
pub fn escape_connection_parameter(s: &str) -> String {
    // RED placeholder — real escaping added in GREEN step.
    s.to_string()
}

impl ScalerTrait for PostgreSqlScaler {
    fn metric_value(&self) -> Option<f64> {
        Some(self.current)
    }

    // GetMetricsAndActivity: isActive = num > activationTargetQueryValue.
    fn is_active(&self) -> bool {
        self.current > self.activation_target_query_value
    }

    fn activation_threshold(&self) -> f64 {
        self.activation_target_query_value
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_conn_array_orders_libpq_params() {
        let mut s = PostgreSqlScaler::new("t");
        s.host = "db".into();
        s.port = "5432".into();
        s.user_name = "postgres".into();
        s.db_name = "test".into();
        s.ssl_mode = "disable".into();
        s.password = "hunter2".into();
        assert_eq!(
            s.build_conn_array(),
            vec![
                "host=db",
                "port=5432",
                "user=postgres",
                "dbname=test",
                "sslmode=disable",
                "password=hunter2",
            ]
        );
    }

    #[test]
    fn escape_passes_through_space_free_values() {
        assert_eq!(escape_connection_parameter("postgres"), "postgres");
        assert_eq!(escape_connection_parameter("5432"), "5432");
    }

    #[test]
    fn escape_quotes_and_wraps_values_with_spaces() {
        assert_eq!(
            escape_connection_parameter("my db"),
            "'my db'"
        );
        // single quote inside a spaced value gets backslash-escaped
        assert_eq!(
            escape_connection_parameter("a 'b'"),
            "'a \\'b\\''"
        );
    }

    #[test]
    fn connection_string_prefers_explicit_connection() {
        let mut s = PostgreSqlScaler::new("t");
        s.connection = "postgres://u:p@h:5432/db".into();
        s.host = "ignored".into();
        assert_eq!(s.connection_string(), "postgres://u:p@h:5432/db");
    }

    #[test]
    fn connection_string_falls_back_to_built_array() {
        let mut s = PostgreSqlScaler::new("t");
        s.host = "h".into();
        s.port = "5432".into();
        s.user_name = "u".into();
        s.db_name = "d".into();
        s.ssl_mode = "require".into();
        s.password = "p".into();
        assert_eq!(
            s.connection_string(),
            "host=h port=5432 user=u dbname=d sslmode=require password=p"
        );
    }

    #[test]
    fn active_when_query_result_exceeds_activation() {
        let mut s = PostgreSqlScaler::new("t");
        s.activation_target_query_value = 5.0;
        s.observe(5.0);
        assert!(!s.is_active()); // strictly greater, not >=
        s.observe(6.0);
        assert!(s.is_active());
        assert_eq!(s.metric_value(), Some(6.0));
    }

    #[test]
    fn default_activation_zero_inactive_at_zero() {
        let s = PostgreSqlScaler::new("t");
        assert!(!s.is_active());
        assert_eq!(s.activation_threshold(), 0.0);
    }
}
