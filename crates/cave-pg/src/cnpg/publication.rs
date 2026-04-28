//! `Publication` CRD — logical-replication publication.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationTarget {
    AllTables,
    Tables(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct Publication {
    pub cluster_name: String,
    pub database: String,
    pub name: String,
    pub target: PublicationTarget,
    pub publish: PublicationOps,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationOps {
    pub insert: bool,
    pub update: bool,
    pub delete: bool,
    pub truncate: bool,
}

impl Default for PublicationOps {
    fn default() -> Self {
        Self {
            insert: true,
            update: true,
            delete: true,
            truncate: true,
        }
    }
}

impl Publication {
    pub fn new(database: &str, name: &str, target: PublicationTarget) -> Self {
        Self {
            cluster_name: String::new(),
            database: database.into(),
            name: name.into(),
            target,
            publish: PublicationOps::default(),
        }
    }

    pub fn render_sql(&self) -> String {
        let mut sql = format!("CREATE PUBLICATION \"{}\"", self.name);
        match &self.target {
            PublicationTarget::AllTables => sql.push_str(" FOR ALL TABLES"),
            PublicationTarget::Tables(tables) => {
                let qualified: Vec<String> =
                    tables.iter().map(|t| format!("\"{}\"", t)).collect();
                sql.push_str(&format!(" FOR TABLE {}", qualified.join(", ")));
            }
        }
        let mut publish_parts = Vec::new();
        if self.publish.insert {
            publish_parts.push("insert");
        }
        if self.publish.update {
            publish_parts.push("update");
        }
        if self.publish.delete {
            publish_parts.push("delete");
        }
        if self.publish.truncate {
            publish_parts.push("truncate");
        }
        sql.push_str(&format!(" WITH (publish = '{}')", publish_parts.join(", ")));
        sql
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_for_all_tables() {
        let p = Publication::new("db", "pub1", PublicationTarget::AllTables);
        let sql = p.render_sql();
        assert!(sql.contains("FOR ALL TABLES"));
    }

    #[test]
    fn render_for_specific_tables() {
        let p = Publication::new(
            "db",
            "pub1",
            PublicationTarget::Tables(vec!["users".into(), "orders".into()]),
        );
        let sql = p.render_sql();
        assert!(sql.contains("FOR TABLE \"users\", \"orders\""));
    }

    #[test]
    fn render_includes_publish_options() {
        let p = Publication::new("db", "pub1", PublicationTarget::AllTables);
        let sql = p.render_sql();
        assert!(sql.contains("publish = 'insert, update, delete, truncate'"));
    }

    #[test]
    fn ops_omit_disabled() {
        let mut p = Publication::new("db", "pub1", PublicationTarget::AllTables);
        p.publish.delete = false;
        p.publish.truncate = false;
        let sql = p.render_sql();
        assert!(sql.contains("publish = 'insert, update'"));
    }

    #[test]
    fn name_quoted_with_double_quotes() {
        let p = Publication::new("db", "weird name", PublicationTarget::AllTables);
        let sql = p.render_sql();
        assert!(sql.contains("\"weird name\""));
    }
}
