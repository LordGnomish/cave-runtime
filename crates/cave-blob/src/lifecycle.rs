//! S3 / Azure object lifecycle policies.
//!
//! Models a tiny subset of the AWS [Lifecycle Configuration] / Azure
//! [Lifecycle Management] schemas:
//!
//! * Filter by key prefix or tag.
//! * Transition to a different storage class after N days.
//! * Expire after N days (delete).
//! * Abort incomplete multipart uploads after N days.
//!
//! [Lifecycle Configuration]:
//!   https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lifecycle-mgmt.html
//! [Lifecycle Management]:
//!   https://learn.microsoft.com/en-us/azure/storage/blobs/lifecycle-management-overview

use chrono::{DateTime, Duration, Utc};

use crate::error::{BlobError, BlobResult};
use crate::object::ObjectMeta;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageClass {
    Standard,
    StandardIa,
    OneZoneIa,
    GlacierInstantRetrieval,
    GlacierFlexibleRetrieval,
    GlacierDeepArchive,
    /// Azure equivalent.
    AzureCool,
    AzureCold,
    AzureArchive,
}

#[derive(Debug, Clone)]
pub struct LifecycleRule {
    pub id: String,
    pub prefix: String,
    pub enabled: bool,
    pub transition_days: Option<i64>,
    pub transition_class: Option<StorageClass>,
    pub expiration_days: Option<i64>,
    pub abort_incomplete_after_days: Option<i64>,
}

impl LifecycleRule {
    pub fn validate(&self) -> BlobResult<()> {
        if self.id.is_empty() {
            return Err(BlobError::InvalidLifecycleRule("missing id".into()));
        }
        if let Some(d) = self.transition_days {
            if d < 0 {
                return Err(BlobError::InvalidLifecycleRule("negative transition days".into()));
            }
            if self.transition_class.is_none() {
                return Err(BlobError::InvalidLifecycleRule(
                    "transition_days without transition_class".into(),
                ));
            }
        }
        if let Some(d) = self.expiration_days {
            if d < 0 {
                return Err(BlobError::InvalidLifecycleRule("negative expiration days".into()));
            }
        }
        if let (Some(t), Some(e)) = (self.transition_days, self.expiration_days) {
            if t >= e {
                return Err(BlobError::InvalidLifecycleRule(
                    "transition must precede expiration".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn applies_to(&self, key: &str) -> bool {
        self.enabled && key.starts_with(&self.prefix)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleAction {
    Noop,
    TransitionTo(StorageClass),
    Expire,
    AbortIncompleteUpload,
}

pub fn evaluate(rule: &LifecycleRule, meta: &ObjectMeta, now: DateTime<Utc>) -> LifecycleAction {
    if !rule.applies_to(&meta.key) {
        return LifecycleAction::Noop;
    }
    let age = now - meta.last_modified;
    if let Some(exp_days) = rule.expiration_days {
        if age >= Duration::days(exp_days) {
            return LifecycleAction::Expire;
        }
    }
    if let Some(t_days) = rule.transition_days {
        if age >= Duration::days(t_days) {
            if let Some(class) = &rule.transition_class {
                return LifecycleAction::TransitionTo(class.clone());
            }
        }
    }
    LifecycleAction::Noop
}

pub fn evaluate_incomplete_upload(
    rule: &LifecycleRule,
    initiated: DateTime<Utc>,
    now: DateTime<Utc>,
) -> LifecycleAction {
    if !rule.enabled {
        return LifecycleAction::Noop;
    }
    if let Some(days) = rule.abort_incomplete_after_days {
        if now - initiated >= Duration::days(days) {
            return LifecycleAction::AbortIncompleteUpload;
        }
    }
    LifecycleAction::Noop
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(key: &str, age_days: i64) -> ObjectMeta {
        ObjectMeta {
            bucket: "b".into(),
            key: key.into(),
            etag: String::new(),
            size: 0,
            content_type: String::new(),
            last_modified: Utc::now() - Duration::days(age_days),
            headers: Default::default(),
            version_id: None,
            immutable_until: None,
        }
    }

    fn rule(transition: Option<i64>, expiration: Option<i64>) -> LifecycleRule {
        LifecycleRule {
            id: "r".into(),
            prefix: String::new(),
            enabled: true,
            transition_days: transition,
            transition_class: transition.map(|_| StorageClass::StandardIa),
            expiration_days: expiration,
            abort_incomplete_after_days: None,
        }
    }

    #[test]
    fn rule_with_no_id_invalid() {
        let mut r = rule(None, Some(7));
        r.id.clear();
        assert!(r.validate().is_err());
    }
    #[test]
    fn rule_negative_transition_invalid() {
        let r = rule(Some(-1), None);
        assert!(r.validate().is_err());
    }
    #[test]
    fn rule_transition_without_class_invalid() {
        let mut r = rule(Some(7), None);
        r.transition_class = None;
        assert!(r.validate().is_err());
    }
    #[test]
    fn rule_transition_after_expiration_invalid() {
        let r = rule(Some(10), Some(5));
        assert!(r.validate().is_err());
    }
    #[test]
    fn rule_valid_transition_then_expire() {
        let r = rule(Some(30), Some(90));
        assert!(r.validate().is_ok());
    }
    #[test]
    fn rule_disabled_does_not_apply() {
        let mut r = rule(None, Some(1));
        r.enabled = false;
        assert!(!r.applies_to("k"));
    }
    #[test]
    fn rule_prefix_filter() {
        let mut r = rule(None, Some(1));
        r.prefix = "logs/".into();
        assert!(r.applies_to("logs/x.txt"));
        assert!(!r.applies_to("data/y.txt"));
    }

    #[test]
    fn evaluate_noop_for_fresh_object() {
        let r = rule(None, Some(7));
        let m = meta("k", 1);
        assert_eq!(evaluate(&r, &m, Utc::now()), LifecycleAction::Noop);
    }
    #[test]
    fn evaluate_expires_old_object() {
        let r = rule(None, Some(7));
        let m = meta("k", 8);
        assert_eq!(evaluate(&r, &m, Utc::now()), LifecycleAction::Expire);
    }
    #[test]
    fn evaluate_transitions_after_threshold() {
        let r = rule(Some(7), Some(30));
        let m = meta("k", 8);
        match evaluate(&r, &m, Utc::now()) {
            LifecycleAction::TransitionTo(StorageClass::StandardIa) => {}
            other => panic!("expected transition, got {:?}", other),
        }
    }
    #[test]
    fn evaluate_expiration_takes_precedence() {
        // age > both transition and expiration → expire.
        let r = rule(Some(7), Some(30));
        let m = meta("k", 31);
        assert_eq!(evaluate(&r, &m, Utc::now()), LifecycleAction::Expire);
    }
    #[test]
    fn evaluate_prefix_filter_skips_unmatched() {
        let mut r = rule(None, Some(1));
        r.prefix = "logs/".into();
        let m = meta("data/x.txt", 100);
        assert_eq!(evaluate(&r, &m, Utc::now()), LifecycleAction::Noop);
    }
    #[test]
    fn evaluate_incomplete_upload_aborts_after_threshold() {
        let mut r = rule(None, None);
        r.abort_incomplete_after_days = Some(7);
        let initiated = Utc::now() - Duration::days(8);
        assert_eq!(
            evaluate_incomplete_upload(&r, initiated, Utc::now()),
            LifecycleAction::AbortIncompleteUpload
        );
    }
    #[test]
    fn evaluate_incomplete_upload_noop_when_fresh() {
        let mut r = rule(None, None);
        r.abort_incomplete_after_days = Some(7);
        let initiated = Utc::now();
        assert_eq!(
            evaluate_incomplete_upload(&r, initiated, Utc::now()),
            LifecycleAction::Noop
        );
    }
}
