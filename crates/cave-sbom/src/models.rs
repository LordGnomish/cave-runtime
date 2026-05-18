// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sbom {
    pub id: Uuid,
    pub name: String,
    pub version: String,
    pub format: SbomFormat,
    pub components: Vec<Component>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SbomFormat {
    CycloneDx,
    Spdx,
    Syft,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub version: String,
    pub purl: Option<String>,
    pub license: Option<String>,
    pub component_type: ComponentType,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ComponentType {
    Library,
    Application,
    Container,
    Device,
    Firmware,
}

#[derive(Debug, Clone)]
pub struct DependencyTree {
    pub root: String,
    pub adjacency: HashMap<String, Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_component(id: &str, ct: ComponentType) -> Component {
        Component {
            id: id.to_string(),
            name: id.to_string(),
            version: "1.0.0".to_string(),
            purl: None,
            license: None,
            component_type: ct,
            dependencies: vec![],
        }
    }

    #[test]
    fn test_sbom_format_serde() {
        assert_eq!(
            serde_json::to_string(&SbomFormat::CycloneDx).unwrap(),
            "\"cyclone_dx\""
        );
        assert_eq!(serde_json::to_string(&SbomFormat::Spdx).unwrap(), "\"spdx\"");
        assert_eq!(serde_json::to_string(&SbomFormat::Syft).unwrap(), "\"syft\"");
    }

    #[test]
    fn test_component_type_serde() {
        assert_eq!(
            serde_json::to_string(&ComponentType::Library).unwrap(),
            "\"library\""
        );
        assert_eq!(
            serde_json::to_string(&ComponentType::Application).unwrap(),
            "\"application\""
        );
    }

    #[test]
    fn test_component_serde_roundtrip() {
        let comp = Component {
            id: "c1".to_string(),
            name: "lodash".to_string(),
            version: "4.17.21".to_string(),
            purl: Some("pkg:npm/lodash@4.17.21".to_string()),
            license: Some("MIT".to_string()),
            component_type: ComponentType::Library,
            dependencies: vec!["c2".to_string()],
        };
        let json = serde_json::to_string(&comp).unwrap();
        let back: Component = serde_json::from_str(&json).unwrap();
        assert_eq!(comp, back);
    }

    #[test]
    fn test_sbom_serde_roundtrip() {
        let sbom = Sbom {
            id: Uuid::new_v4(),
            name: "my-app".to_string(),
            version: "1.0.0".to_string(),
            format: SbomFormat::CycloneDx,
            components: vec![make_component("c1", ComponentType::Library)],
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&sbom).unwrap();
        let back: Sbom = serde_json::from_str(&json).unwrap();
        assert_eq!(sbom, back);
    }

    #[test]
    fn test_component_no_purl_or_license() {
        let comp = make_component("bare", ComponentType::Container);
        let json = serde_json::to_string(&comp).unwrap();
        let back: Component = serde_json::from_str(&json).unwrap();
        assert_eq!(comp, back);
        assert!(back.purl.is_none());
        assert!(back.license.is_none());
    }

    #[test]
    fn test_component_with_multiple_deps() {
        let mut comp = make_component("root", ComponentType::Application);
        comp.dependencies = vec!["dep1".to_string(), "dep2".to_string(), "dep3".to_string()];
        let json = serde_json::to_string(&comp).unwrap();
        let back: Component = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dependencies.len(), 3);
    }

    #[test]
    fn test_sbom_format_deserialization() {
        let f: SbomFormat = serde_json::from_str("\"syft\"").unwrap();
        assert_eq!(f, SbomFormat::Syft);
    }

    #[test]
    fn test_component_type_all_variants() {
        for ct in [
            ComponentType::Library,
            ComponentType::Application,
            ComponentType::Container,
            ComponentType::Device,
            ComponentType::Firmware,
        ] {
            let json = serde_json::to_string(&ct).unwrap();
            let back: ComponentType = serde_json::from_str(&json).unwrap();
            assert_eq!(ct, back);
        }
    }
}
