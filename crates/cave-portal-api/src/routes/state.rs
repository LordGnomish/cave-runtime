//! Shared repo + commit cache used by attribution / activity / dora routes.
//!
//! Walking `git log --all --numstat` is the expensive step; we do it once per
//! 5 minutes and let every dashboard endpoint reuse the result.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;

use super::attribution::{collect_all_commits, CommitRecord};

const CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Default)]
struct Cache {
    records: Mutex<Option<(Instant, Arc<Vec<CommitRecord>>)>>,
}

impl Cache {
    fn get_or_load(&self, repo: &Path) -> Result<Arc<Vec<CommitRecord>>> {
        {
            let guard = self.records.lock().unwrap();
            if let Some((at, recs)) = guard.as_ref() {
                if at.elapsed() < CACHE_TTL {
                    return Ok(Arc::clone(recs));
                }
            }
        }
        let fresh = Arc::new(collect_all_commits(repo)?);
        let mut guard = self.records.lock().unwrap();
        *guard = Some((Instant::now(), Arc::clone(&fresh)));
        Ok(fresh)
    }

    fn invalidate(&self) {
        let mut guard = self.records.lock().unwrap();
        *guard = None;
    }
}

#[derive(Clone)]
pub struct PortalDataState {
    pub repo: PathBuf,
    cache: Arc<Cache>,
}

impl PortalDataState {
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self {
            repo: repo.into(),
            cache: Arc::new(Cache::default()),
        }
    }

    pub fn from_env() -> Self {
        let repo = std::env::var("CAVE_WORKSPACE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        Self::new(repo)
    }

    pub fn records(&self) -> Result<Arc<Vec<CommitRecord>>> {
        self.cache.get_or_load(&self.repo)
    }

    pub fn invalidate(&self) {
        self.cache.invalidate();
    }
}
