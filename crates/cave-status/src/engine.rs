//! Worst-status aggregator for status pages.
//!
//! Computes the overall status of a page by taking the highest-rank
//! component status. Empty inputs resolve to Operational.

use crate::models::{ComponentStatus, StatusComponent, StatusPage};

/// Compute overall status as the worst status among all components.
///
/// If the input slice is empty, returns `ComponentStatus::Operational`.
/// Otherwise, iterates over all components, maps each to a numeric rank,
/// and returns the status corresponding to the maximum rank found.
///
/// # Ranks
///
/// * `0` - Operational
/// * `1` - DegradedPerformance
/// * `2` - PartialOutage
/// * `3` - UnderMaintenance
/// * `4` - MajorOutage
pub fn compute_overall_status(components: &[StatusComponent]) -> ComponentStatus {
    let worst = components.iter().map(|c| status_rank(&c.status)).max().unwrap_or(0);
    rank_to_status(worst)
}

fn status_rank(s: &ComponentStatus) -> u8 {
    match s {
        ComponentStatus::Operational => 0,
        ComponentStatus::DegradedPerformance => 1,
        ComponentStatus::PartialOutage => 2,
        ComponentStatus::UnderMaintenance => 3,
        ComponentStatus::MajorOutage => 4,
    }
}

fn rank_to_status(rank: u8) -> ComponentStatus {
    match rank {
        0 => ComponentStatus::Operational,
        1 => ComponentStatus::DegradedPerformance,
        2 => ComponentStatus::PartialOutage,
        3 => ComponentStatus::UnderMaintenance,
        _ => ComponentStatus::MajorOutage,
    }
}

/// Count components in each status.
///
/// Returns a map where keys are the debug-formatted status names
/// (e.g., "Operational", "MajorOutage") and values are the counts
/// of components with that status.
pub fn count_by_status(components: &[StatusComponent]) -> std::collections::HashMap<String, usize> {
    let mut counts = std::collections::HashMap::new();
    for c in components {
        let key = format!("{:?}", c.status);
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

/// Check if the status page has any non-operational components.
///
/// Returns `true` if any component has a status other than
/// `ComponentStatus::Operational`, indicating some form of issue
/// or maintenance.
pub fn has_issues(page: &StatusPage) -> bool {
    page.components.iter().any(|c| c.status != ComponentStatus::Operational)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_component(name: &str, status: ComponentStatus) -> StatusComponent {
        StatusComponent {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: "test component".to_string(),
            status,
            group: None,
            updated_at: Utc::now(),
        }
    }

    fn make_page(components: Vec<StatusComponent>) -> StatusPage {
        let overall = compute_overall_status(&components);
        StatusPage {
            name: "Test Page".to_string(),
            components,
            overall_status: overall,
            last_updated: Utc::now(),
        }
    }

    #[test]
    fn test_overall_status_all_operational() {
        let components = vec![
            make_component("API", ComponentStatus::Operational),
            make_component("DB", ComponentStatus::Operational),
        ];
        let status = compute_overall_status(&components);
        assert_eq!(status, ComponentStatus::Operational);
    }

    #[test]
    fn test_overall_status_one_outage() {
        let components = vec![
            make_component("API", ComponentStatus::Operational),
            make_component("DB", ComponentStatus::MajorOutage),
        ];
        let status = compute_overall_status(&components);
        assert_eq!(status, ComponentStatus::MajorOutage);
    }

    #[test]
    fn test_overall_status_empty() {
        let components: Vec<StatusComponent> = vec![];
        let status = compute_overall_status(&components);
        assert_eq!(status, ComponentStatus::Operational);
    }

    #[test]
    fn test_has_issues_false() {
        let page = make_page(vec![
            make_component("API", ComponentStatus::Operational),
            make_component("DB", ComponentStatus::Operational),
        ]);
        assert!(!has_issues(&page));
    }

    #[test]
    fn test_has_issues_true() {
        let page = make_page(vec![
            make_component("API", ComponentStatus::Operational),
            make_component("Cache", ComponentStatus::DegradedPerformance),
        ]);
        assert!(has_issues(&page));
    }

    #[test]
    fn test_count_by_status() {
        let components = vec![
            make_component("A", ComponentStatus::Operational),
            make_component("B", ComponentStatus::Operational),
            make_component("C", ComponentStatus::MajorOutage),
            make_component("D", ComponentStatus::DegradedPerformance),
        ];
        let counts = count_by_status(&components);
        assert_eq!(counts.get("Operational").copied().unwrap_or(0), 2);
        assert_eq!(counts.get("MajorOutage").copied().unwrap_or(0), 1);
        assert_eq!(counts.get("DegradedPerformance").copied().unwrap_or(0), 1);
    }
}
