use crate::models::{Component, DependencyTree};
use std::collections::{HashMap, HashSet, VecDeque};

/// Build a dependency tree from a list of components
pub fn build_dependency_tree(components: &[Component], root_id: &str) -> DependencyTree {
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for comp in components {
        adjacency.insert(comp.id.clone(), comp.dependencies.clone());
    }
    DependencyTree {
        root: root_id.to_string(),
        adjacency,
    }
}

/// Find all transitive dependencies of a component (BFS)
pub fn find_transitive_deps(tree: &DependencyTree, component_id: &str) -> Vec<String> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(component_id.to_string());
    visited.insert(component_id.to_string());
    let mut result = vec![];
    while let Some(current) = queue.pop_front() {
        if current != component_id {
            result.push(current.clone());
        }
        if let Some(deps) = tree.adjacency.get(&current) {
            for dep in deps {
                if !visited.contains(dep) {
                    visited.insert(dep.clone());
                    queue.push_back(dep.clone());
                }
            }
        }
    }
    result
}

/// Parse a Package URL (purl) into its parts
/// Format: pkg:type/namespace/name@version or pkg:type/name@version
pub fn parse_purl(purl: &str) -> Option<PurlParts> {
    let without_pkg = purl.strip_prefix("pkg:")?;
    let (type_part, rest) = without_pkg.split_once('/')?;
    let (name_part, version) = rest.rsplit_once('@').unwrap_or((rest, ""));
    let (namespace, name) = if let Some((ns, n)) = name_part.rsplit_once('/') {
        (Some(ns.to_string()), n.to_string())
    } else {
        (None, name_part.to_string())
    };
    Some(PurlParts {
        package_type: type_part.to_string(),
        namespace,
        name,
        version: if version.is_empty() {
            None
        } else {
            Some(version.to_string())
        },
    })
}

#[derive(Debug, PartialEq)]
pub struct PurlParts {
    pub package_type: String,
    pub namespace: Option<String>,
    pub name: String,
    pub version: Option<String>,
}

/// Find all components with a given license
pub fn find_by_license<'a>(components: &'a [Component], license: &str) -> Vec<&'a Component> {
    components
        .iter()
        .filter(|c| c.license.as_deref() == Some(license))
        .collect()
}

/// Count components by type
pub fn count_by_type(components: &[Component]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for comp in components {
        let key = format!("{:?}", comp.component_type).to_lowercase();
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ComponentType;

    fn make_component(id: &str, deps: Vec<&str>, license: Option<&str>, ct: ComponentType) -> Component {
        Component {
            id: id.to_string(),
            name: id.to_string(),
            version: "1.0.0".to_string(),
            purl: None,
            license: license.map(|s| s.to_string()),
            component_type: ct,
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn test_build_dependency_tree() {
        let comps = vec![
            make_component("a", vec!["b", "c"], None, ComponentType::Application),
            make_component("b", vec![], None, ComponentType::Library),
            make_component("c", vec![], None, ComponentType::Library),
        ];
        let tree = build_dependency_tree(&comps, "a");
        assert_eq!(tree.root, "a");
        assert_eq!(tree.adjacency.get("a").unwrap(), &vec!["b".to_string(), "c".to_string()]);
        assert_eq!(tree.adjacency.get("b").unwrap(), &Vec::<String>::new());
    }

    #[test]
    fn test_find_transitive_deps_direct() {
        let comps = vec![
            make_component("a", vec!["b", "c"], None, ComponentType::Application),
            make_component("b", vec![], None, ComponentType::Library),
            make_component("c", vec![], None, ComponentType::Library),
        ];
        let tree = build_dependency_tree(&comps, "a");
        let mut deps = find_transitive_deps(&tree, "a");
        deps.sort();
        assert_eq!(deps, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn test_find_transitive_deps_deep() {
        // A → B → C: transitive includes both B and C
        let comps = vec![
            make_component("a", vec!["b"], None, ComponentType::Application),
            make_component("b", vec!["c"], None, ComponentType::Library),
            make_component("c", vec![], None, ComponentType::Library),
        ];
        let tree = build_dependency_tree(&comps, "a");
        let mut deps = find_transitive_deps(&tree, "a");
        deps.sort();
        assert_eq!(deps, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn test_find_transitive_deps_no_deps() {
        let comps = vec![make_component("leaf", vec![], None, ComponentType::Library)];
        let tree = build_dependency_tree(&comps, "leaf");
        let deps = find_transitive_deps(&tree, "leaf");
        assert!(deps.is_empty());
    }

    #[test]
    fn test_find_transitive_deps_avoids_cycles() {
        // Cycle: A → B → C → A, should not infinite loop
        let comps = vec![
            make_component("a", vec!["b"], None, ComponentType::Library),
            make_component("b", vec!["c"], None, ComponentType::Library),
            make_component("c", vec!["a"], None, ComponentType::Library),
        ];
        let tree = build_dependency_tree(&comps, "a");
        let deps = find_transitive_deps(&tree, "a");
        // Should terminate; B and C should be in results
        assert!(deps.contains(&"b".to_string()));
        assert!(deps.contains(&"c".to_string()));
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_find_transitive_deps_excludes_self() {
        let comps = vec![
            make_component("root", vec!["dep1"], None, ComponentType::Application),
            make_component("dep1", vec![], None, ComponentType::Library),
        ];
        let tree = build_dependency_tree(&comps, "root");
        let deps = find_transitive_deps(&tree, "root");
        assert!(!deps.contains(&"root".to_string()));
    }

    #[test]
    fn test_parse_purl_with_version() {
        let result = parse_purl("pkg:npm/lodash@4.17.21").unwrap();
        assert_eq!(result.package_type, "npm");
        assert_eq!(result.namespace, None);
        assert_eq!(result.name, "lodash");
        assert_eq!(result.version, Some("4.17.21".to_string()));
    }

    #[test]
    fn test_parse_purl_with_namespace() {
        let result = parse_purl("pkg:maven/org.apache/commons@1.0.0").unwrap();
        assert_eq!(result.package_type, "maven");
        assert_eq!(result.namespace, Some("org.apache".to_string()));
        assert_eq!(result.name, "commons");
        assert_eq!(result.version, Some("1.0.0".to_string()));
    }

    #[test]
    fn test_parse_purl_invalid() {
        assert!(parse_purl("not-a-purl").is_none());
        assert!(parse_purl("").is_none());
    }

    #[test]
    fn test_find_by_license() {
        let comps = vec![
            make_component("a", vec![], Some("MIT"), ComponentType::Library),
            make_component("b", vec![], Some("Apache-2.0"), ComponentType::Library),
            make_component("c", vec![], Some("MIT"), ComponentType::Library),
            make_component("d", vec![], None, ComponentType::Library),
        ];
        let mit = find_by_license(&comps, "MIT");
        assert_eq!(mit.len(), 2);
        assert!(mit.iter().all(|c| c.license.as_deref() == Some("MIT")));
    }

    #[test]
    fn test_count_by_type() {
        let comps = vec![
            make_component("a", vec![], None, ComponentType::Library),
            make_component("b", vec![], None, ComponentType::Library),
            make_component("c", vec![], None, ComponentType::Application),
            make_component("d", vec![], None, ComponentType::Container),
        ];
        let counts = count_by_type(&comps);
        assert_eq!(*counts.get("library").unwrap(), 2);
        assert_eq!(*counts.get("application").unwrap(), 1);
        assert_eq!(*counts.get("container").unwrap(), 1);
    }
}
