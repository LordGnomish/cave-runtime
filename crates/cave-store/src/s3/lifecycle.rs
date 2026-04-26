//! Lifecycle rule evaluation and background enforcement.

use crate::s3::types::{LifecycleRule, ObjectVersion, StorageClass};
use chrono::Utc;

/// Evaluate whether a lifecycle rule's expiration applies to an object version.
pub fn should_expire(rule: &LifecycleRule, key: &str, version: &ObjectVersion) -> bool {
    if rule.status != "Enabled" {
        return false;
    }
    if !key.starts_with(&rule.prefix) {
        return false;
    }
    // Tag filter
    for (k, v) in &rule.tags {
        if version.tags.get(k).map(|s| s.as_str()) != Some(v.as_str()) {
            return false;
        }
    }
    let Some(ref exp) = rule.expiration else {
        return false;
    };
    if let Some(days) = exp.days {
        let age_days = (Utc::now() - version.last_modified).num_days();
        return age_days >= days as i64;
    }
    if let Some(date) = exp.date {
        return Utc::now() >= date;
    }
    if exp.expired_object_delete_marker == Some(true) && version.delete_marker {
        return true;
    }
    false
}

/// Evaluate whether an incomplete multipart upload should be aborted.
pub fn should_abort_multipart(
    rule: &LifecycleRule,
    key: &str,
    initiated: &chrono::DateTime<Utc>,
) -> bool {
    if rule.status != "Enabled" {
        return false;
    }
    if !key.starts_with(&rule.prefix) {
        return false;
    }
    let Some(ref abort) = rule.abort_incomplete_multipart_upload else {
        return false;
    };
    let age_days = (Utc::now() - *initiated).num_days();
    age_days >= abort.days_after_initiation as i64
}

/// Determine the target storage class for a lifecycle transition.
pub fn transition_class(
    rule: &LifecycleRule,
    key: &str,
    version: &ObjectVersion,
) -> Option<StorageClass> {
    if rule.status != "Enabled" {
        return None;
    }
    if !key.starts_with(&rule.prefix) {
        return None;
    }
    for t in &rule.transitions {
        if let Some(days) = t.days {
            let age_days = (Utc::now() - version.last_modified).num_days();
            if age_days >= days as i64 && version.storage_class != t.storage_class {
                return Some(t.storage_class.clone());
            }
        }
    }
    None
}
