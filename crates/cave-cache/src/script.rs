use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ring::digest::{digest, SHA256};
use crate::engine::CacheEngine;
use crate::types::{CacheError, CacheResult};

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub struct ScriptEngine {
    scripts: Arc<Mutex<HashMap<String, String>>>,
}

impl ScriptEngine {
    pub fn new(scripts: Arc<Mutex<HashMap<String, String>>>) -> Self {
        Self { scripts }
    }

    pub fn load(&self, script: &str) -> String {
        let hash = digest(&SHA256, script.as_bytes());
        let sha = to_hex(hash.as_ref());
        let mut scripts = self.scripts.lock().unwrap();
        scripts.insert(sha.clone(), script.to_string());
        sha
    }

    pub fn eval(
        &self,
        script: &str,
        keys: Vec<String>,
        argv: Vec<String>,
        engine: &CacheEngine,
    ) -> CacheResult<serde_json::Value> {
        self.execute(script, keys, argv, engine)
    }

    pub fn evalsha(
        &self,
        sha: &str,
        keys: Vec<String>,
        argv: Vec<String>,
        engine: &CacheEngine,
    ) -> CacheResult<serde_json::Value> {
        let script = {
            let scripts = self.scripts.lock().unwrap();
            scripts
                .get(sha)
                .cloned()
                .ok_or_else(|| CacheError::Script(format!("script not found: {}", sha)))?
        };
        self.execute(&script, keys, argv, engine)
    }

    fn execute(
        &self,
        script: &str,
        keys: Vec<String>,
        argv: Vec<String>,
        engine: &CacheEngine,
    ) -> CacheResult<serde_json::Value> {
        let script = script.trim();

        // Handle: return KEYS[N]
        if let Some(rest) = script.strip_prefix("return KEYS[") {
            if let Some(idx_str) = rest.strip_suffix(']') {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    if idx >= 1 && idx <= keys.len() {
                        return Ok(serde_json::Value::String(keys[idx - 1].clone()));
                    }
                    return Err(CacheError::Script(format!("KEYS[{}] out of range", idx)));
                }
            }
        }

        // Handle: return ARGV[N]
        if let Some(rest) = script.strip_prefix("return ARGV[") {
            if let Some(idx_str) = rest.strip_suffix(']') {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    if idx >= 1 && idx <= argv.len() {
                        return Ok(serde_json::Value::String(argv[idx - 1].clone()));
                    }
                    return Err(CacheError::Script(format!("ARGV[{}] out of range", idx)));
                }
            }
        }

        // Handle: return redis.call('get', KEYS[1])
        if let Some(rest) = script.strip_prefix("return redis.call(") {
            let inner = rest.trim_end_matches(')');
            let parts = parse_redis_call(inner);
            if parts.is_empty() {
                return Err(CacheError::Script("invalid redis.call".to_string()));
            }
            let cmd = parts[0].to_lowercase();
            match cmd.as_str() {
                "get" => {
                    if parts.len() < 2 {
                        return Err(CacheError::Script("get requires key".to_string()));
                    }
                    let key = resolve_arg(&parts[1], &keys, &argv);
                    let val = engine.get(&key)?;
                    return Ok(match val {
                        None => serde_json::Value::Null,
                        Some(v) => serde_json::Value::String(
                            String::from_utf8(v).unwrap_or_default(),
                        ),
                    });
                }
                "set" => {
                    if parts.len() < 3 {
                        return Err(CacheError::Script("set requires key and value".to_string()));
                    }
                    let key = resolve_arg(&parts[1], &keys, &argv);
                    let val = resolve_arg(&parts[2], &keys, &argv).into_bytes();
                    engine.set(&key, val, None)?;
                    return Ok(serde_json::Value::String("OK".to_string()));
                }
                "del" => {
                    if parts.len() < 2 {
                        return Err(CacheError::Script("del requires key".to_string()));
                    }
                    let key = resolve_arg(&parts[1], &keys, &argv);
                    let count = engine.del(&[key.as_str()]);
                    return Ok(serde_json::Value::Number(count.into()));
                }
                _ => {
                    return Err(CacheError::Script(format!("unsupported command: {}", cmd)));
                }
            }
        }

        // Handle plain string literals: return "something"
        if let Some(rest) = script.strip_prefix("return \"") {
            if let Some(s) = rest.strip_suffix('"') {
                return Ok(serde_json::Value::String(s.to_string()));
            }
        }

        // Handle numeric return
        if let Some(rest) = script.strip_prefix("return ") {
            if let Ok(n) = rest.trim().parse::<i64>() {
                return Ok(serde_json::Value::Number(n.into()));
            }
        }

        Err(CacheError::Script(format!("unsupported script: {}", script)))
    }
}

fn resolve_arg(part: &str, keys: &[String], argv: &[String]) -> String {
    if let Some(rest) = part.strip_prefix("KEYS[") {
        if let Some(idx_str) = rest.strip_suffix(']') {
            if let Ok(idx) = idx_str.parse::<usize>() {
                if idx >= 1 && idx <= keys.len() {
                    return keys[idx - 1].clone();
                }
            }
        }
    }
    if let Some(rest) = part.strip_prefix("ARGV[") {
        if let Some(idx_str) = rest.strip_suffix(']') {
            if let Ok(idx) = idx_str.parse::<usize>() {
                if idx >= 1 && idx <= argv.len() {
                    return argv[idx - 1].clone();
                }
            }
        }
    }
    // Strip quotes if present
    if part.starts_with('\'') && part.ends_with('\'') {
        return part[1..part.len() - 1].to_string();
    }
    if part.starts_with('"') && part.ends_with('"') {
        return part[1..part.len() - 1].to_string();
    }
    part.to_string()
}

fn parse_redis_call(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut quote_char = ' ';
    for ch in inner.chars() {
        if in_quote {
            if ch == quote_char {
                in_quote = false;
                parts.push(current.clone());
                current.clear();
            } else {
                current.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            in_quote = true;
            quote_char = ch;
        } else if ch == ',' || ch == ' ' {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                parts.push(trimmed);
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        parts.push(trimmed);
    }
    parts
}
