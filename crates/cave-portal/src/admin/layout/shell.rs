//! Page-shell v2 — top bar + sidebar + breadcrumb + content + footer
//! + command palette + shortcuts + toasts, all wired together.
//!
//! Two ways to adopt:
//!
//! 1. **Existing handlers** continue calling
//!    `crate::admin::render::page_shell(title, body)`; that function
//!    now wraps the legacy markup inside a richer shell so every
//!    admin page gets the chrome for free (top bar + sidebar +
//!    breadcrumb + command palette + shortcut handler + toasts).
//! 2. **New handlers** that want explicit control over the chrome
//!    (e.g. to omit the sidebar on a focused detail page) call
//!    [`shell_v2`] with [`ShellOptions`] directly.
//!
//! Either way the result is one self-contained HTML document.

use crate::admin::layout::{
    breadcrumb::{breadcrumb_for_path, render as render_breadcrumb, Crumb},
    command_palette::{command_palette_modal, default_commands_for_persona, CommandItem},
    footer::footer,
    nav::sidebar,
    shortcuts::{shortcuts_help_modal, DEFAULT_BINDINGS},
    theme::theme_class_for_cookie,
    toast::toast_container,
};
use crate::admin::permission::Persona;
use crate::admin::render::escape;

#[derive(Debug, Clone)]
pub struct ShellOptions<'a> {
    pub title: &'a str,
    /// Persona — drives sidebar filtering + top-bar indicator.
    pub persona: Persona,
    /// Tenant the caller is acting as. Goes into the persona pill,
    /// nav-item query params, and command-palette default commands.
    pub tenant_id: &'a str,
    /// Current URL path — used for the breadcrumb + sidebar active
    /// highlight. Defaults to "/" when unknown.
    pub current_path: &'a str,
    /// Optional `cave_theme` cookie value — `dark` / `light` /
    /// `system`. Defaults to system.
    pub theme_cookie: Option<&'a str>,
    /// Optional custom breadcrumb (override the auto-derived one).
    /// `None` means derive from `current_path`.
    pub breadcrumb: Option<Vec<Crumb>>,
    /// Extra command-palette items the caller wants to inject
    /// alongside the defaults.
    pub extra_commands: Vec<CommandItem>,
    /// Cluster info line for the footer.
    pub cluster_info: &'a str,
    /// When true, omit the sidebar (used for full-bleed detail
    /// pages like a single-policy editor). Top bar + breadcrumb +
    /// shortcuts + palette still ship.
    pub hide_sidebar: bool,
    /// Body HTML, escaped by the caller as needed.
    pub body: &'a str,
}

impl<'a> Default for ShellOptions<'a> {
    fn default() -> Self {
        Self {
            title: "cave",
            persona: Persona::Anonymous,
            tenant_id: "dev",
            current_path: "/",
            theme_cookie: None,
            breadcrumb: None,
            extra_commands: Vec::new(),
            cluster_info: "cave-runtime",
            hide_sidebar: false,
            body: "",
        }
    }
}

/// Render the full document. The caller has already produced the
/// per-page `body` HTML; this wraps the chrome around it.
pub fn shell_v2(opts: ShellOptions<'_>) -> String {
    let title_e = escape(opts.title);
    let theme_class = theme_class_for_cookie(opts.theme_cookie);

    let crumbs = opts.breadcrumb.clone().unwrap_or_else(|| breadcrumb_for_path(opts.current_path));
    let crumbs_html = render_breadcrumb(&crumbs);

    // Persona-filter the default commands so TenantAdmin doesn't see
    // PlatformAdmin-only entries (Compliance / ADR / Upstream / live
    // cluster). Caller-supplied extras are appended unfiltered — the
    // caller already knows their audience.
    let mut commands = default_commands_for_persona(opts.tenant_id, opts.persona);
    commands.extend(opts.extra_commands.clone());
    let cmd_palette = command_palette_modal(&commands);
    let shortcuts = shortcuts_help_modal(DEFAULT_BINDINGS, opts.persona, opts.tenant_id);
    let toasts = toast_container();

    let topbar = topbar(opts.persona, opts.tenant_id, theme_class);
    let sidebar_html = if opts.hide_sidebar {
        String::new()
    } else {
        sidebar(opts.persona, opts.current_path, opts.tenant_id)
    };

    let main_classes = if opts.hide_sidebar {
        "px-4 pt-16 pb-8 max-w-6xl mx-auto"
    } else {
        "px-4 pt-16 pb-8 max-w-6xl mx-auto md:ml-56"
    };

    let footer_html = footer(opts.cluster_info);

    format!(
        r##"<!doctype html>
<html lang="en" class="{theme_class}">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title} — cave admin</title>
  <script src="/static/htmx.min.js" defer></script>
  <link rel="stylesheet" href="/static/tailwind-light.css">
  <style>
    :focus-visible {{ outline: 2px solid #3b82f6; outline-offset: 2px; }}
    [data-list-row][data-active] {{ background: rgba(59, 130, 246, 0.1); }}
    .dark [data-list-row][data-active] {{ background: rgba(59, 130, 246, 0.2); }}
    .sr-only {{ position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0,0,0,0); white-space: nowrap; border: 0; }}
  </style>
</head>
<body class="bg-gray-50 dark:bg-zinc-950 text-gray-900 dark:text-zinc-100 font-sans min-h-screen">
<a href="#main" class="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-50 focus:px-3 focus:py-2 focus:rounded focus:bg-blue-600 focus:text-white focus:outline-none focus:ring-2 focus:ring-blue-400">Skip to main content</a>
{topbar}
{sidebar}
<main id="main" class="{main_classes}" tabindex="-1">
  {breadcrumb}
  <h1 class="text-xl font-semibold mb-4">{title}</h1>
  {body}
</main>
{footer}
{cmd_palette}
{shortcuts}
{toasts}
</body>
</html>"##,
        theme_class = theme_class,
        title = title_e,
        topbar = topbar,
        sidebar = sidebar_html,
        main_classes = main_classes,
        breadcrumb = crumbs_html,
        body = opts.body,
        footer = footer_html,
        cmd_palette = cmd_palette,
        shortcuts = shortcuts,
        toasts = toasts,
    )
}

fn persona_label(p: Persona) -> &'static str {
    match p {
        Persona::PlatformAdmin => "Platform Admin",
        Persona::TenantAdmin => "Tenant Admin",
        Persona::Anonymous => "Anonymous",
    }
}

fn persona_badge_class(p: Persona) -> &'static str {
    match p {
        Persona::PlatformAdmin => "bg-purple-100 dark:bg-purple-900/50 text-purple-900 dark:text-purple-100",
        Persona::TenantAdmin => "bg-blue-100 dark:bg-blue-900/50 text-blue-900 dark:text-blue-100",
        Persona::Anonymous => "bg-zinc-100 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-300",
    }
}

fn topbar(persona: Persona, tenant_id: &str, theme_class: &str) -> String {
    let label = persona_label(persona);
    let badge_cls = persona_badge_class(persona);
    let tenant_e = escape(tenant_id);
    let _ = theme_class; // theme is on <html>; topbar shows toggle.
    format!(
        r#"<header class="fixed top-0 left-0 right-0 z-20 h-12 border-b dark:border-zinc-800 bg-white dark:bg-zinc-900 px-3 flex items-center justify-between">
  <div class="flex items-center gap-3">
    <button id="cave-mobile-menu" aria-label="Toggle navigation" class="md:hidden p-1 rounded hover:bg-zinc-100 dark:hover:bg-zinc-800">☰</button>
    <a href="/" class="font-semibold tracking-tight">cave</a>
    <button onclick="window.caveCmdk && window.caveCmdk.open()" class="hidden sm:flex items-center gap-2 px-3 py-1 text-sm bg-zinc-100 dark:bg-zinc-800 rounded text-zinc-600 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700 min-w-[20rem]">
      <span aria-hidden="true">🔍</span>
      <span>Search or jump to…</span>
      <span class="ml-auto text-[10px] uppercase tracking-wider">⌘K</span>
    </button>
  </div>
  <div class="flex items-center gap-2 text-xs">
    <span class="px-2 py-0.5 rounded {badge_cls}">{label}</span>
    <span class="text-zinc-500 dark:text-zinc-400">tenant: <code>{tenant}</code></span>
    <form method="post" action="/api/portal/theme/toggle" class="inline">
      <button type="submit" aria-label="Toggle dark mode" class="px-2 py-1 rounded hover:bg-zinc-100 dark:hover:bg-zinc-800">🌗</button>
    </form>
    <a href="/api/auth/logout" class="px-2 py-1 rounded hover:bg-zinc-100 dark:hover:bg-zinc-800">Sign out</a>
  </div>
</header>
<script>
(function() {{
  // Mobile menu toggle — drawer-style sidebar reveal.
  var btn = document.getElementById('cave-mobile-menu');
  var side = document.getElementById('cave-sidebar');
  if (btn && side) {{
    btn.addEventListener('click', function() {{
      side.classList.toggle('hidden');
      side.classList.toggle('!flex');
      side.classList.toggle('absolute');
      side.classList.toggle('top-12');
      side.classList.toggle('left-0');
      side.classList.toggle('w-56');
      side.classList.toggle('bottom-0');
      side.classList.toggle('z-30');
      side.classList.toggle('bg-white');
      side.classList.toggle('dark:bg-zinc-900');
    }});
  }}
}})();
</script>"#,
        badge_cls = badge_cls,
        label = label,
        tenant = tenant_e,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Persona;

    fn opts<'a>(persona: Persona, path: &'a str, tenant: &'a str, body: &'a str) -> ShellOptions<'a> {
        ShellOptions {
            title: "Test page",
            persona,
            tenant_id: tenant,
            current_path: path,
            theme_cookie: None,
            breadcrumb: None,
            extra_commands: Vec::new(),
            cluster_info: "1 node · v0.1.0",
            hide_sidebar: false,
            body,
        }
    }

    #[test]
    fn shell_v2_includes_viewport_meta_and_lang_attribute() {
        let html = shell_v2(opts(Persona::PlatformAdmin, "/admin/keda", "acme", "<p>hi</p>"));
        assert!(html.contains(r#"lang="en""#));
        assert!(html.contains(r#"name="viewport""#));
        assert!(html.contains("initial-scale=1"));
    }

    #[test]
    fn shell_v2_emits_topbar_persona_label() {
        let html = shell_v2(opts(Persona::PlatformAdmin, "/", "acme", ""));
        assert!(html.contains("Platform Admin"));
        let html = shell_v2(opts(Persona::TenantAdmin, "/", "tenant1", ""));
        assert!(html.contains("Tenant Admin"));
    }

    #[test]
    fn shell_v2_renders_sidebar_for_platform_admin() {
        let html = shell_v2(opts(Persona::PlatformAdmin, "/admin/keda", "acme", ""));
        assert!(html.contains("Compliance"));
        assert!(html.contains("KEDA"));
    }

    #[test]
    fn shell_v2_omits_sidebar_when_hide_sidebar_true() {
        let mut o = opts(Persona::PlatformAdmin, "/", "acme", "");
        o.hide_sidebar = true;
        let html = shell_v2(o);
        // No sidebar id present.
        assert!(!html.contains(r#"id="cave-sidebar""#));
    }

    #[test]
    fn shell_v2_includes_command_palette_and_shortcuts_help() {
        let html = shell_v2(opts(Persona::PlatformAdmin, "/", "acme", ""));
        assert!(html.contains(r#"id="cave-cmdk""#));
        assert!(html.contains(r#"id="cave-help""#));
        assert!(html.contains(r#"id="cave-toasts""#));
    }

    #[test]
    fn shell_v2_renders_breadcrumb_from_path() {
        let html = shell_v2(opts(Persona::PlatformAdmin, "/admin/keda", "acme", ""));
        assert!(html.contains("Home"));
        assert!(html.contains("Admin"));
        assert!(html.contains("KEDA"));
    }

    #[test]
    fn shell_v2_escapes_title_in_h1_and_meta_title() {
        let mut o = opts(Persona::Anonymous, "/", "x", "");
        o.title = "<bad>";
        let html = shell_v2(o);
        // Both occurrences are escaped.
        assert!(html.contains("&lt;bad&gt;"));
        assert!(!html.contains("<bad>"));
    }

    #[test]
    fn shell_v2_carries_theme_class_through_html_tag() {
        let mut o = opts(Persona::Anonymous, "/", "x", "");
        o.theme_cookie = Some("dark");
        let html = shell_v2(o);
        // <html class="dark">
        assert!(html.contains(r#"<html lang="en" class="dark">"#));
    }

    #[test]
    fn shell_v2_extra_commands_appear_in_palette_json() {
        let mut o = opts(Persona::PlatformAdmin, "/admin/keda", "acme", "");
        o.extra_commands = vec![CommandItem::action("Pause all ScaledObjects", "/admin/keda/pause")];
        let html = shell_v2(o);
        assert!(html.contains("Pause all ScaledObjects"));
    }

    #[test]
    fn shell_v2_emits_focus_visible_styles_for_keyboard_navigation() {
        let html = shell_v2(opts(Persona::PlatformAdmin, "/", "x", ""));
        assert!(html.contains(":focus-visible"));
    }

    #[test]
    fn shell_v2_main_margin_shifts_for_sidebar_layout() {
        let html_with_sidebar = shell_v2(opts(Persona::PlatformAdmin, "/", "x", ""));
        assert!(html_with_sidebar.contains("md:ml-56"));
        let mut o = opts(Persona::PlatformAdmin, "/", "x", "");
        o.hide_sidebar = true;
        let html_no_sidebar = shell_v2(o);
        assert!(!html_no_sidebar.contains("md:ml-56"));
    }

    // ── 2026-05-15 polish sweep — palette persona filter ────────────

    /// Whole-document smoke: when the shell renders with `Persona::TenantAdmin`,
    /// the embedded command-palette JSON must NOT carry the four
    /// platform-only entries (Compliance / Upstream / ADR / _audit).
    /// PlatformAdmin still gets all of them.
    #[test]
    fn shell_v2_palette_excludes_platform_entries_for_tenant_admin() {
        let html = shell_v2(opts(Persona::TenantAdmin, "/admin/keda", "tenant1", ""));
        // Palette JSON lives between the `cave-cmdk-data` script tags.
        // Crude check: the platform-only labels must not appear in
        // the command palette section of the HTML.
        let cmd_section = html
            .split(r#"id="cave-cmdk-data""#)
            .nth(1)
            .expect("palette data script present")
            .split("</script>")
            .next()
            .unwrap();
        for forbidden in [
            "Go to Compliance",
            "Go to ADR Browser",
            "Go to Upstream",
            "Go to Audit",
            "Go to Cluster Status",
        ] {
            assert!(
                !cmd_section.contains(forbidden),
                "TenantAdmin palette must NOT contain `{forbidden}`; got section: {cmd_section}"
            );
        }
        // KEDA is tenant-scoped, must still appear.
        assert!(cmd_section.contains("Go to KEDA"));
    }

    #[test]
    fn shell_v2_palette_includes_platform_entries_for_platform_admin() {
        let html = shell_v2(opts(Persona::PlatformAdmin, "/admin/keda", "acme", ""));
        let cmd_section = html
            .split(r#"id="cave-cmdk-data""#)
            .nth(1)
            .expect("palette data script present")
            .split("</script>")
            .next()
            .unwrap();
        for required in [
            "Go to Compliance",
            "Go to ADR Browser",
            "Go to Upstream",
            "Go to Audit",
        ] {
            assert!(
                cmd_section.contains(required),
                "PlatformAdmin palette MUST contain `{required}`"
            );
        }
    }
}
