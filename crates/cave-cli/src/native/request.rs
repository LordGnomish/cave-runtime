// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared `PreparedRequest` shape for native verb tests.
//!
//! Each verb's `prepare` function returns a `PreparedRequest` so tests
//! can assert method/path/body without spinning up an HTTP server.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpVerb {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpVerb {
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpVerb::Get => "GET",
            HttpVerb::Post => "POST",
            HttpVerb::Put => "PUT",
            HttpVerb::Patch => "PATCH",
            HttpVerb::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRequest {
    pub verb: HttpVerb,
    pub path: String,
    pub body: Option<Value>,
}

impl PreparedRequest {
    pub fn new(verb: HttpVerb, path: impl Into<String>) -> Self {
        Self {
            verb,
            path: path.into(),
            body: None,
        }
    }

    pub fn with_body(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verb_as_str_round_trip() {
        assert_eq!(HttpVerb::Get.as_str(), "GET");
        assert_eq!(HttpVerb::Post.as_str(), "POST");
        assert_eq!(HttpVerb::Put.as_str(), "PUT");
        assert_eq!(HttpVerb::Patch.as_str(), "PATCH");
        assert_eq!(HttpVerb::Delete.as_str(), "DELETE");
    }

    #[test]
    fn new_has_no_body_by_default() {
        let r = PreparedRequest::new(HttpVerb::Get, "/x");
        assert!(r.body.is_none());
        assert_eq!(r.path, "/x");
    }

    #[test]
    fn with_body_sets_body() {
        let r = PreparedRequest::new(HttpVerb::Post, "/x").with_body(serde_json::json!({"a":1}));
        assert_eq!(r.body.unwrap()["a"], 1);
    }
}
