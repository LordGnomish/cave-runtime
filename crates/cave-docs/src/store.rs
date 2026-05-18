// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{DocBook, DocPage, DocSpace, DocStats, PageVersion};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct DocsStore {
    spaces: RwLock<HashMap<Uuid, DocSpace>>,
    books: RwLock<HashMap<Uuid, DocBook>>,
    pages: RwLock<HashMap<Uuid, DocPage>>,
    versions: RwLock<HashMap<Uuid, Vec<PageVersion>>>,
}

impl DocsStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Spaces ────────────────────────────────────────────────────────────────

    pub fn create_space(&self, space: DocSpace) -> DocSpace {
        let mut spaces = self.spaces.write().unwrap();
        let s = space.clone();
        spaces.insert(space.id, space);
        s
    }

    pub fn get_space(&self, id: &Uuid) -> Option<DocSpace> {
        self.spaces.read().unwrap().get(id).cloned()
    }

    pub fn list_spaces(&self) -> Vec<DocSpace> {
        let mut spaces: Vec<DocSpace> =
            self.spaces.read().unwrap().values().cloned().collect();
        spaces.sort_by(|a, b| a.name.cmp(&b.name));
        spaces
    }

    // ── Books ─────────────────────────────────────────────────────────────────

    pub fn create_book(&self, book: DocBook) -> DocBook {
        let mut books = self.books.write().unwrap();
        let b = book.clone();
        books.insert(book.id, book);
        b
    }

    pub fn get_book(&self, id: &Uuid) -> Option<DocBook> {
        self.books.read().unwrap().get(id).cloned()
    }

    pub fn list_books_in_space(&self, space_id: &Uuid) -> Vec<DocBook> {
        let mut books: Vec<DocBook> = self
            .books
            .read()
            .unwrap()
            .values()
            .filter(|b| b.space_id == *space_id)
            .cloned()
            .collect();
        books.sort_by(|a, b| a.name.cmp(&b.name));
        books
    }

    // ── Pages ─────────────────────────────────────────────────────────────────

    pub fn create_page(&self, page: DocPage) -> DocPage {
        let initial_version = PageVersion {
            version: 1,
            content: page.content.clone(),
            author: page.author.clone(),
            changed_at: page.created_at,
            change_summary: Some("Initial version".to_string()),
        };
        let mut versions = self.versions.write().unwrap();
        versions.entry(page.id).or_default().push(initial_version);
        drop(versions);

        let mut pages = self.pages.write().unwrap();
        let p = page.clone();
        pages.insert(page.id, page);
        p
    }

    pub fn get_page(&self, id: &Uuid) -> Option<DocPage> {
        self.pages.read().unwrap().get(id).cloned()
    }

    pub fn update_page(
        &self,
        id: &Uuid,
        title: Option<String>,
        content: Option<String>,
        tags: Option<Vec<String>>,
        author: String,
        change_summary: Option<String>,
    ) -> Option<DocPage> {
        let mut pages = self.pages.write().unwrap();
        if let Some(page) = pages.get_mut(id) {
            let new_version = page.version + 1;
            if let Some(t) = title {
                page.title = t;
            }
            if let Some(c) = content {
                page.content = c.clone();
                let pv = PageVersion {
                    version: new_version,
                    content: c,
                    author: author.clone(),
                    changed_at: Utc::now(),
                    change_summary,
                };
                let page_id = page.id;
                page.version = new_version;
                page.updated_at = Utc::now();
                drop(pages);
                self.versions
                    .write()
                    .unwrap()
                    .entry(page_id)
                    .or_default()
                    .push(pv);
                return self.pages.read().unwrap().get(id).cloned();
            }
            if let Some(t) = tags {
                page.tags = t;
            }
            page.updated_at = Utc::now();
            return Some(page.clone());
        }
        None
    }

    pub fn delete_page(&self, id: &Uuid) -> Option<DocPage> {
        self.versions.write().unwrap().remove(id);
        self.pages.write().unwrap().remove(id)
    }

    pub fn list_pages_in_book(&self, book_id: &Uuid) -> Vec<DocPage> {
        let mut pages: Vec<DocPage> = self
            .pages
            .read()
            .unwrap()
            .values()
            .filter(|p| p.book_id == *book_id)
            .cloned()
            .collect();
        pages.sort_by(|a, b| a.order.cmp(&b.order));
        pages
    }

    pub fn publish_page(&self, id: &Uuid) -> Option<DocPage> {
        let mut pages = self.pages.write().unwrap();
        if let Some(page) = pages.get_mut(id) {
            page.published = true;
            page.updated_at = Utc::now();
            return Some(page.clone());
        }
        None
    }

    pub fn get_page_versions(&self, page_id: &Uuid) -> Vec<PageVersion> {
        self.versions
            .read()
            .unwrap()
            .get(page_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn search_pages(&self, query: &str) -> Vec<DocPage> {
        let lower = query.to_lowercase();
        self.pages
            .read()
            .unwrap()
            .values()
            .filter(|p| {
                p.title.to_lowercase().contains(&lower)
                    || p.content.to_lowercase().contains(&lower)
            })
            .cloned()
            .collect()
    }

    // ── Stats ─────────────────────────────────────────────────────────────────

    pub fn compute_stats(&self) -> DocStats {
        let pages = self.pages.read().unwrap();
        let total_words: u64 = pages
            .values()
            .map(|p| p.content.split_whitespace().count() as u64)
            .sum();
        DocStats {
            total_spaces: self.spaces.read().unwrap().len() as u64,
            total_books: self.books.read().unwrap().len() as u64,
            total_pages: pages.len() as u64,
            total_words,
        }
    }
}
