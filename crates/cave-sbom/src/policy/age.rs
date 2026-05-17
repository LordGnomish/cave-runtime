// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/policy/ComponentAgePolicyEvaluator.java
//
//! Age policy — flag components whose `published_at` is older than `days`.

use crate::components::ComponentRecord;
use chrono::{DateTime, Duration, Utc};

pub fn violates(c: &ComponentRecord, days: u32, now: DateTime<Utc>) -> Option<String> {
    let Some(published) = c.published_at else {
        return None;
    };
    let age = now - published;
    let threshold = Duration::days(days as i64);
    if age > threshold {
        Some(format!(
            "component published {} days ago (threshold: {})",
            age.num_days(),
            days
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn comp(published: Option<DateTime<Utc>>) -> ComponentRecord {
        let mut c = ComponentRecord::new(Uuid::new_v4(), "x", "1");
        c.published_at = published;
        c
    }

    #[test]
    fn fires_when_older_than_threshold() {
        let now = Utc::now();
        let c = comp(Some(now - Duration::days(400)));
        assert!(violates(&c, 365, now).is_some());
    }

    #[test]
    fn passes_when_within_threshold() {
        let now = Utc::now();
        let c = comp(Some(now - Duration::days(100)));
        assert!(violates(&c, 365, now).is_none());
    }

    #[test]
    fn no_published_means_no_violation() {
        let now = Utc::now();
        let c = comp(None);
        assert!(violates(&c, 0, now).is_none());
    }

    #[test]
    fn future_published_is_not_violation() {
        let now = Utc::now();
        let c = comp(Some(now + Duration::days(10)));
        assert!(violates(&c, 0, now).is_none());
    }
}
