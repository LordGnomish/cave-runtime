// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{Service, ServiceTier};

pub fn filter_by_team<'a>(services: &'a [Service], team: &str) -> Vec<&'a Service> {
    services.iter().filter(|s| s.team == team).collect()
}

pub fn filter_by_tier<'a>(services: &'a [Service], tier: &ServiceTier) -> Vec<&'a Service> {
    services.iter().filter(|s| &s.tier == tier).collect()
}

pub fn search_by_tag<'a>(services: &'a [Service], tag: &str) -> Vec<&'a Service> {
    services
        .iter()
        .filter(|s| s.tags.iter().any(|t| t == tag))
        .collect()
}

pub fn tier1_count(services: &[Service]) -> usize {
    services
        .iter()
        .filter(|s| s.tier == ServiceTier::Tier1)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Service, ServiceTier};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_service(name: &str, team: &str, tier: ServiceTier, tags: Vec<&str>) -> Service {
        Service {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: "A service".to_string(),
            team: team.to_string(),
            tier,
            language: "rust".to_string(),
            repo_url: "https://github.com/example/repo".to_string(),
            tags: tags.into_iter().map(|t| t.to_string()).collect(),
            registered_at: Utc::now(),
        }
    }

    #[test]
    fn test_filter_by_team() {
        let services = vec![
            make_service("svc-a", "platform", ServiceTier::Tier1, vec![]),
            make_service("svc-b", "data", ServiceTier::Tier2, vec![]),
            make_service("svc-c", "platform", ServiceTier::Tier3, vec![]),
        ];
        let platform = filter_by_team(&services, "platform");
        assert_eq!(platform.len(), 2);
        for s in &platform {
            assert_eq!(s.team, "platform");
        }
    }

    #[test]
    fn test_filter_by_tier() {
        let services = vec![
            make_service("svc-a", "platform", ServiceTier::Tier1, vec![]),
            make_service("svc-b", "data", ServiceTier::Tier2, vec![]),
            make_service("svc-c", "platform", ServiceTier::Tier1, vec![]),
        ];
        let tier1 = filter_by_tier(&services, &ServiceTier::Tier1);
        assert_eq!(tier1.len(), 2);
    }

    #[test]
    fn test_search_by_tag_found() {
        let services = vec![
            make_service(
                "svc-a",
                "platform",
                ServiceTier::Tier1,
                vec!["critical", "payments"],
            ),
            make_service("svc-b", "data", ServiceTier::Tier2, vec!["analytics"]),
        ];
        let found = search_by_tag(&services, "payments");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "svc-a");
    }

    #[test]
    fn test_search_by_tag_not_found() {
        let services = vec![make_service(
            "svc-a",
            "platform",
            ServiceTier::Tier1,
            vec!["critical"],
        )];
        let found = search_by_tag(&services, "nonexistent");
        assert!(found.is_empty());
    }

    #[test]
    fn test_tier1_count() {
        let services = vec![
            make_service("svc-a", "platform", ServiceTier::Tier1, vec![]),
            make_service("svc-b", "data", ServiceTier::Tier2, vec![]),
            make_service("svc-c", "platform", ServiceTier::Tier1, vec![]),
            make_service("svc-d", "platform", ServiceTier::Tier3, vec![]),
        ];
        assert_eq!(tier1_count(&services), 2);
    }
}
