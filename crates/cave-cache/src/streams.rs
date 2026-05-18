// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashMap;
use crate::engine::CacheEngine;
use crate::types::{CacheEntry, CacheError, CacheResult, CacheValue, StreamEntry};

fn current_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn generate_stream_id(millis: u64, seq: u64) -> String {
    format!("{}-{}", millis, seq)
}

fn parse_stream_id(id: &str) -> Option<(u64, u64)> {
    let mut parts = id.splitn(2, '-');
    let ms: u64 = parts.next()?.parse().ok()?;
    let seq: u64 = parts.next().unwrap_or("0").parse().ok()?;
    Some((ms, seq))
}

fn id_gt(a: &str, b: &str) -> bool {
    let (ams, aseq) = parse_stream_id(a).unwrap_or((0, 0));
    let (bms, bseq) = parse_stream_id(b).unwrap_or((0, 0));
    (ams, aseq) > (bms, bseq)
}

fn id_ge(a: &str, b: &str) -> bool {
    !id_gt(b, a)
}

impl CacheEngine {
    pub fn xadd(
        &self,
        key: &str,
        id: Option<&str>,
        fields: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> CacheResult<String> {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(key.to_string()).or_insert_with(|| CacheEntry {
            value: CacheValue::Stream(vec![]),
            expires_at: None,
            version: 0,
        });
        entry.version += 1;
        match &mut entry.value {
            CacheValue::Stream(stream) => {
                let new_id = if let Some(explicit_id) = id {
                    explicit_id.to_string()
                } else {
                    let ms = current_millis();
                    // Find max seq for this ms
                    let max_seq = stream
                        .iter()
                        .filter_map(|e| parse_stream_id(&e.id))
                        .filter(|(m, _)| *m == ms)
                        .map(|(_, s)| s)
                        .max()
                        .map(|s| s + 1)
                        .unwrap_or(0);
                    generate_stream_id(ms, max_seq)
                };
                stream.push(StreamEntry {
                    id: new_id.clone(),
                    fields,
                });
                Ok(new_id)
            }
            _ => Err(CacheError::WrongType),
        }
    }

    pub fn xread(
        &self,
        keys: &[(&str, &str)],
        count: Option<usize>,
    ) -> CacheResult<Vec<(String, Vec<StreamEntry>)>> {
        let mut store = self.store.lock().unwrap();
        let mut result = Vec::new();
        for (key, last_id) in keys {
            match Self::get_entry(&mut store, key) {
                None => {}
                Some(entry) => match &entry.value {
                    CacheValue::Stream(stream) => {
                        let entries: Vec<StreamEntry> = stream
                            .iter()
                            .filter(|e| id_gt(&e.id, last_id))
                            .take(count.unwrap_or(usize::MAX))
                            .cloned()
                            .collect();
                        if !entries.is_empty() {
                            result.push((key.to_string(), entries));
                        }
                    }
                    _ => return Err(CacheError::WrongType),
                },
            }
        }
        Ok(result)
    }

    pub fn xrange(
        &self,
        key: &str,
        start: &str,
        end: &str,
        count: Option<usize>,
    ) -> CacheResult<Vec<StreamEntry>> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(vec![]),
            Some(entry) => match &entry.value {
                CacheValue::Stream(stream) => {
                    let entries: Vec<StreamEntry> = stream
                        .iter()
                        .filter(|e| {
                            let after_start = start == "-" || id_ge(&e.id, start);
                            let before_end = end == "+" || id_ge(end, &e.id);
                            after_start && before_end
                        })
                        .take(count.unwrap_or(usize::MAX))
                        .cloned()
                        .collect();
                    Ok(entries)
                }
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn xlen(&self, key: &str) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(0),
            Some(entry) => match &entry.value {
                CacheValue::Stream(stream) => Ok(stream.len()),
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn xgroup_create(&self, key: &str, group: &str, id: &str) -> CacheResult<()> {
        // Ensure key exists as a stream
        {
            let mut store = self.store.lock().unwrap();
            let entry = store.entry(key.to_string()).or_insert_with(|| CacheEntry {
                value: CacheValue::Stream(vec![]),
                expires_at: None,
                version: 0,
            });
            match &entry.value {
                CacheValue::Stream(_) => {}
                _ => return Err(CacheError::WrongType),
            }
        }
        let mut groups = self.groups.lock().unwrap();
        let key_groups = groups.entry(key.to_string()).or_insert_with(HashMap::new);
        key_groups.insert(group.to_string(), id.to_string());
        Ok(())
    }
}
