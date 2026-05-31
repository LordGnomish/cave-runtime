// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM Attachment — `packages/twenty-server/src/modules/attachment/standard-objects/attachment.workspace-entity.ts`
//!
//! A file uploaded against any CRM record. Twenty stores `name`,
//! `fullPath`, a derived `fileCategory`, the uploading `author`
//! (WorkspaceMember), and a fan-out of nullable polymorphic `target*`
//! relations (Task / Note / Person / Company / Opportunity / Dashboard /
//! Workflow). We mirror the polymorphic link with a single
//! `(target_kind, target_id)` pair, matching the `ActivityTarget` idiom,
//! and derive `file_category` from the file extension exactly as Twenty's
//! `fileCategory` computed column does.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The CRM record an attachment hangs off. Mirrors Twenty's
/// `targetTask`/`targetNote`/`targetPerson`/... relation fan-out.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AttachmentTargetKind {
    Person,
    Company,
    Opportunity,
    Task,
    Note,
    Workflow,
    Dashboard,
}

/// Coarse file class derived from the extension — mirrors Twenty's
/// `fileCategory` computed value used by the front-end preview chooser.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FileCategory {
    Image,
    Video,
    Audio,
    TextDocument,
    Pdf,
    Spreadsheet,
    Presentation,
    Archive,
    Other,
}

/// Attachment workspace-entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attachment {
    pub id: Uuid,
    pub workspace_id: Uuid,
    /// Twenty `name` (display label).
    pub name: String,
    /// Twenty `fullPath` (storage key / path).
    pub full_path: String,
    /// Twenty `fileCategory` (derived, non-null).
    pub file_category: FileCategory,
    /// Twenty `authorId` — the WorkspaceMember who uploaded.
    pub author_id: Option<Uuid>,
    /// Polymorphic `target*` relation kind (one-of).
    pub target_kind: Option<AttachmentTargetKind>,
    /// FK into the targeted record.
    pub target_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Attachment {
    /// Build an unattached attachment, deriving `file_category` from the
    /// `full_path` extension.
    pub fn new(
        workspace_id: Uuid,
        name: impl Into<String>,
        full_path: impl Into<String>,
    ) -> Self {
        let full_path = full_path.into();
        let file_category = Self::categorize(&full_path);
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            name: name.into(),
            full_path,
            file_category,
            author_id: None,
            target_kind: None,
            target_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Classify a filename/path by extension — mirrors Twenty's
    /// `fileCategory` derivation. Unknown / extensionless paths fall through
    /// to [`FileCategory::Other`]. Matching is case-insensitive.
    pub fn categorize(path: &str) -> FileCategory {
        let ext = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .rsplit('.')
            .next()
            .map(|e| e.to_ascii_lowercase())
            .filter(|e| e != &path.to_ascii_lowercase()); // no '.' → no extension
        match ext.as_deref() {
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "heic") => {
                FileCategory::Image
            }
            Some("mp4" | "mov" | "webm" | "avi" | "mkv") => FileCategory::Video,
            Some("mp3" | "wav" | "ogg" | "flac" | "m4a") => FileCategory::Audio,
            Some("pdf") => FileCategory::Pdf,
            Some("xls" | "xlsx" | "csv" | "tsv" | "ods") => FileCategory::Spreadsheet,
            Some("ppt" | "pptx" | "odp" | "key") => FileCategory::Presentation,
            Some("doc" | "docx" | "txt" | "md" | "rtf" | "odt") => FileCategory::TextDocument,
            Some("zip" | "gz" | "tar" | "rar" | "7z" | "bz2") => FileCategory::Archive,
            _ => FileCategory::Other,
        }
    }

    /// Attach this file to a CRM record (consuming builder).
    pub fn attach_to(mut self, kind: AttachmentTargetKind, target_id: Uuid) -> Self {
        self.target_kind = Some(kind);
        self.target_id = Some(target_id);
        self
    }

    /// True once a polymorphic target is set.
    pub fn is_attached(&self) -> bool {
        self.target_kind.is_some() && self.target_id.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorize_matches_extension_case_insensitively() {
        assert_eq!(Attachment::categorize("photo.PNG"), FileCategory::Image);
        assert_eq!(Attachment::categorize("clip.mp4"), FileCategory::Video);
        assert_eq!(Attachment::categorize("call.WAV"), FileCategory::Audio);
        assert_eq!(Attachment::categorize("contract.pdf"), FileCategory::Pdf);
        assert_eq!(Attachment::categorize("q3.xlsx"), FileCategory::Spreadsheet);
        assert_eq!(Attachment::categorize("deck.pptx"), FileCategory::Presentation);
        assert_eq!(Attachment::categorize("notes.md"), FileCategory::TextDocument);
        assert_eq!(Attachment::categorize("backup.tar.gz"), FileCategory::Archive);
        assert_eq!(Attachment::categorize("mystery.xyz"), FileCategory::Other);
        assert_eq!(Attachment::categorize("noext"), FileCategory::Other);
    }

    #[test]
    fn new_derives_category_from_full_path() {
        let a = Attachment::new(Uuid::new_v4(), "Logo", "uploads/logo.svg");
        assert_eq!(a.name, "Logo");
        assert_eq!(a.file_category, FileCategory::Image);
        assert!(a.target_kind.is_none());
        assert!(a.target_id.is_none());
    }

    #[test]
    fn attach_to_sets_polymorphic_target() {
        let person = Uuid::new_v4();
        let a = Attachment::new(Uuid::new_v4(), "CV", "cv.pdf")
            .attach_to(AttachmentTargetKind::Person, person);
        assert_eq!(a.target_kind, Some(AttachmentTargetKind::Person));
        assert_eq!(a.target_id, Some(person));
        assert!(a.is_attached());
    }

    #[test]
    fn fresh_attachment_is_not_attached() {
        let a = Attachment::new(Uuid::new_v4(), "x", "x.txt");
        assert!(!a.is_attached());
    }

    #[test]
    fn category_serializes_screaming_snake() {
        let a = Attachment::new(Uuid::new_v4(), "Q3", "q3.xlsx");
        let j = serde_json::to_value(&a).unwrap();
        assert_eq!(j["file_category"], "SPREADSHEET");
    }
}
