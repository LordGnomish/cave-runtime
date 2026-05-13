//! Footer — cluster info, version, support links.

use crate::admin::render::escape;

/// Render the global footer. `cluster_info` is a short string the
/// caller composes (e.g. "3 nodes · leader: node1 · v0.1.0").
pub fn footer(cluster_info: &str) -> String {
    format!(
        r#"<footer class="border-t dark:border-zinc-800 bg-white dark:bg-zinc-900 text-xs text-zinc-500 dark:text-zinc-400 px-4 py-2 mt-8">
  <div class="max-w-6xl mx-auto flex items-center justify-between flex-wrap gap-2">
    <span>cave-runtime · {info}</span>
    <span class="flex gap-3">
      <a href="/docs/charter" class="hover:text-blue-600 dark:hover:text-blue-300">Charter</a>
      <a href="https://github.com/anthropic/cave-runtime/issues" target="_blank" rel="noopener" class="hover:text-blue-600 dark:hover:text-blue-300">Support ↗</a>
      <span>Apache-2.0</span>
    </span>
  </div>
</footer>"#,
        info = escape(cluster_info),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_includes_cluster_info_and_links() {
        let html = footer("3 nodes · leader: node1 · v0.1.0");
        assert!(html.contains("3 nodes · leader: node1"));
        assert!(html.contains("Charter"));
        assert!(html.contains("Support"));
        assert!(html.contains("Apache-2.0"));
    }

    #[test]
    fn footer_external_link_has_noopener_and_target_blank() {
        let html = footer("info");
        assert!(html.contains(r#"target="_blank""#));
        assert!(html.contains(r#"rel="noopener""#));
    }

    #[test]
    fn footer_escapes_cluster_info() {
        let html = footer(r#"<script>x</script>"#);
        assert!(!html.contains("<script>x</script>"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
