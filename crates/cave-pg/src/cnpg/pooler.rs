//! `Pooler` CRD — PgBouncer-style connection pool fronting a Cluster.

use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PoolerError {
    #[error("invalid pool size: {0}")]
    InvalidPoolSize(i32),
    #[error("unknown auth method: {0}")]
    UnknownAuthMethod(String),
    #[error("invalid mode: {0}")]
    InvalidMode(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolerMode {
    Session,
    Transaction,
    Statement,
}

impl PoolerMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Transaction => "transaction",
            Self::Statement => "statement",
        }
    }

    pub fn parse(s: &str) -> Result<Self, PoolerError> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "session" => Self::Session,
            "transaction" => Self::Transaction,
            "statement" => Self::Statement,
            other => return Err(PoolerError::InvalidMode(other.into())),
        })
    }
}

#[derive(Debug, Clone)]
pub struct PoolerSpec {
    pub cluster_name: String,
    pub instances: i32,
    pub pool_mode: PoolerMode,
    pub default_pool_size: i32,
    pub max_client_conn: i32,
    pub min_pool_size: i32,
    pub reserve_pool_size: i32,
    pub max_db_connections: i32,
    pub auth_query_user: String,
    pub auth_method: String,
    pub server_idle_timeout: i32,
    pub server_lifetime: i32,
}

impl Default for PoolerSpec {
    fn default() -> Self {
        Self {
            cluster_name: "cluster".into(),
            instances: 1,
            pool_mode: PoolerMode::Transaction,
            default_pool_size: 20,
            max_client_conn: 100,
            min_pool_size: 0,
            reserve_pool_size: 0,
            max_db_connections: 0,
            auth_query_user: "cnpg_pooler_pgbouncer".into(),
            auth_method: "scram-sha-256".into(),
            server_idle_timeout: 600,
            server_lifetime: 3600,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PoolerStatus {
    pub ready_replicas: i32,
    pub current_connections: i32,
    pub waiting_clients: i32,
    pub server_pool_size: BTreeMap<String, i32>,
}

#[derive(Debug, Clone)]
pub struct Pooler {
    pub name: String,
    pub spec: PoolerSpec,
    pub status: PoolerStatus,
}

impl Pooler {
    pub fn new(name: &str, spec: PoolerSpec) -> Self {
        Self {
            name: name.into(),
            spec,
            status: PoolerStatus::default(),
        }
    }

    pub fn validate(&self) -> Result<(), PoolerError> {
        if self.spec.default_pool_size <= 0 {
            return Err(PoolerError::InvalidPoolSize(self.spec.default_pool_size));
        }
        if self.spec.max_client_conn < self.spec.default_pool_size {
            return Err(PoolerError::InvalidPoolSize(self.spec.max_client_conn));
        }
        match self.spec.auth_method.as_str() {
            "trust" | "md5" | "scram-sha-256" | "cert" => {}
            other => return Err(PoolerError::UnknownAuthMethod(other.into())),
        }
        Ok(())
    }

    pub fn render_pgbouncer_ini(&self) -> String {
        let mut out = String::new();
        out.push_str("[databases]\n");
        out.push_str(&format!(
            "* = host=localhost port=5432 auth_user={}\n\n",
            self.spec.auth_query_user
        ));
        out.push_str("[pgbouncer]\n");
        out.push_str(&format!("listen_addr = *\n"));
        out.push_str("listen_port = 5432\n");
        out.push_str(&format!("auth_type = {}\n", self.spec.auth_method));
        out.push_str(&format!("pool_mode = {}\n", self.spec.pool_mode.as_str()));
        out.push_str(&format!(
            "default_pool_size = {}\n",
            self.spec.default_pool_size
        ));
        out.push_str(&format!("max_client_conn = {}\n", self.spec.max_client_conn));
        if self.spec.min_pool_size > 0 {
            out.push_str(&format!("min_pool_size = {}\n", self.spec.min_pool_size));
        }
        if self.spec.reserve_pool_size > 0 {
            out.push_str(&format!(
                "reserve_pool_size = {}\n",
                self.spec.reserve_pool_size
            ));
        }
        if self.spec.max_db_connections > 0 {
            out.push_str(&format!(
                "max_db_connections = {}\n",
                self.spec.max_db_connections
            ));
        }
        out.push_str(&format!(
            "server_idle_timeout = {}\n",
            self.spec.server_idle_timeout
        ));
        out.push_str(&format!("server_lifetime = {}\n", self.spec.server_lifetime));
        out
    }

    pub fn record_connection(&mut self) {
        self.status.current_connections += 1;
    }

    pub fn release_connection(&mut self) {
        self.status.current_connections = (self.status.current_connections - 1).max(0);
    }

    pub fn is_at_capacity(&self) -> bool {
        self.status.current_connections >= self.spec.max_client_conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_mode_parse_round_trip() {
        for m in [PoolerMode::Session, PoolerMode::Transaction, PoolerMode::Statement] {
            assert_eq!(PoolerMode::parse(m.as_str()).unwrap(), m);
        }
    }

    #[test]
    fn pool_mode_parse_unknown_errors() {
        let err = PoolerMode::parse("hot").unwrap_err();
        assert!(matches!(err, PoolerError::InvalidMode(_)));
    }

    #[test]
    fn validate_default_spec_succeeds() {
        let p = Pooler::new("p", PoolerSpec::default());
        assert!(p.validate().is_ok());
    }

    #[test]
    fn validate_zero_pool_size_errors() {
        let mut p = Pooler::new("p", PoolerSpec::default());
        p.spec.default_pool_size = 0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_max_below_default_errors() {
        let mut p = Pooler::new("p", PoolerSpec::default());
        p.spec.default_pool_size = 100;
        p.spec.max_client_conn = 10;
        assert!(p.validate().is_err());
    }

    #[test]
    fn validate_unknown_auth_errors() {
        let mut p = Pooler::new("p", PoolerSpec::default());
        p.spec.auth_method = "ldap".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn render_pgbouncer_includes_pool_mode() {
        let p = Pooler::new("p", PoolerSpec::default());
        let ini = p.render_pgbouncer_ini();
        assert!(ini.contains("pool_mode = transaction"));
    }

    #[test]
    fn render_pgbouncer_includes_max_client_conn() {
        let p = Pooler::new("p", PoolerSpec::default());
        let ini = p.render_pgbouncer_ini();
        assert!(ini.contains("max_client_conn = 100"));
    }

    #[test]
    fn render_omits_min_pool_when_zero() {
        let p = Pooler::new("p", PoolerSpec::default());
        let ini = p.render_pgbouncer_ini();
        assert!(!ini.contains("min_pool_size"));
    }

    #[test]
    fn render_includes_min_pool_when_set() {
        let mut p = Pooler::new("p", PoolerSpec::default());
        p.spec.min_pool_size = 5;
        let ini = p.render_pgbouncer_ini();
        assert!(ini.contains("min_pool_size = 5"));
    }

    #[test]
    fn record_then_release_returns_to_zero() {
        let mut p = Pooler::new("p", PoolerSpec::default());
        p.record_connection();
        p.record_connection();
        p.release_connection();
        p.release_connection();
        assert_eq!(p.status.current_connections, 0);
    }

    #[test]
    fn release_does_not_go_negative() {
        let mut p = Pooler::new("p", PoolerSpec::default());
        p.release_connection();
        assert_eq!(p.status.current_connections, 0);
    }

    #[test]
    fn is_at_capacity_after_max_conns() {
        let mut p = Pooler::new("p", PoolerSpec::default());
        p.spec.max_client_conn = 2;
        p.record_connection();
        p.record_connection();
        assert!(p.is_at_capacity());
    }
}
