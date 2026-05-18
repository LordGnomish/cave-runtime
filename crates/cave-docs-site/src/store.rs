// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::error::*;
use crate::renderer::MarkdownRenderer;
use crate::types::*;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[allow(dead_code)]
pub struct DocsStore {
    spaces: Arc<RwLock<HashMap<String, Space>>>,
    pages: Arc<RwLock<HashMap<String, Page>>>,
    groups: Arc<RwLock<HashMap<String, PageGroup>>>,
    versions: Arc<RwLock<HashMap<String, Vec<DocVersion>>>>,
    custom_domains: Arc<RwLock<HashMap<String, CustomDomain>>>,
    renderer: MarkdownRenderer,
}

impl DocsStore {
    pub fn new() -> Self {
        DocsStore {
            spaces: Arc::new(RwLock::new(HashMap::new())),
            pages: Arc::new(RwLock::new(HashMap::new())),
            groups: Arc::new(RwLock::new(HashMap::new())),
            versions: Arc::new(RwLock::new(HashMap::new())),
            custom_domains: Arc::new(RwLock::new(HashMap::new())),
            renderer: MarkdownRenderer::new(),
        }
    }

    // -------------------------------------------------------------------------
    // Spaces
    // -------------------------------------------------------------------------

    pub fn create_space(
        &self,
        slug: &str,
        title: &str,
        description: &str,
    ) -> DocsResult<Space> {
        let mut spaces = self.spaces.write().unwrap();
        // Check slug uniqueness
        if spaces.values().any(|s| s.slug == slug) {
            return Err(DocsError::SpaceExists(slug.to_string()));
        }
        let now = Utc::now();
        let space = Space {
            id: Uuid::new_v4().to_string(),
            slug: slug.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            custom_domain: None,
            created_at: now,
            updated_at: now,
            visibility: Visibility::Public,
            default_version: "main".to_string(),
        };
        spaces.insert(space.id.clone(), space.clone());
        Ok(space)
    }

    pub fn get_space(&self, id: &str) -> DocsResult<Space> {
        self.spaces
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DocsError::SpaceNotFound(id.to_string()))
    }

    pub fn get_space_by_slug(&self, slug: &str) -> DocsResult<Space> {
        self.spaces
            .read()
            .unwrap()
            .values()
            .find(|s| s.slug == slug)
            .cloned()
            .ok_or_else(|| DocsError::SpaceNotFound(slug.to_string()))
    }

    pub fn list_spaces(&self) -> Vec<Space> {
        self.spaces.read().unwrap().values().cloned().collect()
    }

    pub fn update_space(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        custom_domain: Option<String>,
    ) -> DocsResult<Space> {
        let mut spaces = self.spaces.write().unwrap();
        let space = spaces
            .get_mut(id)
            .ok_or_else(|| DocsError::SpaceNotFound(id.to_string()))?;
        if let Some(t) = title {
            space.title = t.to_string();
        }
        if let Some(d) = description {
            space.description = d.to_string();
        }
        if custom_domain.is_some() {
            space.custom_domain = custom_domain;
        }
        space.updated_at = Utc::now();
        Ok(space.clone())
    }

    pub fn delete_space(&self, id: &str) -> DocsResult<()> {
        let mut spaces = self.spaces.write().unwrap();
        spaces
            .remove(id)
            .ok_or_else(|| DocsError::SpaceNotFound(id.to_string()))?;
        // Clean up pages & groups for this space
        self.pages
            .write()
            .unwrap()
            .retain(|_, p| p.space_id != id);
        self.groups
            .write()
            .unwrap()
            .retain(|_, g| g.space_id != id);
        self.versions.write().unwrap().remove(id);
        Ok(())
    }

    pub fn set_custom_domain(&self, space_id: &str, domain: &str) -> DocsResult<CustomDomain> {
        // Verify space exists
        let _ = self.get_space(space_id)?;
        let cd = CustomDomain {
            domain: domain.to_string(),
            space_id: space_id.to_string(),
            verified: false,
            created_at: Utc::now(),
        };
        self.custom_domains
            .write()
            .unwrap()
            .insert(domain.to_string(), cd.clone());
        // Update space record
        let _ = self.update_space(space_id, None, None, Some(domain.to_string()));
        Ok(cd)
    }

    pub fn resolve_custom_domain(&self, domain: &str) -> Option<Space> {
        let domains = self.custom_domains.read().unwrap();
        let cd = domains.get(domain)?;
        let space_id = cd.space_id.clone();
        drop(domains);
        self.get_space(&space_id).ok()
    }

    // -------------------------------------------------------------------------
    // Versions
    // -------------------------------------------------------------------------

    pub fn create_version(
        &self,
        space_id: &str,
        name: &str,
        branch: Option<&str>,
    ) -> DocsResult<DocVersion> {
        let _ = self.get_space(space_id)?;
        let ver = DocVersion {
            id: Uuid::new_v4().to_string(),
            space_id: space_id.to_string(),
            name: name.to_string(),
            branch: branch.map(|b| b.to_string()),
            is_default: false,
            published: false,
            created_at: Utc::now(),
        };
        self.versions
            .write()
            .unwrap()
            .entry(space_id.to_string())
            .or_default()
            .push(ver.clone());
        Ok(ver)
    }

    pub fn get_version(&self, space_id: &str, name: &str) -> DocsResult<DocVersion> {
        self.versions
            .read()
            .unwrap()
            .get(space_id)
            .and_then(|vs| vs.iter().find(|v| v.name == name).cloned())
            .ok_or_else(|| DocsError::VersionNotFound(name.to_string()))
    }

    pub fn list_versions(&self, space_id: &str) -> Vec<DocVersion> {
        self.versions
            .read()
            .unwrap()
            .get(space_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_default_version(&self, space_id: &str, version_name: &str) -> DocsResult<()> {
        let mut versions = self.versions.write().unwrap();
        let vs = versions
            .get_mut(space_id)
            .ok_or_else(|| DocsError::VersionNotFound(version_name.to_string()))?;
        let found = vs.iter().any(|v| v.name == version_name);
        if !found {
            return Err(DocsError::VersionNotFound(version_name.to_string()));
        }
        for v in vs.iter_mut() {
            v.is_default = v.name == version_name;
        }
        Ok(())
    }

    pub fn publish_version(&self, space_id: &str, version_name: &str) -> DocsResult<()> {
        let mut versions = self.versions.write().unwrap();
        let vs = versions
            .get_mut(space_id)
            .ok_or_else(|| DocsError::VersionNotFound(version_name.to_string()))?;
        let v = vs
            .iter_mut()
            .find(|v| v.name == version_name)
            .ok_or_else(|| DocsError::VersionNotFound(version_name.to_string()))?;
        v.published = true;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Page Groups
    // -------------------------------------------------------------------------

    pub fn create_group(
        &self,
        space_id: &str,
        title: &str,
        order: u32,
        version: &str,
    ) -> DocsResult<PageGroup> {
        let _ = self.get_space(space_id)?;
        let group = PageGroup {
            id: Uuid::new_v4().to_string(),
            space_id: space_id.to_string(),
            title: title.to_string(),
            order,
            version: version.to_string(),
        };
        self.groups
            .write()
            .unwrap()
            .insert(group.id.clone(), group.clone());
        Ok(group)
    }

    pub fn list_groups(&self, space_id: &str, version: &str) -> Vec<PageGroup> {
        self.groups
            .read()
            .unwrap()
            .values()
            .filter(|g| g.space_id == space_id && g.version == version)
            .cloned()
            .collect()
    }

    pub fn delete_group(&self, group_id: &str) -> DocsResult<()> {
        self.groups
            .write()
            .unwrap()
            .remove(group_id)
            .ok_or_else(|| DocsError::PageNotFound(group_id.to_string()))?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Pages
    // -------------------------------------------------------------------------

    pub fn create_page(
        &self,
        space_id: &str,
        slug: &str,
        title: &str,
        markdown: &str,
        group_id: Option<String>,
        order: u32,
        version: &str,
    ) -> DocsResult<Page> {
        let _ = self.get_space(space_id)?;
        let now = Utc::now();
        let (meta, content) = MarkdownRenderer::extract_frontmatter(markdown);
        let html_content = self.renderer.render(content);
        let mut page = Page {
            id: Uuid::new_v4().to_string(),
            space_id: space_id.to_string(),
            slug: slug.to_string(),
            title: title.to_string(),
            markdown_content: markdown.to_string(),
            html_content: Some(html_content),
            group_id,
            parent_id: None,
            order,
            created_at: now,
            updated_at: now,
            version: version.to_string(),
            metadata: meta,
        };
        // Re-apply frontmatter metadata
        let (fm_meta, _) = MarkdownRenderer::extract_frontmatter(&page.markdown_content.clone());
        page.metadata = fm_meta;
        self.pages
            .write()
            .unwrap()
            .insert(page.id.clone(), page.clone());
        Ok(page)
    }

    pub fn get_page(&self, page_id: &str) -> DocsResult<Page> {
        self.pages
            .read()
            .unwrap()
            .get(page_id)
            .cloned()
            .ok_or_else(|| DocsError::PageNotFound(page_id.to_string()))
    }

    pub fn get_page_by_slug(&self, space_id: &str, slug: &str, version: &str) -> DocsResult<Page> {
        self.pages
            .read()
            .unwrap()
            .values()
            .find(|p| p.space_id == space_id && p.slug == slug && p.version == version)
            .cloned()
            .ok_or_else(|| DocsError::PageNotFound(slug.to_string()))
    }

    pub fn list_pages(&self, space_id: &str, version: &str) -> Vec<Page> {
        let mut pages: Vec<Page> = self
            .pages
            .read()
            .unwrap()
            .values()
            .filter(|p| p.space_id == space_id && p.version == version)
            .cloned()
            .collect();
        pages.sort_by_key(|p| p.order);
        pages
    }

    pub fn update_page(
        &self,
        page_id: &str,
        title: Option<&str>,
        markdown: Option<&str>,
    ) -> DocsResult<Page> {
        let mut pages = self.pages.write().unwrap();
        let page = pages
            .get_mut(page_id)
            .ok_or_else(|| DocsError::PageNotFound(page_id.to_string()))?;
        if let Some(t) = title {
            page.title = t.to_string();
        }
        if let Some(md) = markdown {
            page.markdown_content = md.to_string();
            let (meta, content) = MarkdownRenderer::extract_frontmatter(md);
            page.metadata = meta;
            page.html_content = Some(self.renderer.render(content));
        }
        page.updated_at = Utc::now();
        Ok(page.clone())
    }

    pub fn delete_page(&self, page_id: &str) -> DocsResult<()> {
        self.pages
            .write()
            .unwrap()
            .remove(page_id)
            .ok_or_else(|| DocsError::PageNotFound(page_id.to_string()))?;
        Ok(())
    }

    pub fn render_page(&self, page_id: &str) -> DocsResult<String> {
        let pages = self.pages.read().unwrap();
        let page = pages
            .get(page_id)
            .ok_or_else(|| DocsError::PageNotFound(page_id.to_string()))?;
        if let Some(html) = &page.html_content {
            return Ok(html.clone());
        }
        let (_, content) = MarkdownRenderer::extract_frontmatter(&page.markdown_content);
        Ok(self.renderer.render(content))
    }

    #[allow(dead_code)]
    fn render_and_cache(&self, page: &mut Page) {
        let md = page.markdown_content.clone();
        let (meta, content) = MarkdownRenderer::extract_frontmatter(&md);
        page.metadata = meta;
        page.html_content = Some(self.renderer.render(content));
    }
}

impl Default for DocsStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_space_and_page() {
        let store = DocsStore::new();
        let space = store
            .create_space("my-space", "My Space", "A test space")
            .unwrap();
        assert_eq!(space.slug, "my-space");

        let page = store
            .create_page(
                &space.id,
                "intro",
                "Introduction",
                "# Introduction\n\nHello world!",
                None,
                0,
                "main",
            )
            .unwrap();
        assert_eq!(page.title, "Introduction");

        let fetched = store.get_page(&page.id).unwrap();
        assert_eq!(fetched.slug, "intro");
        assert!(fetched.markdown_content.contains("Hello world!"));
    }

    #[test]
    fn render_page() {
        let store = DocsStore::new();
        let space = store.create_space("render-test", "Render", "").unwrap();
        let page = store
            .create_page(
                &space.id,
                "test",
                "Test",
                "# Header\n\n**bold text**",
                None,
                0,
                "main",
            )
            .unwrap();
        let html = store.render_page(&page.id).unwrap();
        assert!(html.contains("<h1"), "expected h1 in: {html}");
        assert!(html.contains("<strong>"), "expected strong in: {html}");
    }

    #[test]
    fn version_create_and_list() {
        let store = DocsStore::new();
        let space = store.create_space("ver-space", "Ver Space", "").unwrap();
        let v1 = store.create_version(&space.id, "v1.0", Some("main")).unwrap();
        let v2 = store.create_version(&space.id, "v2.0", None).unwrap();

        assert_eq!(v1.name, "v1.0");
        assert_eq!(v2.name, "v2.0");

        let versions = store.list_versions(&space.id);
        assert_eq!(versions.len(), 2);

        store.set_default_version(&space.id, "v1.0").unwrap();
        let versions = store.list_versions(&space.id);
        let default = versions.iter().find(|v| v.is_default).unwrap();
        assert_eq!(default.name, "v1.0");
    }

    #[test]
    fn custom_domain_set_resolve() {
        let store = DocsStore::new();
        let space = store
            .create_space("domain-space", "Domain Space", "")
            .unwrap();
        let cd = store
            .set_custom_domain(&space.id, "docs.example.com")
            .unwrap();
        assert_eq!(cd.domain, "docs.example.com");

        let resolved = store.resolve_custom_domain("docs.example.com").unwrap();
        assert_eq!(resolved.id, space.id);

        let none = store.resolve_custom_domain("unknown.com");
        assert!(none.is_none());
    }
}
