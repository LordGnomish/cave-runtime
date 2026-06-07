// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! MongoDB → PostgreSQL SQL translation (FerretDB-style clean-room).
//!
//! This is the heart of the cave-docdb *hybrid* strategy: the MongoDB
//! wire-protocol surface (OP_MSG / BSON, see [`crate::wire`]) is paired with a
//! PostgreSQL storage backend by translating Mongo query documents and
//! aggregation pipelines into parameterised SQL over a JSONB document column.
//!
//! Storage model (matches FerretDB v1 `internal/handlers/pg`): every collection
//! is a table with a single `_jsonb jsonb` column holding the document. Query
//! operators become expressions over `_jsonb` using PostgreSQL's native JSONB
//! operators (`->`, `->>`, `#>`, `?`, `@>`, `jsonb_array_length`, ...).
//!
//! All emitted SQL is **parameterised** (`$1`, `$2`, ...) — user values never
//! appear inline, which is both injection-safe and lets the driver bind types.
//! Field names are SQL-string-literal escaped; table identifiers are quoted.

// RED stub — replaced with the real translator in the GREEN commit.

use serde_json::Value;

/// A `WHERE` clause fragment plus its ordered bind parameters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WhereClause {
    /// SQL boolean expression (no leading `WHERE`). Empty string == match-all.
    pub sql: String,
    /// Ordered bind parameters, textual; `$N` placeholders index into this.
    pub params: Vec<String>,
}

/// Translate a MongoDB filter document into a parameterised SQL `WHERE` clause.
pub fn filter_to_where(_filter: &Value) -> WhereClause {
    WhereClause::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wc(filter: serde_json::Value) -> WhereClause {
        filter_to_where(&filter)
    }

    #[test]
    fn eq_scalar_becomes_jsonb_equality() {
        let w = wc(json!({"status": "active"}));
        assert_eq!(w.sql, "_jsonb -> 'status' = $1::jsonb");
        assert_eq!(w.params, vec!["\"active\""]);
    }

    #[test]
    fn comparison_operator_gt() {
        let w = wc(json!({"age": {"$gt": 21}}));
        assert_eq!(w.sql, "_jsonb -> 'age' > $1::jsonb");
        assert_eq!(w.params, vec!["21"]);
    }

    #[test]
    fn ne_uses_is_distinct_from() {
        let w = wc(json!({"s": {"$ne": "x"}}));
        assert_eq!(w.sql, "_jsonb -> 's' IS DISTINCT FROM $1::jsonb");
        assert_eq!(w.params, vec!["\"x\""]);
    }

    #[test]
    fn in_operator_expands_placeholders() {
        let w = wc(json!({"x": {"$in": [1, 2]}}));
        assert_eq!(w.sql, "_jsonb -> 'x' IN ($1::jsonb, $2::jsonb)");
        assert_eq!(w.params, vec!["1", "2"]);
    }

    #[test]
    fn nin_handles_missing_field() {
        let w = wc(json!({"x": {"$nin": [1, 2]}}));
        assert_eq!(
            w.sql,
            "(NOT (_jsonb ? 'x') OR _jsonb -> 'x' NOT IN ($1::jsonb, $2::jsonb))"
        );
        assert_eq!(w.params, vec!["1", "2"]);
    }

    #[test]
    fn exists_true_and_false() {
        assert_eq!(wc(json!({"e": {"$exists": true}})).sql, "_jsonb ? 'e'");
        assert_eq!(
            wc(json!({"e": {"$exists": false}})).sql,
            "NOT (_jsonb ? 'e')"
        );
    }

    #[test]
    fn implicit_and_sorts_fields_deterministically() {
        let w = wc(json!({"b": 2, "a": 1}));
        assert_eq!(
            w.sql,
            "_jsonb -> 'a' = $1::jsonb AND _jsonb -> 'b' = $2::jsonb"
        );
        assert_eq!(w.params, vec!["1", "2"]);
    }

    #[test]
    fn explicit_or() {
        let w = wc(json!({"$or": [{"a": 1}, {"b": 2}]}));
        assert_eq!(
            w.sql,
            "(_jsonb -> 'a' = $1::jsonb OR _jsonb -> 'b' = $2::jsonb)"
        );
    }

    #[test]
    fn nor_negates_or() {
        let w = wc(json!({"$nor": [{"a": 1}, {"b": 2}]}));
        assert_eq!(
            w.sql,
            "NOT (_jsonb -> 'a' = $1::jsonb OR _jsonb -> 'b' = $2::jsonb)"
        );
    }

    #[test]
    fn regex_with_case_insensitive_option() {
        let w = wc(json!({"n": {"$regex": "^a", "$options": "i"}}));
        assert_eq!(w.sql, "_jsonb ->> 'n' ~* $1");
        assert_eq!(w.params, vec!["^a"]);
    }

    #[test]
    fn dotted_path_uses_hash_arrow() {
        let w = wc(json!({"a.b": {"$gt": 5}}));
        assert_eq!(w.sql, "_jsonb #> '{a,b}' > $1::jsonb");
    }

    #[test]
    fn size_operator() {
        let w = wc(json!({"t": {"$size": 3}}));
        assert_eq!(w.sql, "jsonb_array_length(_jsonb -> 't') = $1::int");
        assert_eq!(w.params, vec!["3"]);
    }

    #[test]
    fn not_operator_negates_inner() {
        let w = wc(json!({"x": {"$not": {"$gt": 5}}}));
        assert_eq!(w.sql, "NOT (_jsonb -> 'x' > $1::jsonb)");
    }

    #[test]
    fn multiple_operators_on_one_field_anded() {
        let w = wc(json!({"age": {"$gt": 1, "$lt": 10}}));
        assert_eq!(
            w.sql,
            "_jsonb -> 'age' > $1::jsonb AND _jsonb -> 'age' < $2::jsonb"
        );
    }

    #[test]
    fn all_operator_uses_containment() {
        let w = wc(json!({"tags": {"$all": ["a", "b"]}}));
        assert_eq!(w.sql, "_jsonb -> 'tags' @> $1::jsonb");
        assert_eq!(w.params, vec!["[\"a\",\"b\"]"]);
    }

    #[test]
    fn mod_operator() {
        let w = wc(json!({"n": {"$mod": [4, 1]}}));
        assert_eq!(
            w.sql,
            "((_jsonb ->> 'n')::numeric % $1::numeric) = $2::numeric"
        );
        assert_eq!(w.params, vec!["4", "1"]);
    }

    #[test]
    fn empty_filter_is_match_all() {
        assert_eq!(wc(json!({})).sql, "");
    }
}
