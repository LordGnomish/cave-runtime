// SPDX-License-Identifier: AGPL-3.0-or-later
//! Centralised filesystem paths.
//!
//! In production cave-cri stores container state under `/var/lib/cave/`. Tests
//! and sandboxed runs override the root by setting the `CAVE_ROOT_DIR`
//! environment variable.

use std::path::PathBuf;

const DEFAULT_ROOT: &str = "/var/lib/cave";
const ENV_VAR: &str = "CAVE_ROOT_DIR";

/// Root directory for all cave-cri persistent state. Reads `CAVE_ROOT_DIR`
/// if set, otherwise returns `/var/lib/cave`.
pub fn root() -> PathBuf {
    std::env::var(ENV_VAR)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_ROOT))
}

/// Per-container subtree: `<root>/containers/<container_id>`.
pub fn container_dir(container_id: &str) -> PathBuf {
    root().join("containers").join(container_id)
}

/// Default log path for a container: `<root>/log/containers/<container_id>.log`.
pub fn container_log_path(container_id: &str) -> PathBuf {
    root().join("log").join("containers").join(format!("{}.log", container_id))
}

/// Image cache directory: `<root>/images`.
pub fn image_cache_dir() -> PathBuf {
    root().join("images")
}

/// Default checkpoint directory for a container: `<root>/checkpoints/<container_id>`.
pub fn checkpoint_dir(container_id: &str) -> PathBuf {
    root().join("checkpoints").join(container_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: scoped env var that resets on drop. Only used inside tests
    /// — `cargo test` runs inside the same process so we serialise via a mutex.
    fn with_root<F: FnOnce()>(value: &str, f: F) {
        use std::sync::Mutex;
        static GUARD: Mutex<()> = Mutex::new(());
        let _g = GUARD.lock().unwrap();
        let prev = std::env::var(ENV_VAR).ok();
        std::env::set_var(ENV_VAR, value);
        f();
        match prev {
            Some(v) => std::env::set_var(ENV_VAR, v),
            None => std::env::remove_var(ENV_VAR),
        }
    }

    #[test]
    fn root_uses_default_when_env_unset() {
        // Note: don't use with_root here so we don't perturb the real env.
        // We just check that *if* the env is unset, we get the default.
        let prev = std::env::var(ENV_VAR).ok();
        std::env::remove_var(ENV_VAR);
        assert_eq!(root(), PathBuf::from(DEFAULT_ROOT));
        if let Some(v) = prev {
            std::env::set_var(ENV_VAR, v);
        }
    }

    #[test]
    fn root_uses_env_var_when_set() {
        with_root("/tmp/cave-test-root", || {
            assert_eq!(root(), PathBuf::from("/tmp/cave-test-root"));
        });
    }

    #[test]
    fn container_dir_includes_id() {
        with_root("/tmp/cave-test-cd", || {
            let p = container_dir("abc-123");
            assert!(p.starts_with("/tmp/cave-test-cd"));
            assert!(p.ends_with("abc-123"));
            assert!(p.to_string_lossy().contains("containers"));
        });
    }

    #[test]
    fn container_log_path_format() {
        with_root("/tmp/cave-test-log", || {
            let p = container_log_path("c1");
            assert!(p.to_string_lossy().ends_with("c1.log"));
            assert!(p.to_string_lossy().contains("log/containers"));
        });
    }

    #[test]
    fn image_cache_dir_under_root() {
        with_root("/tmp/cave-test-img", || {
            assert_eq!(image_cache_dir(), PathBuf::from("/tmp/cave-test-img/images"));
        });
    }

    #[test]
    fn checkpoint_dir_includes_id() {
        with_root("/tmp/cave-test-ckpt", || {
            let p = checkpoint_dir("ckpt-id-1");
            assert!(p.ends_with("ckpt-id-1"));
            assert!(p.to_string_lossy().contains("checkpoints"));
        });
    }

    #[test]
    fn root_with_relative_path_preserved_as_given() {
        with_root("relative/path", || {
            assert_eq!(root(), PathBuf::from("relative/path"));
        });
    }
}
