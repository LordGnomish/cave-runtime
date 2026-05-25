// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! GraphQL schema description (subset).
//!
//! Twenty exposes a fully-generated GraphQL schema (Apollo Server +
//! TypeORM @WorkspaceEntity decorators). The MVP doesn't ship a runtime
//! GraphQL resolver — that's a v0.2 milestone — but the *schema string*
//! is essential for portal/cavectl tooling (codegen, schema diff). We
//! generate the schema from the in-Rust object metadata so portal can
//! consume a single source of truth.
//!
//! Resolver runtime is deferred — see `[[scope_cuts]]` "graphql-resolvers".

use crate::models::{FieldKind, FieldMetadata, ObjectMetadata};

/// Render an object's GraphQL type definition.
pub fn render_type(object: &ObjectMetadata, fields: &[FieldMetadata]) -> String {
    let mut out = String::new();
    out.push_str(&format!("\"\"\"\n{}\n\"\"\"\n", object.label_singular));
    out.push_str(&format!(
        "type {} {{\n",
        to_pascal_case(&object.name_singular)
    ));
    out.push_str("  id: UUID!\n");
    out.push_str("  workspaceId: UUID!\n");
    out.push_str("  createdAt: DateTime!\n");
    out.push_str("  updatedAt: DateTime!\n");
    for f in fields.iter().filter(|f| f.object_metadata_id == object.id) {
        out.push_str(&format!(
            "  {}: {}\n",
            to_camel_case(&f.name),
            graphql_type(f.kind, f.is_nullable)
        ));
    }
    out.push_str("}\n");
    out
}

/// Render the workspace's schema as a single SDL string (Query root +
/// every standard object + every custom object).
pub fn render_workspace_schema(objects: &[ObjectMetadata], fields: &[FieldMetadata]) -> String {
    let mut out = String::from(SCHEMA_PRELUDE);
    out.push_str("\ntype Query {\n");
    for o in objects {
        let pcs = to_pascal_case(&o.name_singular);
        let pcp = to_pascal_case(&o.name_plural);
        out.push_str(&format!(
            "  {}(id: UUID!): {}\n  {}: [{}!]!\n",
            to_camel_case(&o.name_singular),
            pcs,
            to_camel_case(&o.name_plural),
            pcs,
        ));
        let _ = pcp; // not used for the kanban subset
    }
    out.push_str("}\n\n");
    for o in objects {
        out.push_str(&render_type(o, fields));
        out.push('\n');
    }
    out
}

const SCHEMA_PRELUDE: &str = "\
\"\"\"Cave CRM GraphQL schema (auto-generated from ObjectMetadata).\"\"\"
scalar UUID
scalar DateTime
scalar JSON
";

fn graphql_type(kind: FieldKind, nullable: bool) -> String {
    let base = match kind {
        FieldKind::Uuid => "UUID",
        FieldKind::Text | FieldKind::RichText | FieldKind::TsVector => "String",
        FieldKind::Datetime => "DateTime",
        FieldKind::Date => "DateTime",
        FieldKind::Boolean => "Boolean",
        FieldKind::Number | FieldKind::Rating | FieldKind::Probability | FieldKind::Position => {
            "Int"
        }
        FieldKind::Numeric | FieldKind::Currency => "Float",
        FieldKind::Phones
        | FieldKind::Emails
        | FieldKind::FullName
        | FieldKind::Links
        | FieldKind::Address
        | FieldKind::Select
        | FieldKind::MultiSelect
        | FieldKind::Relation
        | FieldKind::Actor
        | FieldKind::Array
        | FieldKind::RawJson => "JSON",
    };
    if nullable {
        base.to_string()
    } else {
        format!("{}!", base)
    }
}

fn to_pascal_case(snake: &str) -> String {
    snake
        .split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn to_camel_case(snake: &str) -> String {
    let mut parts = snake.split('_').filter(|p| !p.is_empty());
    let mut out = String::new();
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    for p in parts {
        let mut c = p.chars();
        if let Some(ch) = c.next() {
            out.push(ch.to_ascii_uppercase());
        }
        out.push_str(c.as_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn pascal_case_handles_snake() {
        assert_eq!(to_pascal_case("calendar_event"), "CalendarEvent");
        assert_eq!(to_pascal_case("person"), "Person");
    }

    #[test]
    fn camel_case_handles_snake() {
        assert_eq!(to_camel_case("calendar_event"), "calendarEvent");
        assert_eq!(to_camel_case("person"), "person");
    }

    #[test]
    fn graphql_type_maps_field_kinds() {
        assert_eq!(graphql_type(FieldKind::Text, true), "String");
        assert_eq!(graphql_type(FieldKind::Text, false), "String!");
        assert_eq!(graphql_type(FieldKind::Currency, true), "Float");
        assert_eq!(graphql_type(FieldKind::Phones, true), "JSON");
    }

    #[test]
    fn render_type_emits_minimum_columns() {
        let ws = Uuid::new_v4();
        let o = ObjectMetadata::new(ws, "person", "people");
        let mut f = FieldMetadata::new(ws, o.id, "first_name", FieldKind::Text);
        f.is_nullable = false;
        let sdl = render_type(&o, &[f]);
        assert!(sdl.contains("type Person"));
        assert!(sdl.contains("  id: UUID!"));
        assert!(sdl.contains("  firstName: String!"));
    }

    #[test]
    fn render_workspace_schema_includes_all_objects() {
        let ws = Uuid::new_v4();
        let objs = ObjectMetadata::standards(ws);
        let sdl = render_workspace_schema(&objs, &[]);
        assert!(sdl.contains("type Person"));
        assert!(sdl.contains("type Opportunity"));
        assert!(sdl.contains("type Query"));
        assert!(sdl.contains("scalar UUID"));
    }
}
