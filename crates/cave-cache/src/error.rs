// SPDX-License-Identifier: AGPL-3.0-or-later
//! Error types for cave-cache.

use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum CacheError {
    #[error("WRONGTYPE Operation against a key holding the wrong kind of value")]
    WrongType,
    #[error("ERR no such key")]
    NotFound,
    #[error("ERR value is not an integer or out of range")]
    NotInteger,
    #[error("ERR value is not a valid float")]
    NotFloat,
    #[error("ERR index out of range")]
    OutOfRange,
    #[error("ERR {0}")]
    Generic(String),
    #[error("NOSCRIPT No matching script. Please use EVAL.")]
    NoScript,
    #[error("EXECABORT Transaction discarded because of previous errors.")]
    ExecAbort,
    #[error("ERR EXEC without MULTI")]
    ExecWithoutMulti,
    #[error("ERR DISCARD without MULTI")]
    DiscardWithoutMulti,
    #[error("ERR Command not allowed inside a transaction")]
    NotInMulti,
    #[error("ERR wrong number of arguments for '{0}' command")]
    WrongArity(String),
    #[error("ERR syntax error")]
    Syntax,
    #[error("ERR bit offset not an integer or out of range")]
    BitOffset,
    #[error("ERR bit is not an integer or out of range")]
    BitValue,
    #[error("ERR invalid expire time in '{0}' command")]
    InvalidExpire(String),
    #[error("ERR DB index is out of range")]
    InvalidDb,
    #[error("ERR no such consumer group")]
    NoGroup,
    #[error("ERR The ID specified in XADD is equal or smaller than the target stream top item")]
    StreamIdTooSmall,
    #[error("ERR no elements")]
    Empty,
    #[error("LOADING Redis is loading the dataset in memory")]
    Loading,
    #[error("NOAUTH Authentication required.")]
    NoAuth,
    #[error("WRONGPASS invalid username-password pair or user is disabled.")]
    WrongPass,
    #[error("ERR Protocol error: {0}")]
    Protocol(String),
    #[error("ERR I/O error")]
    Io,
    #[error("MOVED {slot} {addr}")]
    Moved { slot: u16, addr: String },
    #[error("ASK {slot} {addr}")]
    Ask { slot: u16, addr: String },
    #[error("CLUSTERDOWN The cluster is down")]
    ClusterDown,
}

impl CacheError {
    pub fn generic(msg: impl Into<String>) -> Self {
        CacheError::Generic(msg.into())
    }

    pub fn wrong_arity(cmd: &str) -> Self {
        CacheError::WrongArity(cmd.to_ascii_lowercase())
    }

    /// Convert to RESP error string (without "-" prefix).
    pub fn to_resp_error(&self) -> String {
        self.to_string()
    }
}

pub type CacheResult<T> = Result<T, CacheError>;
