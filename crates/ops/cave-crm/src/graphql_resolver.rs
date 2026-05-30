// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! GraphQL query resolver runtime (read path).
//!
//! Twenty auto-generates a resolver pair per object from its
//! ObjectMetadata — `findOne` (singular, `filter` → a single record) and
//! `findMany` (plural, returns an `IConnection<T>` envelope
//! `{ edges:[{ node, cursor }], pageInfo, totalCount }`). See
//! `packages/twenty-server/src/engine/api/graphql/workspace-resolver-builder/`.
//!
//! This module ports the *execution* contract: a hand-rolled parser for the
//! query subset cave-crm serves (root fields + args + nested selection set)
//! and an executor that filters / orders / paginates a record set and
//! projects the requested columns into the Connection envelope. It is the
//! runtime that `graphql_schema.rs` previously only described — closing the
//! `[[partial]] graphql-resolvers` gap.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn people() -> ObjectData {
        ObjectData::new(
            "person",
            "people",
            vec![
                json!({"id": "11111111-1111-1111-1111-111111111111", "first_name": "Ada", "last_name": "Lovelace", "position": 1}),
                json!({"id": "22222222-2222-2222-2222-222222222222", "first_name": "Bob", "last_name": "Smith", "position": 0}),
                json!({"id": "33333333-3333-3333-3333-333333333333", "first_name": "Cleo", "last_name": "Vance", "position": 2}),
            ],
        )
    }

    fn resolver() -> GraphQlResolver {
        GraphQlResolver::new(vec![people()])
    }

    #[test]
    fn parse_rejects_unknown_root_field() {
        let r = resolver();
        let out = r.execute("{ widgets { edges { node { id } } } }");
        // Unknown root field → GraphQL `errors` array, `data` null for it.
        assert!(out["errors"].is_array());
    }

    #[test]
    fn find_many_wraps_connection_envelope() {
        let r = resolver();
        let out = r.execute("{ people { edges { node { id firstName } } totalCount } }");
        let conn = &out["data"]["people"];
        assert_eq!(conn["totalCount"], 3);
        assert!(conn["edges"].is_array());
        assert_eq!(conn["edges"].as_array().unwrap().len(), 3);
        // Projection is camelCase and only the selected columns.
        let node0 = &conn["edges"][0]["node"];
        assert!(node0["id"].is_string());
        assert!(node0["firstName"].is_string());
        assert!(node0["lastName"].is_null()); // not selected → absent/null
        // Every edge carries an opaque cursor.
        assert!(conn["edges"][0]["cursor"].is_string());
    }

    #[test]
    fn find_many_orders_ascending_by_field() {
        let r = resolver();
        let out = r.execute("{ people(orderBy: {position: ASC}) { edges { node { firstName } } } }");
        let edges = out["data"]["people"]["edges"].as_array().unwrap();
        assert_eq!(edges[0]["node"]["firstName"], "Bob"); // position 0
        assert_eq!(edges[1]["node"]["firstName"], "Ada"); // position 1
        assert_eq!(edges[2]["node"]["firstName"], "Cleo"); // position 2
    }

    #[test]
    fn find_many_orders_descending_by_field() {
        let r = resolver();
        let out = r.execute("{ people(orderBy: {position: DESC}) { edges { node { firstName } } } }");
        let edges = out["data"]["people"]["edges"].as_array().unwrap();
        assert_eq!(edges[0]["node"]["firstName"], "Cleo");
        assert_eq!(edges[2]["node"]["firstName"], "Bob");
    }

    #[test]
    fn find_many_filters_by_field_equality() {
        let r = resolver();
        let out = r.execute("{ people(filter: {firstName: \"Ada\"}) { edges { node { id } } totalCount } }");
        assert_eq!(out["data"]["people"]["totalCount"], 1);
    }

    #[test]
    fn find_many_filters_with_ilike_operator() {
        let r = resolver();
        let out = r.execute("{ people(filter: {lastName: {ilike: \"va\"}}) { totalCount } }");
        // "Vance" matches case-insensitively.
        assert_eq!(out["data"]["people"]["totalCount"], 1);
    }

    #[test]
    fn find_many_paginates_first_and_sets_has_next_page() {
        let r = resolver();
        let out = r.execute("{ people(first: 2, orderBy: {position: ASC}) { edges { node { firstName } } pageInfo { hasNextPage hasPreviousPage endCursor } } }");
        let conn = &out["data"]["people"];
        assert_eq!(conn["edges"].as_array().unwrap().len(), 2);
        assert_eq!(conn["pageInfo"]["hasNextPage"], true);
        assert_eq!(conn["pageInfo"]["hasPreviousPage"], false);
        assert!(conn["pageInfo"]["endCursor"].is_string());
    }

    #[test]
    fn find_one_returns_unwrapped_record_by_id_filter() {
        let r = resolver();
        let out = r.execute(
            "{ person(filter: {id: \"22222222-2222-2222-2222-222222222222\"}) { id firstName } }",
        );
        let rec = &out["data"]["person"];
        assert_eq!(rec["firstName"], "Bob");
        // Single record — NOT wrapped in a Connection.
        assert!(rec.get("edges").is_none());
    }

    #[test]
    fn find_one_missing_record_is_null() {
        let r = resolver();
        let out =
            r.execute("{ person(filter: {id: \"00000000-0000-0000-0000-000000000000\"}) { id } }");
        assert!(out["data"]["person"].is_null());
    }

    #[test]
    fn multiple_root_fields_resolve_independently() {
        let r = GraphQlResolver::new(vec![
            people(),
            ObjectData::new(
                "company",
                "companies",
                vec![json!({"id": "aaaaaaaa-0000-0000-0000-000000000000", "name": "Acme"})],
            ),
        ]);
        let out = r.execute("{ people { totalCount } companies { totalCount } }");
        assert_eq!(out["data"]["people"]["totalCount"], 3);
        assert_eq!(out["data"]["companies"]["totalCount"], 1);
    }
}
