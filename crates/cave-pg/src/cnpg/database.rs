//! `Database` CRD — declarative `CREATE DATABASE` with grants/extensions.

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DatabaseError {
    #[error("invalid name: {0}")]
    InvalidName(String),
    #[error("invalid owner: {0}")]
    InvalidOwner(String),
    #[error("invalid encoding: {0}")]
    InvalidEncoding(String),
}

#[derive(Debug, Clone)]
pub struct DatabaseSpec {
    pub cluster_name: String,
    pub name: String,
    pub owner: String,
    pub encoding: String,
    pub locale: String,
    pub template: String,
    pub allow_connections: bool,
    pub connection_limit: i32,
    pub is_template: bool,
    pub extensions: Vec<String>,
    pub schemas: Vec<String>,
}

impl Default for DatabaseSpec {
    fn default() -> Self {
        Self {
            cluster_name: "cluster".into(),
            name: "app".into(),
            owner: "app".into(),
            encoding: "UTF8".into(),
            locale: "C".into(),
            template: "template1".into(),
            allow_connections: true,
            connection_limit: -1,
            is_template: false,
            extensions: vec![],
            schemas: vec!["public".into()],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Database {
    pub name: String,
    pub spec: DatabaseSpec,
    pub created: bool,
}

impl Database {
    pub fn new(name: &str, spec: DatabaseSpec) -> Self {
        Self {
            name: name.into(),
            spec,
            created: false,
        }
    }

    pub fn validate(&self) -> Result<(), DatabaseError> {
        validate_identifier(&self.spec.name).map_err(|e| DatabaseError::InvalidName(e))?;
        validate_identifier(&self.spec.owner).map_err(|e| DatabaseError::InvalidOwner(e))?;
        match self.spec.encoding.as_str() {
            "UTF8" | "LATIN1" | "SQL_ASCII" | "WIN1252" => {}
            other => return Err(DatabaseError::InvalidEncoding(other.into())),
        }
        Ok(())
    }

    pub fn render_create_sql(&self) -> String {
        let mut sql = format!("CREATE DATABASE \"{}\"", self.spec.name);
        sql.push_str(&format!(" WITH OWNER \"{}\"", self.spec.owner));
        sql.push_str(&format!(" ENCODING '{}'", self.spec.encoding));
        if !self.spec.locale.is_empty() {
            sql.push_str(&format!(" LC_COLLATE '{}' LC_CTYPE '{}'", self.spec.locale, self.spec.locale));
        }
        if !self.spec.template.is_empty() {
            sql.push_str(&format!(" TEMPLATE \"{}\"", self.spec.template));
        }
        sql.push_str(&format!(
            " ALLOW_CONNECTIONS {}",
            if self.spec.allow_connections {
                "true"
            } else {
                "false"
            }
        ));
        if self.spec.connection_limit >= 0 {
            sql.push_str(&format!(" CONNECTION LIMIT {}", self.spec.connection_limit));
        }
        if self.spec.is_template {
            sql.push_str(" IS_TEMPLATE true");
        }
        sql
    }

    pub fn render_extension_sql(&self) -> Vec<String> {
        self.spec
            .extensions
            .iter()
            .map(|e| format!("CREATE EXTENSION IF NOT EXISTS \"{}\"", e))
            .collect()
    }
}

pub fn validate_identifier(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("empty".into());
    }
    if name.len() > 63 {
        return Err("> 63 chars".into());
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err("must start with letter or underscore".into());
    }
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() && ch != '_' && ch != '$' {
            return Err(format!("invalid char '{}'", ch));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_default_spec_succeeds() {
        let d = Database::new("d", DatabaseSpec::default());
        assert!(d.validate().is_ok());
    }

    #[test]
    fn validate_uppercase_encoding_required() {
        let mut d = Database::new("d", DatabaseSpec::default());
        d.spec.encoding = "ascii".into();
        assert!(d.validate().is_err());
    }

    #[test]
    fn validate_invalid_name_errors() {
        let mut d = Database::new("d", DatabaseSpec::default());
        d.spec.name = "1bad".into();
        assert!(matches!(d.validate(), Err(DatabaseError::InvalidName(_))));
    }

    #[test]
    fn validate_invalid_owner_errors() {
        let mut d = Database::new("d", DatabaseSpec::default());
        d.spec.owner = "with space".into();
        assert!(matches!(d.validate(), Err(DatabaseError::InvalidOwner(_))));
    }

    #[test]
    fn validate_too_long_name_errors() {
        let mut d = Database::new("d", DatabaseSpec::default());
        d.spec.name = "a".repeat(64);
        assert!(d.validate().is_err());
    }

    #[test]
    fn render_create_sql_includes_owner_and_encoding() {
        let d = Database::new("d", DatabaseSpec::default());
        let sql = d.render_create_sql();
        assert!(sql.contains("CREATE DATABASE \"app\""));
        assert!(sql.contains("OWNER \"app\""));
        assert!(sql.contains("ENCODING 'UTF8'"));
    }

    #[test]
    fn render_extension_sql_emits_per_ext() {
        let mut d = Database::new("d", DatabaseSpec::default());
        d.spec.extensions = vec!["pg_trgm".into(), "uuid-ossp".into()];
        let cmds = d.render_extension_sql();
        assert_eq!(cmds.len(), 2);
        assert!(cmds[0].contains("\"pg_trgm\""));
    }

    #[test]
    fn render_includes_template_clause() {
        let d = Database::new("d", DatabaseSpec::default());
        let sql = d.render_create_sql();
        assert!(sql.contains("TEMPLATE \"template1\""));
    }

    #[test]
    fn render_includes_connection_limit_when_set() {
        let mut d = Database::new("d", DatabaseSpec::default());
        d.spec.connection_limit = 50;
        let sql = d.render_create_sql();
        assert!(sql.contains("CONNECTION LIMIT 50"));
    }

    #[test]
    fn render_omits_connection_limit_when_unset() {
        let d = Database::new("d", DatabaseSpec::default());
        let sql = d.render_create_sql();
        assert!(!sql.contains("CONNECTION LIMIT"));
    }

    #[test]
    fn validate_identifier_accepts_underscore_start() {
        assert!(validate_identifier("_internal").is_ok());
    }

    #[test]
    fn validate_identifier_rejects_dash() {
        assert!(validate_identifier("foo-bar").is_err());
    }

    #[test]
    fn validate_identifier_accepts_dollar() {
        assert!(validate_identifier("foo$bar").is_ok());
    }
}
