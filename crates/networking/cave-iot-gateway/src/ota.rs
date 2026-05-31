// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! OTA firmware/software campaigns. (RED.)

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg() -> OtaPackage {
        OtaPackage {
            id: "p1".into(),
            title: "fw".into(),
            version: "2.0.0".into(),
            package_type: OtaType::Firmware,
            checksum: "abc123".into(),
            checksum_algorithm: "SHA256".into(),
            size: 1024,
        }
    }

    #[test]
    fn new_campaign_starts_all_queued() {
        let c = OtaCampaign::new("c1", pkg(), &["d1".into(), "d2".into()]);
        let p = c.progress();
        assert_eq!(p.total, 2);
        assert_eq!(p.queued, 2);
        assert_eq!(p.updated, 0);
        assert_eq!(c.status("d1"), Some(OtaStatus::Queued));
    }

    #[test]
    fn happy_path_transitions_succeed() {
        let mut c = OtaCampaign::new("c1", pkg(), &["d1".into()]);
        for s in [
            OtaStatus::Initiated,
            OtaStatus::Downloading,
            OtaStatus::Downloaded,
            OtaStatus::Verified,
            OtaStatus::Updating,
            OtaStatus::Updated,
        ] {
            assert!(c.advance("d1", s).is_ok(), "transition to {s:?} should be allowed");
        }
        assert_eq!(c.status("d1"), Some(OtaStatus::Updated));
    }

    #[test]
    fn skipping_states_is_rejected() {
        let mut c = OtaCampaign::new("c1", pkg(), &["d1".into()]);
        // Queued cannot jump straight to Updated.
        assert!(c.advance("d1", OtaStatus::Updated).is_err());
    }

    #[test]
    fn any_state_can_fail() {
        let mut c = OtaCampaign::new("c1", pkg(), &["d1".into()]);
        c.advance("d1", OtaStatus::Initiated).unwrap();
        c.advance("d1", OtaStatus::Downloading).unwrap();
        assert!(c.advance("d1", OtaStatus::Failed).is_ok());
        // Failed is terminal — no resurrection.
        assert!(c.advance("d1", OtaStatus::Downloading).is_err());
    }

    #[test]
    fn progress_and_completion() {
        let mut c = OtaCampaign::new("c1", pkg(), &["d1".into(), "d2".into()]);
        // d1 → Updated, d2 → Failed.
        for s in [
            OtaStatus::Initiated,
            OtaStatus::Downloading,
            OtaStatus::Downloaded,
            OtaStatus::Verified,
            OtaStatus::Updating,
            OtaStatus::Updated,
        ] {
            c.advance("d1", s).unwrap();
        }
        c.advance("d2", OtaStatus::Failed).unwrap();
        let p = c.progress();
        assert_eq!(p.updated, 1);
        assert_eq!(p.failed, 1);
        assert!(c.is_complete());
    }

    #[test]
    fn checksum_verification() {
        let p = pkg();
        assert!(p.verify("abc123"));
        assert!(!p.verify("deadbeef"));
    }

    #[test]
    fn advance_unknown_device_errors() {
        let mut c = OtaCampaign::new("c1", pkg(), &["d1".into()]);
        assert!(c.advance("ghost", OtaStatus::Initiated).is_err());
    }
}
