// SPDX-License-Identifier: AGPL-3.0-or-later
#[derive(Debug, thiserror::Error)]
pub enum DocsError {
    #[error("space not found: {0}")]
    SpaceNotFound(String),
    #[error("page not found: {0}")]
    PageNotFound(String),
    #[error("space already exists: {0}")]
    SpaceExists(String),
    #[error("version not found: {0}")]
    VersionNotFound(String),
    #[error("render error: {0}")]
    RenderError(String),
    #[error("search error: {0}")]
    SearchError(String),
    #[error("invalid slug: {0}")]
    InvalidSlug(String),
    #[error("openapi parse error: {0}")]
    OpenApiError(String),
}

pub type DocsResult<T> = Result<T, DocsError>;
