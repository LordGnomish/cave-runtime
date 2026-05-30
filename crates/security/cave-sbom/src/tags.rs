// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/model/Tag.java
//   src/main/java/alpine/server/json/TrimmedStringDeserializer.java
//
//! Tag model + project tagging. Upstream `Tag.name` validation at this commit:
//! whitespace-trimmed (`TrimmedStringDeserializer`), `@NotBlank`,
//! `@Size(min=1,max=255)`, `@Pattern(PRINTABLE_CHARS)`. NOTE: there is **no**
//! lowercase normalisation at this pin — case is preserved verbatim.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Project;

    #[test]
    fn tag_trims_surrounding_whitespace() {
        let t = Tag::new("  prod  ").unwrap();
        assert_eq!(t.name, "prod");
    }

    #[test]
    fn tag_preserves_case_no_lowercasing() {
        // Grounded: Tag.java @128fd0fa does NOT lowercase. Case is verbatim.
        let t = Tag::new("MyTeam/Backend").unwrap();
        assert_eq!(t.name, "MyTeam/Backend");
    }

    #[test]
    fn tag_rejects_blank() {
        assert_eq!(Tag::new("   "), Err(TagError::Blank));
        assert_eq!(Tag::new(""), Err(TagError::Blank));
    }

    #[test]
    fn tag_rejects_too_long() {
        let long = "a".repeat(256);
        assert_eq!(Tag::new(&long), Err(TagError::TooLong));
        // Exactly 255 is allowed.
        assert!(Tag::new(&"a".repeat(255)).is_ok());
    }

    #[test]
    fn tag_rejects_non_printable() {
        assert_eq!(Tag::new("bad\u{0007}tag"), Err(TagError::NotPrintable));
        assert_eq!(Tag::new("line\nbreak"), Err(TagError::NotPrintable));
    }

    #[test]
    fn tag_equality_is_by_name() {
        assert_eq!(Tag::new("a").unwrap(), Tag::new(" a ").unwrap());
        assert_ne!(Tag::new("a").unwrap(), Tag::new("A").unwrap());
    }

    #[test]
    fn project_add_tag_sorts_ascending_and_dedups() {
        let mut p = Project::new("p", None);
        p.add_tag("zeta").unwrap();
        p.add_tag("alpha").unwrap();
        p.add_tag(" alpha ").unwrap(); // dup after trim
        p.add_tag("mid").unwrap();
        assert_eq!(p.tags, vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn project_add_tag_propagates_validation_error() {
        let mut p = Project::new("p", None);
        assert_eq!(p.add_tag("  "), Err(TagError::Blank));
        assert!(p.tags.is_empty());
    }

    #[test]
    fn projects_with_tag_filters_by_membership() {
        let mut a = Project::new("a", None);
        a.add_tag("frontend").unwrap();
        let mut b = Project::new("b", None);
        b.add_tag("backend").unwrap();
        let mut c = Project::new("c", None);
        c.add_tag("frontend").unwrap();
        let projects = vec![a, b, c];
        let mut names: Vec<&str> = projects_with_tag(&projects, "frontend")
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a", "c"]);
        // Tag match is case-sensitive (no normalisation).
        assert!(projects_with_tag(&projects, "Frontend").is_empty());
    }
}
