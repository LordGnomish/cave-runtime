// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Plugin host — the registry of pages, panels, and nav entries contributed by
//! modules.
//!
//! Backstage models the same idea as a plugin tree where each plugin exports
//! Routes / Cards / Tabs. Cave's portal-web is server-rendered, so a "plugin"
//! is simply a [`Plugin`] trait impl whose `register` fn pushes [`Page`]s and
//! sidebar entries into a [`PluginRegistry`].

use crate::page::Page;

/// A single sidebar entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavEntry {
    pub label: String,
    pub icon: String,
    pub path: String,
    pub group: String,
    pub order: i32,
}

impl NavEntry {
    pub fn new(label: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: "default".into(),
            path: path.into(),
            group: "general".into(),
            order: 100,
        }
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn with_group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }

    pub fn with_order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }
}

/// A panel rendered inside the dashboard for a given module.
///
/// Panels are *small* pieces of UI (a chart, a counter, a list). Each panel
/// has a *kind* (`"chart"`, `"list"`, `"counter"`, `"alert"`, ...) and a body
/// produced lazily so it can use live request context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panel {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub module: String,
    pub body: String,
}

impl Panel {
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        title: impl Into<String>,
        module: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            title: title.into(),
            module: module.into(),
            body: String::new(),
        }
    }

    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }
}

/// Anything implementing [`Plugin`] can be registered with a
/// [`PluginRegistry`].
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn register(&self, registry: &mut PluginRegistry);
}

#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<String>,
    pages: Vec<Page>,
    nav: Vec<NavEntry>,
    panels: Vec<Panel>,
}

impl std::fmt::Debug for PluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginRegistry")
            .field("plugins", &self.plugins)
            .field("page_count", &self.pages.len())
            .field("nav_count", &self.nav.len())
            .field("panel_count", &self.panels.len())
            .finish()
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install<P: Plugin>(&mut self, plugin: &P) -> &mut Self {
        self.plugins.push(plugin.name().to_string());
        plugin.register(self);
        self
    }

    pub fn add_page(&mut self, page: Page) -> &mut Self {
        self.pages.push(page);
        self
    }

    pub fn add_nav(&mut self, entry: NavEntry) -> &mut Self {
        self.nav.push(entry);
        self
    }

    pub fn add_panel(&mut self, panel: Panel) -> &mut Self {
        self.panels.push(panel);
        self
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    pub fn nav(&self) -> &[NavEntry] {
        &self.nav
    }

    pub fn panels(&self) -> &[Panel] {
        &self.panels
    }

    pub fn plugins(&self) -> &[String] {
        &self.plugins
    }

    pub fn nav_grouped(&self) -> Vec<(String, Vec<NavEntry>)> {
        let mut groups: std::collections::BTreeMap<String, Vec<NavEntry>> = Default::default();
        for entry in &self.nav {
            groups.entry(entry.group.clone()).or_default().push(entry.clone());
        }
        for entries in groups.values_mut() {
            entries.sort_by_key(|e| (e.order, e.label.clone()));
        }
        groups.into_iter().collect()
    }

    pub fn panels_for_module(&self, module: &str) -> Vec<&Panel> {
        self.panels.iter().filter(|p| p.module == module).collect()
    }

    pub fn find_page(&self, id: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == id)
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn nav_count(&self) -> usize {
        self.nav.len()
    }

    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::Scope;

    struct DummyPlugin {
        name: &'static str,
        pages: Vec<Page>,
        nav: Vec<NavEntry>,
    }

    impl Plugin for DummyPlugin {
        fn name(&self) -> &str {
            self.name
        }
        fn register(&self, registry: &mut PluginRegistry) {
            for p in &self.pages {
                registry.add_page(p.clone());
            }
            for n in &self.nav {
                registry.add_nav(n.clone());
            }
        }
    }

    fn dummy_page(id: &str, path: &str) -> Page {
        Page::builder(id, path).scope(Scope::Public).build()
    }

    #[test]
    fn nav_entry_defaults() {
        let n = NavEntry::new("Home", "/");
        assert_eq!(n.label, "Home");
        assert_eq!(n.path, "/");
        assert_eq!(n.icon, "default");
        assert_eq!(n.group, "general");
        assert_eq!(n.order, 100);
    }

    #[test]
    fn nav_entry_builder_methods() {
        let n = NavEntry::new("Home", "/")
            .with_icon("home")
            .with_group("core")
            .with_order(10);
        assert_eq!(n.icon, "home");
        assert_eq!(n.group, "core");
        assert_eq!(n.order, 10);
    }

    #[test]
    fn panel_new_has_empty_body() {
        let p = Panel::new("p1", "chart", "Title", "mod");
        assert_eq!(p.body, "");
    }

    #[test]
    fn panel_with_body_sets_body() {
        let p = Panel::new("p1", "chart", "T", "m").with_body("<div/>");
        assert_eq!(p.body, "<div/>");
    }

    #[test]
    fn registry_starts_empty() {
        let r = PluginRegistry::new();
        assert_eq!(r.page_count(), 0);
        assert_eq!(r.nav_count(), 0);
        assert_eq!(r.panel_count(), 0);
        assert!(r.plugins().is_empty());
    }

    #[test]
    fn registry_add_page() {
        let mut r = PluginRegistry::new();
        r.add_page(dummy_page("a", "/a"));
        assert_eq!(r.page_count(), 1);
        assert_eq!(r.pages()[0].path, "/a");
    }

    #[test]
    fn registry_add_nav() {
        let mut r = PluginRegistry::new();
        r.add_nav(NavEntry::new("Home", "/"));
        assert_eq!(r.nav_count(), 1);
    }

    #[test]
    fn registry_add_panel() {
        let mut r = PluginRegistry::new();
        r.add_panel(Panel::new("p", "list", "T", "m"));
        assert_eq!(r.panel_count(), 1);
    }

    #[test]
    fn registry_install_records_plugin_name() {
        let plugin = DummyPlugin { name: "demo", pages: vec![], nav: vec![] };
        let mut r = PluginRegistry::new();
        r.install(&plugin);
        assert_eq!(r.plugins(), &["demo".to_string()]);
    }

    #[test]
    fn registry_install_runs_register_callback() {
        let plugin = DummyPlugin {
            name: "demo",
            pages: vec![dummy_page("a", "/a"), dummy_page("b", "/b")],
            nav: vec![NavEntry::new("A", "/a")],
        };
        let mut r = PluginRegistry::new();
        r.install(&plugin);
        assert_eq!(r.page_count(), 2);
        assert_eq!(r.nav_count(), 1);
    }

    #[test]
    fn registry_find_page_by_id() {
        let mut r = PluginRegistry::new();
        r.add_page(dummy_page("home", "/"));
        r.add_page(dummy_page("about", "/about"));
        assert_eq!(r.find_page("home").unwrap().path, "/");
        assert_eq!(r.find_page("about").unwrap().path, "/about");
        assert!(r.find_page("missing").is_none());
    }

    #[test]
    fn registry_panels_for_module_filters() {
        let mut r = PluginRegistry::new();
        r.add_panel(Panel::new("p1", "chart", "T", "modA"));
        r.add_panel(Panel::new("p2", "list", "T", "modB"));
        r.add_panel(Panel::new("p3", "counter", "T", "modA"));
        let modA = r.panels_for_module("modA");
        assert_eq!(modA.len(), 2);
    }

    #[test]
    fn registry_nav_grouped_groups_entries() {
        let mut r = PluginRegistry::new();
        r.add_nav(NavEntry::new("Apps", "/apps").with_group("dev"));
        r.add_nav(NavEntry::new("Logs", "/logs").with_group("ops"));
        r.add_nav(NavEntry::new("Tests", "/tests").with_group("dev"));
        let groups = r.nav_grouped();
        let dev = groups.iter().find(|(g, _)| g == "dev").unwrap();
        assert_eq!(dev.1.len(), 2);
        let ops = groups.iter().find(|(g, _)| g == "ops").unwrap();
        assert_eq!(ops.1.len(), 1);
    }

    #[test]
    fn registry_nav_grouped_sorts_by_order() {
        let mut r = PluginRegistry::new();
        r.add_nav(NavEntry::new("Z", "/z").with_group("g").with_order(50));
        r.add_nav(NavEntry::new("A", "/a").with_group("g").with_order(10));
        r.add_nav(NavEntry::new("M", "/m").with_group("g").with_order(20));
        let groups = r.nav_grouped();
        let g = groups.iter().find(|(g, _)| g == "g").unwrap();
        let labels: Vec<&str> = g.1.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["A", "M", "Z"]);
    }

    #[test]
    fn registry_install_chain_multiple_plugins() {
        let p1 = DummyPlugin { name: "p1", pages: vec![dummy_page("a", "/a")], nav: vec![] };
        let p2 = DummyPlugin { name: "p2", pages: vec![dummy_page("b", "/b")], nav: vec![] };
        let mut r = PluginRegistry::new();
        r.install(&p1).install(&p2);
        assert_eq!(r.plugins(), &["p1".to_string(), "p2".to_string()]);
        assert_eq!(r.page_count(), 2);
    }

    #[test]
    fn registry_debug_format_summarizes() {
        let mut r = PluginRegistry::new();
        r.add_page(dummy_page("a", "/a"));
        let s = format!("{:?}", r);
        assert!(s.contains("page_count: 1"));
    }
}
