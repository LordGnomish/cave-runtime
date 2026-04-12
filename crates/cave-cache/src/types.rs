use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone)]
pub enum CacheValue {
    String(Vec<u8>),
    List(VecDeque<Vec<u8>>),
    Set(HashSet<Vec<u8>>),
    ZSet(Vec<(Vec<u8>, f64)>), // sorted by score
    Hash(HashMap<Vec<u8>, Vec<u8>>),
    Stream(Vec<StreamEntry>),
}

#[derive(Debug, Clone)]
pub struct StreamEntry {
    pub id: String, // "timestamp-seq"
    pub fields: Vec<(Vec<u8>, Vec<u8>)>,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub value: CacheValue,
    pub expires_at: Option<std::time::Instant>,
    pub version: u64,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum CacheError {
    #[error("wrong type")]
    WrongType,
    #[error("key not found")]
    NotFound,
    #[error("out of range")]
    OutOfRange,
    #[error("parse error: {0}")]
    Parse(String),
    #[error("script error: {0}")]
    Script(String),
    #[error("transaction aborted")]
    TxAborted,
    #[error("cluster error: {0}")]
    Cluster(String),
}

pub type CacheResult<T> = Result<T, CacheError>;
