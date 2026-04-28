//! `Subscription` CRD — logical-replication subscription.

#[derive(Debug, Clone)]
pub struct Subscription {
    pub cluster_name: String,
    pub database: String,
    pub name: String,
    pub publication_name: String,
    pub external_cluster_conninfo: String,
    pub copy_data: bool,
    pub create_slot: bool,
    pub enabled: bool,
}

impl Subscription {
    pub fn new(
        database: &str,
        name: &str,
        publication_name: &str,
        external_cluster_conninfo: &str,
    ) -> Self {
        Self {
            cluster_name: String::new(),
            database: database.into(),
            name: name.into(),
            publication_name: publication_name.into(),
            external_cluster_conninfo: external_cluster_conninfo.into(),
            copy_data: true,
            create_slot: true,
            enabled: true,
        }
    }

    pub fn render_sql(&self) -> String {
        let mut sql = format!(
            "CREATE SUBSCRIPTION \"{}\" CONNECTION '{}' PUBLICATION \"{}\"",
            self.name, self.external_cluster_conninfo, self.publication_name
        );
        let mut opts = Vec::new();
        opts.push(format!("copy_data = {}", self.copy_data));
        opts.push(format!("create_slot = {}", self.create_slot));
        opts.push(format!("enabled = {}", self.enabled));
        sql.push_str(&format!(" WITH ({})", opts.join(", ")));
        sql
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_connection_and_publication() {
        let s = Subscription::new("db", "sub1", "pub1", "host=remote port=5432 dbname=src");
        let sql = s.render_sql();
        assert!(sql.contains("CREATE SUBSCRIPTION \"sub1\""));
        assert!(sql.contains("PUBLICATION \"pub1\""));
        assert!(sql.contains("host=remote"));
    }

    #[test]
    fn render_includes_copy_data_flag() {
        let s = Subscription::new("db", "s", "p", "conninfo");
        let sql = s.render_sql();
        assert!(sql.contains("copy_data = true"));
    }

    #[test]
    fn render_disables_copy_data() {
        let mut s = Subscription::new("db", "s", "p", "conninfo");
        s.copy_data = false;
        let sql = s.render_sql();
        assert!(sql.contains("copy_data = false"));
    }

    #[test]
    fn render_includes_enabled_flag() {
        let mut s = Subscription::new("db", "s", "p", "conninfo");
        s.enabled = false;
        let sql = s.render_sql();
        assert!(sql.contains("enabled = false"));
    }

    #[test]
    fn render_includes_create_slot_flag() {
        let mut s = Subscription::new("db", "s", "p", "conninfo");
        s.create_slot = false;
        let sql = s.render_sql();
        assert!(sql.contains("create_slot = false"));
    }
}
