/// Configuration for CacheEngine (for future extensibility)
#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub max_memory_bytes: Option<usize>,
    pub eviction_policy: EvictionPolicy,
}

#[derive(Debug, Clone, Default)]
pub enum EvictionPolicy {
    #[default]
    NoEviction,
    AllKeysLru,
    VolatileLru,
    AllKeysRandom,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: None,
            eviction_policy: EvictionPolicy::NoEviction,
        }
    }
}
