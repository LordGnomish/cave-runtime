//! System catalogs — pg_catalog and information_schema virtual tables.
//!
//! These are virtual read-only result sets synthesized from the in-memory engine
//! state. Every query against pg_catalog.* or information_schema.* is intercepted
//! and routed here.

use crate::storage::{Database, Engine};
use crate::types::{oid, ColumnDesc, Oid, PgValue, ResultSet};
use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────────────────
// pg_catalog views
// ─────────────────────────────────────────────────────────────────────────────

/// Build a pg_namespace result set.
pub fn pg_namespace(db: &Database) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("oid", oid::OID),
        ColumnDesc::new("nspname", oid::TEXT),
        ColumnDesc::new("nspowner", oid::OID),
        ColumnDesc::new("nspacl", oid::TEXT),
    ];
    let mut rs = ResultSet::new(cols);
    for schema_ref in db.schemas.iter() {
        let schema = schema_ref.read();
        rs.push_row(vec![
            PgValue::Oid(schema.oid),
            PgValue::Text(schema.name.clone()),
            PgValue::Oid(10), // pg_authid OID for postgres
            PgValue::Null,   // nspacl
        ]);
    }
    rs
}

/// Build a pg_class result set (tables + indexes + sequences + views).
pub fn pg_class(db: &Database) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("oid", oid::OID),
        ColumnDesc::new("relname", oid::TEXT),
        ColumnDesc::new("relnamespace", oid::OID),
        ColumnDesc::new("reltype", oid::OID),
        ColumnDesc::new("reloftype", oid::OID),
        ColumnDesc::new("relowner", oid::OID),
        ColumnDesc::new("relam", oid::OID),
        ColumnDesc::new("relfilenode", oid::OID),
        ColumnDesc::new("reltablespace", oid::OID),
        ColumnDesc::new("relpages", oid::INT4),
        ColumnDesc::new("reltuples", oid::FLOAT4),
        ColumnDesc::new("relallvisible", oid::INT4),
        ColumnDesc::new("reltoastrelid", oid::OID),
        ColumnDesc::new("relhasindex", oid::BOOL),
        ColumnDesc::new("relisshared", oid::BOOL),
        ColumnDesc::new("relpersistence", oid::CHAR),
        ColumnDesc::new("relkind", oid::CHAR),
        ColumnDesc::new("relnatts", oid::INT2),
        ColumnDesc::new("relchecks", oid::INT2),
        ColumnDesc::new("relhasrules", oid::BOOL),
        ColumnDesc::new("relhastriggers", oid::BOOL),
        ColumnDesc::new("relhassubclass", oid::BOOL),
        ColumnDesc::new("relrowsecurity", oid::BOOL),
        ColumnDesc::new("relforcerowsecurity", oid::BOOL),
        ColumnDesc::new("relispopulated", oid::BOOL),
        ColumnDesc::new("relreplident", oid::CHAR),
        ColumnDesc::new("relispartition", oid::BOOL),
        ColumnDesc::new("relrewrite", oid::OID),
        ColumnDesc::new("relfrozenxid", oid::OID),
        ColumnDesc::new("relminmxid", oid::OID),
        ColumnDesc::new("relacl", oid::TEXT),
        ColumnDesc::new("reloptions", oid::TEXT),
        ColumnDesc::new("relpartbound", oid::TEXT),
    ];
    let mut rs = ResultSet::new(cols);

    for schema_ref in db.schemas.iter() {
        let schema = schema_ref.read();
        let ns_oid = schema.oid;

        // Tables (relkind = 'r')
        for table_ref in schema.tables.values() {
            let table = table_ref.read();
            let has_index = !schema.indexes_for_table(&table.name).is_empty();
            rs.push_row(vec![
                PgValue::Oid(table.oid),
                PgValue::Text(table.name.clone()),
                PgValue::Oid(ns_oid),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Oid(10),
                PgValue::Oid(403), // btree AM OID
                PgValue::Oid(table.oid),
                PgValue::Oid(0),
                PgValue::Int4(table.relpages as i32),
                PgValue::Float4(table.reltuples as f32),
                PgValue::Int4(0),
                PgValue::Oid(0),
                PgValue::Bool(has_index),
                PgValue::Bool(false),
                PgValue::Char("p".to_string()), // permanent
                PgValue::Char(if table.matview_query.is_some() { "m" } else { "r" }.to_string()),
                PgValue::Int2(table.columns.len() as i16),
                PgValue::Int2(table.constraints.len() as i16),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(true),
                PgValue::Char("d".to_string()), // default replica identity
                PgValue::Bool(false),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
            ]);
        }

        // Views (relkind = 'v')
        for view_ref in schema.views.values() {
            let view = view_ref.read();
            let kind = if view.is_materialized { "m" } else { "v" };
            rs.push_row(vec![
                PgValue::Oid(view.oid),
                PgValue::Text(view.name.clone()),
                PgValue::Oid(ns_oid),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Oid(10),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Int4(0),
                PgValue::Float4(0.0),
                PgValue::Int4(0),
                PgValue::Oid(0),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Char("p".to_string()),
                PgValue::Char(kind.to_string()),
                PgValue::Int2(0),
                PgValue::Int2(0),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(true),
                PgValue::Char("d".to_string()),
                PgValue::Bool(false),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
            ]);
        }

        // Indexes (relkind = 'i')
        for index_ref in schema.indexes.values() {
            let index = index_ref.read();
            rs.push_row(vec![
                PgValue::Oid(index.oid),
                PgValue::Text(index.name.clone()),
                PgValue::Oid(ns_oid),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Oid(10),
                PgValue::Oid(403),
                PgValue::Oid(index.oid),
                PgValue::Oid(0),
                PgValue::Int4(1),
                PgValue::Float4(0.0),
                PgValue::Int4(0),
                PgValue::Oid(0),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Char("p".to_string()),
                PgValue::Char("i".to_string()),
                PgValue::Int2(index.key_columns.len() as i16),
                PgValue::Int2(0),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(true),
                PgValue::Char("n".to_string()),
                PgValue::Bool(false),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
            ]);
        }

        // Sequences (relkind = 'S')
        for seq_ref in schema.sequences.values() {
            let seq = seq_ref.as_ref();
            rs.push_row(vec![
                PgValue::Oid(seq.oid),
                PgValue::Text(seq.name.clone()),
                PgValue::Oid(ns_oid),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Oid(10),
                PgValue::Oid(0),
                PgValue::Oid(seq.oid),
                PgValue::Oid(0),
                PgValue::Int4(1),
                PgValue::Float4(1.0),
                PgValue::Int4(0),
                PgValue::Oid(0),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Char("p".to_string()),
                PgValue::Char("S".to_string()),
                PgValue::Int2(1),
                PgValue::Int2(0),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(false),
                PgValue::Bool(true),
                PgValue::Char("n".to_string()),
                PgValue::Bool(false),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
            ]);
        }
    }
    rs
}

/// Build a pg_attribute result set.
pub fn pg_attribute(db: &Database) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("attrelid", oid::OID),
        ColumnDesc::new("attname", oid::TEXT),
        ColumnDesc::new("atttypid", oid::OID),
        ColumnDesc::new("attstattarget", oid::INT4),
        ColumnDesc::new("attlen", oid::INT2),
        ColumnDesc::new("attnum", oid::INT2),
        ColumnDesc::new("attndims", oid::INT4),
        ColumnDesc::new("attcacheoff", oid::INT4),
        ColumnDesc::new("atttypmod", oid::INT4),
        ColumnDesc::new("attbyval", oid::BOOL),
        ColumnDesc::new("attstorage", oid::CHAR),
        ColumnDesc::new("attalign", oid::CHAR),
        ColumnDesc::new("attnotnull", oid::BOOL),
        ColumnDesc::new("atthasdef", oid::BOOL),
        ColumnDesc::new("atthasmissing", oid::BOOL),
        ColumnDesc::new("attidentity", oid::CHAR),
        ColumnDesc::new("attgenerated", oid::CHAR),
        ColumnDesc::new("attisdropped", oid::BOOL),
        ColumnDesc::new("attislocal", oid::BOOL),
        ColumnDesc::new("attinhcount", oid::INT4),
        ColumnDesc::new("attcollation", oid::OID),
        ColumnDesc::new("attacl", oid::TEXT),
        ColumnDesc::new("attoptions", oid::TEXT),
        ColumnDesc::new("attfdwoptions", oid::TEXT),
        ColumnDesc::new("attmissingval", oid::TEXT),
    ];
    let mut rs = ResultSet::new(cols);

    for schema_ref in db.schemas.iter() {
        let schema = schema_ref.read();
        for table_ref in schema.tables.values() {
            let table = table_ref.read();
            for col in &table.columns {
                let type_size = crate::types::type_name_for_oid(col.type_oid);
                rs.push_row(vec![
                    PgValue::Oid(table.oid),
                    PgValue::Text(col.name.clone()),
                    PgValue::Oid(col.type_oid),
                    PgValue::Int4(-1), // attstattarget default
                    PgValue::Int2(-1), // attlen
                    PgValue::Int2(col.attr_num),
                    PgValue::Int4(0),
                    PgValue::Int4(-1),
                    PgValue::Int4(col.type_modifier),
                    PgValue::Bool(false),
                    PgValue::Char("x".to_string()), // extended storage
                    PgValue::Char("i".to_string()), // int alignment
                    PgValue::Bool(col.not_null),
                    PgValue::Bool(col.default_expr.is_some()),
                    PgValue::Bool(false),
                    PgValue::Char(String::new()),
                    PgValue::Char(String::new()),
                    PgValue::Bool(false),
                    PgValue::Bool(true),
                    PgValue::Int4(0),
                    PgValue::Oid(0),
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                ]);
            }
        }
    }
    rs
}

/// pg_type — a simplified type catalog.
pub fn pg_type(db: &Database) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("oid", oid::OID),
        ColumnDesc::new("typname", oid::TEXT),
        ColumnDesc::new("typnamespace", oid::OID),
        ColumnDesc::new("typowner", oid::OID),
        ColumnDesc::new("typlen", oid::INT2),
        ColumnDesc::new("typbyval", oid::BOOL),
        ColumnDesc::new("typtype", oid::CHAR),
        ColumnDesc::new("typcategory", oid::CHAR),
        ColumnDesc::new("typispreferred", oid::BOOL),
        ColumnDesc::new("typisdefined", oid::BOOL),
        ColumnDesc::new("typdelim", oid::CHAR),
        ColumnDesc::new("typrelid", oid::OID),
        ColumnDesc::new("typelem", oid::OID),
        ColumnDesc::new("typarray", oid::OID),
        ColumnDesc::new("typinput", oid::TEXT),
        ColumnDesc::new("typoutput", oid::TEXT),
        ColumnDesc::new("typreceive", oid::TEXT),
        ColumnDesc::new("typsend", oid::TEXT),
        ColumnDesc::new("typmodin", oid::TEXT),
        ColumnDesc::new("typmodout", oid::TEXT),
        ColumnDesc::new("typanalyze", oid::TEXT),
        ColumnDesc::new("typalign", oid::CHAR),
        ColumnDesc::new("typstorage", oid::CHAR),
        ColumnDesc::new("typnotnull", oid::BOOL),
        ColumnDesc::new("typbasetype", oid::OID),
        ColumnDesc::new("typtypmod", oid::INT4),
        ColumnDesc::new("typndims", oid::INT4),
        ColumnDesc::new("typcollation", oid::OID),
        ColumnDesc::new("typdefaultbin", oid::TEXT),
        ColumnDesc::new("typdefault", oid::TEXT),
        ColumnDesc::new("typacl", oid::TEXT),
    ];

    let mut rs = ResultSet::new(cols);

    // Built-in types
    let builtin_types: &[(Oid, &str, i16, bool, &str, &str)] = &[
        (oid::BOOL, "bool", 1, true, "b", "B"),
        (oid::BYTEA, "bytea", -1, false, "b", "U"),
        (oid::CHAR, "char", 1, true, "b", "Z"),
        (oid::NAME, "name", 64, false, "b", "S"),
        (oid::INT8, "int8", 8, true, "b", "N"),
        (oid::INT2, "int2", 2, true, "b", "N"),
        (oid::INT4, "int4", 4, true, "b", "N"),
        (oid::TEXT, "text", -1, false, "b", "S"),
        (oid::OID, "oid", 4, true, "b", "N"),
        (oid::FLOAT4, "float4", 4, true, "b", "N"),
        (oid::FLOAT8, "float8", 8, true, "b", "N"),
        (oid::BPCHAR, "bpchar", -1, false, "b", "S"),
        (oid::VARCHAR, "varchar", -1, false, "b", "S"),
        (oid::DATE, "date", 4, true, "b", "D"),
        (oid::TIME, "time", 8, true, "b", "D"),
        (oid::TIMESTAMP, "timestamp", 8, true, "b", "D"),
        (oid::TIMESTAMPTZ, "timestamptz", 8, true, "b", "D"),
        (oid::INTERVAL, "interval", 16, false, "b", "T"),
        (oid::NUMERIC, "numeric", -1, false, "b", "N"),
        (oid::UUID, "uuid", 16, false, "b", "U"),
        (oid::JSON, "json", -1, false, "b", "U"),
        (oid::JSONB, "jsonb", -1, false, "b", "U"),
        (oid::INET, "inet", -1, false, "b", "I"),
        (oid::CIDR, "cidr", -1, false, "b", "I"),
        (oid::MACADDR, "macaddr", 6, false, "b", "U"),
        (oid::BIT, "bit", -1, false, "b", "V"),
        (oid::VARBIT, "varbit", -1, false, "b", "V"),
        (oid::XML, "xml", -1, false, "b", "U"),
        (oid::VOID, "void", 4, true, "p", "P"),
        (oid::RECORD, "record", -1, false, "p", "P"),
        (oid::UNKNOWN, "unknown", -2, false, "b", "X"),
    ];

    let pg_catalog_oid = db.schema("pg_catalog")
        .map(|s| s.read().oid)
        .unwrap_or(11);

    for (type_oid, name, typlen, typbyval, typtype, typcategory) in builtin_types {
        let type_array_oid = crate::types::array_oid_for(*type_oid);
        rs.push_row(vec![
            PgValue::Oid(*type_oid),
            PgValue::Text(name.to_string()),
            PgValue::Oid(pg_catalog_oid),
            PgValue::Oid(10),
            PgValue::Int2(*typlen),
            PgValue::Bool(*typbyval),
            PgValue::Char(typtype.to_string()),
            PgValue::Char(typcategory.to_string()),
            PgValue::Bool(false),
            PgValue::Bool(true),
            PgValue::Char(",".to_string()),
            PgValue::Oid(0),
            PgValue::Oid(0),
            PgValue::Oid(type_array_oid),
            PgValue::Text(format!("{name}in")),
            PgValue::Text(format!("{name}out")),
            PgValue::Text(format!("{name}recv")),
            PgValue::Text(format!("{name}send")),
            PgValue::Null,
            PgValue::Null,
            PgValue::Null,
            PgValue::Char("i".to_string()),
            PgValue::Char("p".to_string()),
            PgValue::Bool(false),
            PgValue::Oid(0),
            PgValue::Int4(-1),
            PgValue::Int4(0),
            PgValue::Oid(0),
            PgValue::Null,
            PgValue::Null,
            PgValue::Null,
        ]);
    }

    // User-defined enum types
    for schema_ref in db.schemas.iter() {
        let schema = schema_ref.read();
        for (type_name, _labels) in &schema.enum_types {
            let type_oid = alloc_enum_oid(type_name);
            rs.push_row(vec![
                PgValue::Oid(type_oid),
                PgValue::Text(type_name.clone()),
                PgValue::Oid(schema.oid),
                PgValue::Oid(10),
                PgValue::Int2(4),
                PgValue::Bool(true),
                PgValue::Char("e".to_string()),
                PgValue::Char("E".to_string()),
                PgValue::Bool(false),
                PgValue::Bool(true),
                PgValue::Char(",".to_string()),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Oid(0),
                PgValue::Text("enum_in".to_string()),
                PgValue::Text("enum_out".to_string()),
                PgValue::Text("enum_recv".to_string()),
                PgValue::Text("enum_send".to_string()),
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
                PgValue::Char("i".to_string()),
                PgValue::Char("p".to_string()),
                PgValue::Bool(false),
                PgValue::Oid(0),
                PgValue::Int4(-1),
                PgValue::Int4(0),
                PgValue::Oid(0),
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
            ]);
        }
    }

    rs
}

fn alloc_enum_oid(name: &str) -> Oid {
    // Stable hash-based OID for enum types
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    name.hash(&mut h);
    (h.finish() as u32 % 100000) + crate::types::oid::USER_DEFINED_START
}

/// pg_index
pub fn pg_index(db: &Database) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("indexrelid", oid::OID),
        ColumnDesc::new("indrelid", oid::OID),
        ColumnDesc::new("indnatts", oid::INT2),
        ColumnDesc::new("indnkeyatts", oid::INT2),
        ColumnDesc::new("indisunique", oid::BOOL),
        ColumnDesc::new("indisprimary", oid::BOOL),
        ColumnDesc::new("indisexclusion", oid::BOOL),
        ColumnDesc::new("indimmediate", oid::BOOL),
        ColumnDesc::new("indisclustered", oid::BOOL),
        ColumnDesc::new("indisvalid", oid::BOOL),
        ColumnDesc::new("indcheckxmin", oid::BOOL),
        ColumnDesc::new("indisready", oid::BOOL),
        ColumnDesc::new("indislive", oid::BOOL),
        ColumnDesc::new("indisreplident", oid::BOOL),
        ColumnDesc::new("indkey", oid::TEXT),
        ColumnDesc::new("indcollation", oid::TEXT),
        ColumnDesc::new("indclass", oid::TEXT),
        ColumnDesc::new("indoption", oid::TEXT),
        ColumnDesc::new("indexprs", oid::TEXT),
        ColumnDesc::new("indpred", oid::TEXT),
    ];
    let mut rs = ResultSet::new(cols);

    for schema_ref in db.schemas.iter() {
        let schema = schema_ref.read();
        for index_ref in schema.indexes.values() {
            let index = index_ref.read();
            // Find the table OID
            let table_oid = schema.table(&index.table_name)
                .map(|t| t.read().oid)
                .unwrap_or(0);

            // Build indkey (column numbers)
            let indkey = "0"; // simplified

            rs.push_row(vec![
                PgValue::Oid(index.oid),
                PgValue::Oid(table_oid),
                PgValue::Int2(index.key_columns.len() as i16),
                PgValue::Int2(index.key_columns.len() as i16),
                PgValue::Bool(index.is_unique),
                PgValue::Bool(index.is_primary),
                PgValue::Bool(false),
                PgValue::Bool(true),
                PgValue::Bool(false),
                PgValue::Bool(true),
                PgValue::Bool(false),
                PgValue::Bool(true),
                PgValue::Bool(true),
                PgValue::Bool(false),
                PgValue::Text(indkey.to_string()),
                PgValue::Text("0".to_string()),
                PgValue::Text("0".to_string()),
                PgValue::Text("0".to_string()),
                PgValue::Null,
                index.predicate.as_ref().map(|p| PgValue::Text(p.clone())).unwrap_or(PgValue::Null),
            ]);
        }
    }
    rs
}

/// pg_constraint
pub fn pg_constraint(db: &Database) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("oid", oid::OID),
        ColumnDesc::new("conname", oid::TEXT),
        ColumnDesc::new("connamespace", oid::OID),
        ColumnDesc::new("contype", oid::CHAR),
        ColumnDesc::new("condeferrable", oid::BOOL),
        ColumnDesc::new("condeferred", oid::BOOL),
        ColumnDesc::new("convalidated", oid::BOOL),
        ColumnDesc::new("conrelid", oid::OID),
        ColumnDesc::new("contypid", oid::OID),
        ColumnDesc::new("conindid", oid::OID),
        ColumnDesc::new("conparentid", oid::OID),
        ColumnDesc::new("confrelid", oid::OID),
        ColumnDesc::new("confupdtype", oid::CHAR),
        ColumnDesc::new("confdeltype", oid::CHAR),
        ColumnDesc::new("confmatchtype", oid::CHAR),
        ColumnDesc::new("conislocal", oid::BOOL),
        ColumnDesc::new("coninhcount", oid::INT4),
        ColumnDesc::new("connoinherit", oid::BOOL),
        ColumnDesc::new("conkey", oid::TEXT),
        ColumnDesc::new("confkey", oid::TEXT),
        ColumnDesc::new("conpfeqop", oid::TEXT),
        ColumnDesc::new("conppeqop", oid::TEXT),
        ColumnDesc::new("conffeqop", oid::TEXT),
        ColumnDesc::new("conexclop", oid::TEXT),
        ColumnDesc::new("conbin", oid::TEXT),
    ];
    let mut rs = ResultSet::new(cols);

    use crate::storage::heap::Constraint;
    let mut con_oid_counter: u32 = crate::types::oid::USER_DEFINED_START + 50000;

    for schema_ref in db.schemas.iter() {
        let schema = schema_ref.read();
        for table_ref in schema.tables.values() {
            let table = table_ref.read();
            for constraint in &table.constraints {
                let (contype, conname) = match constraint {
                    Constraint::PrimaryKey { name, .. } => ("p", name.as_str()),
                    Constraint::Unique { name, .. } => ("u", name.as_str()),
                    Constraint::Check { name, .. } => ("c", name.as_str()),
                    Constraint::ForeignKey { name, .. } => ("f", name.as_str()),
                    Constraint::NotNull { name, .. } => ("n", name.as_str()),
                    Constraint::Exclude { name, .. } => ("x", name.as_str()),
                };
                con_oid_counter += 1;
                rs.push_row(vec![
                    PgValue::Oid(con_oid_counter),
                    PgValue::Text(conname.to_string()),
                    PgValue::Oid(schema.oid),
                    PgValue::Char(contype.to_string()),
                    PgValue::Bool(false),
                    PgValue::Bool(false),
                    PgValue::Bool(true),
                    PgValue::Oid(table.oid),
                    PgValue::Oid(0),
                    PgValue::Oid(0),
                    PgValue::Oid(0),
                    PgValue::Oid(0),
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Bool(true),
                    PgValue::Int4(0),
                    PgValue::Bool(false),
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                ]);
            }
        }
    }
    rs
}

/// pg_settings — server configuration parameters.
pub fn pg_settings() -> ResultSet {
    let cols = vec![
        ColumnDesc::new("name", oid::TEXT),
        ColumnDesc::new("setting", oid::TEXT),
        ColumnDesc::new("unit", oid::TEXT),
        ColumnDesc::new("category", oid::TEXT),
        ColumnDesc::new("short_desc", oid::TEXT),
        ColumnDesc::new("extra_desc", oid::TEXT),
        ColumnDesc::new("context", oid::TEXT),
        ColumnDesc::new("vartype", oid::TEXT),
        ColumnDesc::new("source", oid::TEXT),
        ColumnDesc::new("min_val", oid::TEXT),
        ColumnDesc::new("max_val", oid::TEXT),
        ColumnDesc::new("enumvals", oid::TEXT),
        ColumnDesc::new("boot_val", oid::TEXT),
        ColumnDesc::new("reset_val", oid::TEXT),
        ColumnDesc::new("sourcefile", oid::TEXT),
        ColumnDesc::new("sourceline", oid::INT4),
        ColumnDesc::new("pending_restart", oid::BOOL),
    ];
    let mut rs = ResultSet::new(cols);

    let settings: &[(&str, &str, &str)] = &[
        ("server_version", "16.0", "string"),
        ("server_version_num", "160000", "integer"),
        ("server_encoding", "UTF8", "string"),
        ("client_encoding", "UTF8", "string"),
        ("DateStyle", "ISO, MDY", "string"),
        ("IntervalStyle", "postgres", "string"),
        ("TimeZone", "UTC", "string"),
        ("timezone_abbreviations", "Default", "string"),
        ("extra_float_digits", "1", "integer"),
        ("max_connections", "100", "integer"),
        ("work_mem", "4096", "integer"),
        ("maintenance_work_mem", "65536", "integer"),
        ("shared_buffers", "16384", "integer"),
        ("wal_level", "replica", "enum"),
        ("max_wal_senders", "10", "integer"),
        ("standard_conforming_strings", "on", "bool"),
        ("bytea_output", "hex", "enum"),
        ("lc_messages", "en_US.UTF-8", "string"),
        ("lc_monetary", "en_US.UTF-8", "string"),
        ("lc_numeric", "en_US.UTF-8", "string"),
        ("lc_time", "en_US.UTF-8", "string"),
        ("default_transaction_isolation", "read committed", "enum"),
        ("default_transaction_read_only", "off", "bool"),
        ("transaction_isolation", "read committed", "enum"),
        ("transaction_read_only", "off", "bool"),
        ("log_timezone", "UTC", "string"),
        ("application_name", "", "string"),
        ("integer_datetimes", "on", "bool"),
        ("is_superuser", "on", "bool"),
        ("session_authorization", "postgres", "string"),
    ];

    for (name, setting, vartype) in settings {
        rs.push_row(vec![
            PgValue::Text(name.to_string()),
            PgValue::Text(setting.to_string()),
            PgValue::Null,
            PgValue::Text("Preset Options".to_string()),
            PgValue::Text(format!("{name} setting")),
            PgValue::Null,
            PgValue::Text("user".to_string()),
            PgValue::Text(vartype.to_string()),
            PgValue::Text("default".to_string()),
            PgValue::Null,
            PgValue::Null,
            PgValue::Null,
            PgValue::Text(setting.to_string()),
            PgValue::Text(setting.to_string()),
            PgValue::Null,
            PgValue::Null,
            PgValue::Bool(false),
        ]);
    }
    rs
}

// ─────────────────────────────────────────────────────────────────────────────
// information_schema
// ─────────────────────────────────────────────────────────────────────────────

/// information_schema.schemata
pub fn information_schema_schemata(db: &Database) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("catalog_name", oid::TEXT),
        ColumnDesc::new("schema_name", oid::TEXT),
        ColumnDesc::new("schema_owner", oid::TEXT),
        ColumnDesc::new("default_character_set_catalog", oid::TEXT),
        ColumnDesc::new("default_character_set_schema", oid::TEXT),
        ColumnDesc::new("default_character_set_name", oid::TEXT),
        ColumnDesc::new("sql_path", oid::TEXT),
    ];
    let mut rs = ResultSet::new(cols);
    for schema_ref in db.schemas.iter() {
        let schema = schema_ref.read();
        rs.push_row(vec![
            PgValue::Text(db.name.clone()),
            PgValue::Text(schema.name.clone()),
            PgValue::Text(schema.owner.clone()),
            PgValue::Null,
            PgValue::Null,
            PgValue::Null,
            PgValue::Null,
        ]);
    }
    rs
}

/// information_schema.tables
pub fn information_schema_tables(db: &Database) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("table_catalog", oid::TEXT),
        ColumnDesc::new("table_schema", oid::TEXT),
        ColumnDesc::new("table_name", oid::TEXT),
        ColumnDesc::new("table_type", oid::TEXT),
        ColumnDesc::new("self_referencing_column_name", oid::TEXT),
        ColumnDesc::new("reference_generation", oid::TEXT),
        ColumnDesc::new("user_defined_type_catalog", oid::TEXT),
        ColumnDesc::new("user_defined_type_schema", oid::TEXT),
        ColumnDesc::new("user_defined_type_name", oid::TEXT),
        ColumnDesc::new("is_insertable_into", oid::TEXT),
        ColumnDesc::new("is_typed", oid::TEXT),
        ColumnDesc::new("commit_action", oid::TEXT),
    ];
    let mut rs = ResultSet::new(cols);
    for schema_ref in db.schemas.iter() {
        let schema = schema_ref.read();
        for table_ref in schema.tables.values() {
            let table = table_ref.read();
            let table_type = if table.matview_query.is_some() {
                "LOCAL TEMPORARY"
            } else {
                "BASE TABLE"
            };
            rs.push_row(vec![
                PgValue::Text(db.name.clone()),
                PgValue::Text(schema.name.clone()),
                PgValue::Text(table.name.clone()),
                PgValue::Text(table_type.to_string()),
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
                PgValue::Text("YES".to_string()),
                PgValue::Text("NO".to_string()),
                PgValue::Null,
            ]);
        }
        for view_ref in schema.views.values() {
            let view = view_ref.read();
            let table_type = if view.is_materialized { "MATERIALIZED VIEW" } else { "VIEW" };
            rs.push_row(vec![
                PgValue::Text(db.name.clone()),
                PgValue::Text(schema.name.clone()),
                PgValue::Text(view.name.clone()),
                PgValue::Text(table_type.to_string()),
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
                PgValue::Null,
                PgValue::Text("NO".to_string()),
                PgValue::Text("NO".to_string()),
                PgValue::Null,
            ]);
        }
    }
    rs
}

/// information_schema.columns
pub fn information_schema_columns(db: &Database) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("table_catalog", oid::TEXT),
        ColumnDesc::new("table_schema", oid::TEXT),
        ColumnDesc::new("table_name", oid::TEXT),
        ColumnDesc::new("column_name", oid::TEXT),
        ColumnDesc::new("ordinal_position", oid::INT4),
        ColumnDesc::new("column_default", oid::TEXT),
        ColumnDesc::new("is_nullable", oid::TEXT),
        ColumnDesc::new("data_type", oid::TEXT),
        ColumnDesc::new("character_maximum_length", oid::INT4),
        ColumnDesc::new("character_octet_length", oid::INT4),
        ColumnDesc::new("numeric_precision", oid::INT4),
        ColumnDesc::new("numeric_precision_radix", oid::INT4),
        ColumnDesc::new("numeric_scale", oid::INT4),
        ColumnDesc::new("datetime_precision", oid::INT4),
        ColumnDesc::new("interval_type", oid::TEXT),
        ColumnDesc::new("interval_precision", oid::INT4),
        ColumnDesc::new("character_set_catalog", oid::TEXT),
        ColumnDesc::new("character_set_schema", oid::TEXT),
        ColumnDesc::new("character_set_name", oid::TEXT),
        ColumnDesc::new("collation_catalog", oid::TEXT),
        ColumnDesc::new("collation_schema", oid::TEXT),
        ColumnDesc::new("collation_name", oid::TEXT),
        ColumnDesc::new("domain_catalog", oid::TEXT),
        ColumnDesc::new("domain_schema", oid::TEXT),
        ColumnDesc::new("domain_name", oid::TEXT),
        ColumnDesc::new("udt_catalog", oid::TEXT),
        ColumnDesc::new("udt_schema", oid::TEXT),
        ColumnDesc::new("udt_name", oid::TEXT),
        ColumnDesc::new("scope_catalog", oid::TEXT),
        ColumnDesc::new("scope_schema", oid::TEXT),
        ColumnDesc::new("scope_name", oid::TEXT),
        ColumnDesc::new("maximum_cardinality", oid::INT4),
        ColumnDesc::new("dtd_identifier", oid::TEXT),
        ColumnDesc::new("is_self_referencing", oid::TEXT),
        ColumnDesc::new("is_identity", oid::TEXT),
        ColumnDesc::new("identity_generation", oid::TEXT),
        ColumnDesc::new("identity_start", oid::TEXT),
        ColumnDesc::new("identity_increment", oid::TEXT),
        ColumnDesc::new("identity_maximum", oid::TEXT),
        ColumnDesc::new("identity_minimum", oid::TEXT),
        ColumnDesc::new("identity_cycle", oid::TEXT),
        ColumnDesc::new("is_generated", oid::TEXT),
        ColumnDesc::new("generation_expression", oid::TEXT),
        ColumnDesc::new("is_updatable", oid::TEXT),
    ];
    let col_count = cols.len();
    let mut rs = ResultSet::new(cols);

    for schema_ref in db.schemas.iter() {
        let schema = schema_ref.read();
        for table_ref in schema.tables.values() {
            let table = table_ref.read();
            for (i, col) in table.columns.iter().enumerate() {
                let type_name = crate::types::type_name_for_oid(col.type_oid);
                let is_nullable = if col.not_null { "NO" } else { "YES" };
                let char_max_len = match col.type_oid {
                    crate::types::oid::VARCHAR | crate::types::oid::BPCHAR => {
                        if col.type_modifier > 4 {
                            PgValue::Int4(col.type_modifier - 4)
                        } else {
                            PgValue::Null
                        }
                    }
                    _ => PgValue::Null,
                };
                let mut row = vec![
                    PgValue::Text(db.name.clone()),
                    PgValue::Text(schema.name.clone()),
                    PgValue::Text(table.name.clone()),
                    PgValue::Text(col.name.clone()),
                    PgValue::Int4((i + 1) as i32),
                    col.default_expr.as_ref().map(|d| PgValue::Text(d.clone())).unwrap_or(PgValue::Null),
                    PgValue::Text(is_nullable.to_string()),
                    PgValue::Text(type_name.to_string()),
                    char_max_len,
                ];
                // Fill remaining columns with NULL
                while row.len() < col_count {
                    row.push(PgValue::Null);
                }
                // is_updatable
                *row.last_mut().unwrap() = PgValue::Text("YES".to_string());
                rs.push_row(row);
            }
        }
    }
    rs
}

/// information_schema.table_constraints
pub fn information_schema_table_constraints(db: &Database) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("constraint_catalog", oid::TEXT),
        ColumnDesc::new("constraint_schema", oid::TEXT),
        ColumnDesc::new("constraint_name", oid::TEXT),
        ColumnDesc::new("table_catalog", oid::TEXT),
        ColumnDesc::new("table_schema", oid::TEXT),
        ColumnDesc::new("table_name", oid::TEXT),
        ColumnDesc::new("constraint_type", oid::TEXT),
        ColumnDesc::new("is_deferrable", oid::TEXT),
        ColumnDesc::new("initially_deferred", oid::TEXT),
        ColumnDesc::new("enforced", oid::TEXT),
        ColumnDesc::new("nulls_distinct", oid::TEXT),
    ];
    let mut rs = ResultSet::new(cols);

    use crate::storage::heap::Constraint;
    for schema_ref in db.schemas.iter() {
        let schema = schema_ref.read();
        for table_ref in schema.tables.values() {
            let table = table_ref.read();
            for constraint in &table.constraints {
                let (con_type, con_name) = match constraint {
                    Constraint::PrimaryKey { name, .. } => ("PRIMARY KEY", name.as_str()),
                    Constraint::Unique { name, .. } => ("UNIQUE", name.as_str()),
                    Constraint::Check { name, .. } => ("CHECK", name.as_str()),
                    Constraint::ForeignKey { name, .. } => ("FOREIGN KEY", name.as_str()),
                    Constraint::NotNull { name, .. } => ("CHECK", name.as_str()),
                    Constraint::Exclude { name, .. } => ("EXCLUDE", name.as_str()),
                };
                rs.push_row(vec![
                    PgValue::Text(db.name.clone()),
                    PgValue::Text(schema.name.clone()),
                    PgValue::Text(con_name.to_string()),
                    PgValue::Text(db.name.clone()),
                    PgValue::Text(schema.name.clone()),
                    PgValue::Text(table.name.clone()),
                    PgValue::Text(con_type.to_string()),
                    PgValue::Text("NO".to_string()),
                    PgValue::Text("NO".to_string()),
                    PgValue::Text("YES".to_string()),
                    PgValue::Text("YES".to_string()),
                ]);
            }
        }
    }
    rs
}

/// pg_authid / pg_roles — user accounts.
pub fn pg_roles() -> ResultSet {
    let cols = vec![
        ColumnDesc::new("oid", oid::OID),
        ColumnDesc::new("rolname", oid::TEXT),
        ColumnDesc::new("rolsuper", oid::BOOL),
        ColumnDesc::new("rolinherit", oid::BOOL),
        ColumnDesc::new("rolcreaterole", oid::BOOL),
        ColumnDesc::new("rolcreatedb", oid::BOOL),
        ColumnDesc::new("rolcanlogin", oid::BOOL),
        ColumnDesc::new("rolreplication", oid::BOOL),
        ColumnDesc::new("rolbypassrls", oid::BOOL),
        ColumnDesc::new("rolconnlimit", oid::INT4),
        ColumnDesc::new("rolpassword", oid::TEXT),
        ColumnDesc::new("rolvaliduntil", oid::TIMESTAMPTZ),
    ];
    let mut rs = ResultSet::new(cols);
    rs.push_row(vec![
        PgValue::Oid(10),
        PgValue::Text("postgres".to_string()),
        PgValue::Bool(true),
        PgValue::Bool(true),
        PgValue::Bool(true),
        PgValue::Bool(true),
        PgValue::Bool(true),
        PgValue::Bool(true),
        PgValue::Bool(true),
        PgValue::Int4(-1),
        PgValue::Null,
        PgValue::Null,
    ]);
    rs.push_row(vec![
        PgValue::Oid(16384),
        PgValue::Text("cave".to_string()),
        PgValue::Bool(true),
        PgValue::Bool(true),
        PgValue::Bool(true),
        PgValue::Bool(true),
        PgValue::Bool(true),
        PgValue::Bool(false),
        PgValue::Bool(false),
        PgValue::Int4(-1),
        PgValue::Null,
        PgValue::Null,
    ]);
    rs
}

/// pg_database
pub fn pg_database(engine: &Engine) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("oid", oid::OID),
        ColumnDesc::new("datname", oid::TEXT),
        ColumnDesc::new("datdba", oid::OID),
        ColumnDesc::new("encoding", oid::INT4),
        ColumnDesc::new("datlocprovider", oid::CHAR),
        ColumnDesc::new("datistemplate", oid::BOOL),
        ColumnDesc::new("datallowconn", oid::BOOL),
        ColumnDesc::new("datconnlimit", oid::INT4),
        ColumnDesc::new("datfrozenxid", oid::OID),
        ColumnDesc::new("datminmxid", oid::OID),
        ColumnDesc::new("dattablespace", oid::OID),
        ColumnDesc::new("datcollate", oid::TEXT),
        ColumnDesc::new("datctype", oid::TEXT),
        ColumnDesc::new("daticulocale", oid::TEXT),
        ColumnDesc::new("daticurules", oid::TEXT),
        ColumnDesc::new("datcollversion", oid::TEXT),
        ColumnDesc::new("datacl", oid::TEXT),
    ];
    let mut rs = ResultSet::new(cols);
    for db_ref in engine.databases.iter() {
        let db = db_ref.value();
        rs.push_row(vec![
            PgValue::Oid(db.oid),
            PgValue::Text(db.name.clone()),
            PgValue::Oid(10),
            PgValue::Int4(6), // UTF8 encoding number
            PgValue::Char("c".to_string()),
            PgValue::Bool(db.name == "template1"),
            PgValue::Bool(true),
            PgValue::Int4(-1),
            PgValue::Oid(0),
            PgValue::Oid(0),
            PgValue::Oid(1663), // pg_default tablespace
            PgValue::Text(db.collation.clone()),
            PgValue::Text(db.collation.clone()),
            PgValue::Null,
            PgValue::Null,
            PgValue::Null,
            PgValue::Null,
        ]);
    }
    rs
}

/// pg_stat_activity — currently active connections (returns empty for now).
pub fn pg_stat_activity() -> ResultSet {
    let cols = vec![
        ColumnDesc::new("datid", oid::OID),
        ColumnDesc::new("datname", oid::TEXT),
        ColumnDesc::new("pid", oid::INT4),
        ColumnDesc::new("leader_pid", oid::INT4),
        ColumnDesc::new("usesysid", oid::OID),
        ColumnDesc::new("usename", oid::TEXT),
        ColumnDesc::new("application_name", oid::TEXT),
        ColumnDesc::new("client_addr", oid::INET),
        ColumnDesc::new("client_hostname", oid::TEXT),
        ColumnDesc::new("client_port", oid::INT4),
        ColumnDesc::new("backend_start", oid::TIMESTAMPTZ),
        ColumnDesc::new("xact_start", oid::TIMESTAMPTZ),
        ColumnDesc::new("query_start", oid::TIMESTAMPTZ),
        ColumnDesc::new("state_change", oid::TIMESTAMPTZ),
        ColumnDesc::new("wait_event_type", oid::TEXT),
        ColumnDesc::new("wait_event", oid::TEXT),
        ColumnDesc::new("state", oid::TEXT),
        ColumnDesc::new("backend_xid", oid::OID),
        ColumnDesc::new("backend_xmin", oid::OID),
        ColumnDesc::new("query_id", oid::INT8),
        ColumnDesc::new("query", oid::TEXT),
        ColumnDesc::new("backend_type", oid::TEXT),
    ];
    ResultSet::new(cols) // empty — no active connections from catalog perspective
}

/// pg_stats — table statistics for the planner.
pub fn pg_stats(db: &Database) -> ResultSet {
    let cols = vec![
        ColumnDesc::new("schemaname", oid::TEXT),
        ColumnDesc::new("tablename", oid::TEXT),
        ColumnDesc::new("attname", oid::TEXT),
        ColumnDesc::new("inherited", oid::BOOL),
        ColumnDesc::new("null_frac", oid::FLOAT4),
        ColumnDesc::new("avg_width", oid::INT4),
        ColumnDesc::new("n_distinct", oid::FLOAT4),
        ColumnDesc::new("most_common_vals", oid::TEXT),
        ColumnDesc::new("most_common_freqs", oid::TEXT),
        ColumnDesc::new("histogram_bounds", oid::TEXT),
        ColumnDesc::new("correlation", oid::FLOAT4),
        ColumnDesc::new("most_common_elems", oid::TEXT),
        ColumnDesc::new("most_common_elem_freqs", oid::TEXT),
        ColumnDesc::new("elem_count_histogram", oid::TEXT),
    ];
    let mut rs = ResultSet::new(cols);

    for schema_ref in db.schemas.iter() {
        let schema = schema_ref.read();
        for table_ref in schema.tables.values() {
            let table = table_ref.read();
            for col in &table.columns {
                rs.push_row(vec![
                    PgValue::Text(schema.name.clone()),
                    PgValue::Text(table.name.clone()),
                    PgValue::Text(col.name.clone()),
                    PgValue::Bool(false),
                    PgValue::Float4(0.0),
                    PgValue::Int4(4),
                    PgValue::Float4(-1.0), // -1 = unknown/all distinct
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                    PgValue::Null,
                ]);
            }
        }
    }
    rs
}

/// Dispatch a catalog query by (schema, table_name, engine, db).
pub fn query_catalog_table(
    schema_name: &str,
    table_name: &str,
    engine: &Engine,
    db: &Database,
) -> Option<crate::types::ResultSet> {
    match (schema_name, table_name) {
        ("pg_catalog", "pg_namespace") | ("pg_namespace", _) => Some(pg_namespace(db)),
        ("pg_catalog", "pg_class") | ("pg_class", _) => Some(pg_class(db)),
        ("pg_catalog", "pg_attribute") | ("pg_attribute", _) => Some(pg_attribute(db)),
        ("pg_catalog", "pg_type") | ("pg_type", _) => Some(pg_type(db)),
        ("pg_catalog", "pg_index") | ("pg_index", _) => Some(pg_index(db)),
        ("pg_catalog", "pg_constraint") | ("pg_constraint", _) => Some(pg_constraint(db)),
        ("pg_catalog", "pg_settings") | ("pg_settings", _) => Some(pg_settings()),
        ("pg_catalog", "pg_roles") | ("pg_roles", _) => Some(pg_roles()),
        ("pg_catalog", "pg_authid") => Some(pg_roles()),
        ("pg_catalog", "pg_database") | ("pg_database", _) => Some(pg_database(engine)),
        ("pg_catalog", "pg_stat_activity") => Some(pg_stat_activity()),
        ("pg_catalog", "pg_stats") => Some(pg_stats(db)),
        ("information_schema", "schemata") => Some(information_schema_schemata(db)),
        ("information_schema", "tables") => Some(information_schema_tables(db)),
        ("information_schema", "columns") => Some(information_schema_columns(db)),
        ("information_schema", "table_constraints") => Some(information_schema_table_constraints(db)),
        _ => None,
    }
}
