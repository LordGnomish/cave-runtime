// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Geospatial commands: GEOADD, GEODIST, GEOHASH, GEOPOS, GEOSEARCH, GEOSEARCHSTORE.
//!
//! Geo data is stored as a Sorted Set with the geohash as the score.

use crate::db::Db;
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::{bytes_to_f64, bytes_to_i64, Entry, Value, ZSet};

// ── Constants ─────────────────────────────────────────────────────────────────

const GEO_LAT_MIN: f64 = -85.05112878;
const GEO_LAT_MAX: f64 = 85.05112878;
const GEO_LON_MIN: f64 = -180.0;
const GEO_LON_MAX: f64 = 180.0;
const EARTH_RADIUS_M: f64 = 6372797.560856;

// ── GEOADD ───────────────────────────────────────────────────────────────────

pub fn cmd_geoadd(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 5 { return Err(CacheError::wrong_arity("geoadd")); }
    let key = args[1].clone();

    let mut nx = false;
    let mut xx = false;
    let mut ch = false;
    let mut i = 2;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"NX" => { nx = true; i += 1; }
            b"XX" => { xx = true; i += 1; }
            b"CH" => { ch = true; i += 1; }
            _ => break,
        }
    }

    if (args.len() - i) % 3 != 0 {
        return Err(CacheError::Syntax);
    }

    let zset = match db.get_typed_mut(&key, "zset")? {
        Some(e) => match &mut e.value {
            Value::ZSet(z) => z as *mut ZSet,
            _ => unreachable!(),
        },
        None => {
            db.insert(key.clone(), Entry::new(Value::ZSet(ZSet::new())));
            match db.get_typed_mut(&key, "zset")? {
                Some(e) => match &mut e.value {
                    Value::ZSet(z) => z as *mut ZSet,
                    _ => unreachable!(),
                },
                None => unreachable!(),
            }
        }
    };
    let zset = unsafe { &mut *zset };

    let mut added = 0i64;
    let mut changed = 0i64;

    while i < args.len() {
        let lon = bytes_to_f64(&args[i]).ok_or(CacheError::NotFloat)?;
        let lat = bytes_to_f64(&args[i + 1]).ok_or(CacheError::NotFloat)?;
        let member = args[i + 2].clone();
        i += 3;

        if lon < GEO_LON_MIN || lon > GEO_LON_MAX || lat < GEO_LAT_MIN || lat > GEO_LAT_MAX {
            return Err(CacheError::generic("ERR invalid longitude,latitude pair"));
        }

        let old_exists = zset.score(&member).is_some();
        if nx && old_exists { continue; }
        if xx && !old_exists { continue; }

        let score = encode_geohash(lon, lat);
        let was_new = zset.add(member, score);
        if was_new { added += 1; } else { changed += 1; }
    }

    Ok(Resp::Integer(if ch { added + changed } else { added }))
}

// ── GEODIST ──────────────────────────────────────────────────────────────────

pub fn cmd_geodist(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 || args.len() > 5 { return Err(CacheError::wrong_arity("geodist")); }
    let unit = args.get(4).map(|u| parse_unit(u)).transpose()?.unwrap_or(1.0);

    let pos1 = get_geo_pos(db, &args[1], &args[2])?;
    let pos2 = get_geo_pos(db, &args[1], &args[3])?;

    match (pos1, pos2) {
        (Some((lon1, lat1)), Some((lon2, lat2))) => {
            let dist = haversine(lat1, lon1, lat2, lon2);
            Ok(Resp::BulkString(Some(format!("{:.4}", dist / unit).into_bytes())))
        }
        _ => Ok(Resp::nil()),
    }
}

fn get_geo_pos(db: &mut Db, key: &[u8], member: &[u8]) -> CacheResult<Option<(f64, f64)>> {
    match db.get_typed(key, "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(z) => {
                Ok(z.score(member).map(|score| decode_geohash(score)))
            }
            _ => unreachable!(),
        },
        None => Ok(None),
    }
}

// ── GEOHASH ──────────────────────────────────────────────────────────────────

pub fn cmd_geohash(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("geohash")); }
    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(z) => {
                let results: Vec<Resp> = args[2..].iter().map(|member| {
                    z.score(member).map(|score| {
                        let (lon, lat) = decode_geohash(score);
                        Resp::BulkString(Some(base32_encode_geohash(lat, lon).into_bytes()))
                    }).unwrap_or(Resp::nil())
                }).collect();
                Ok(Resp::Array(Some(results)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(args[2..].iter().map(|_| Resp::nil()).collect()))),
    }
}

// ── GEOPOS ───────────────────────────────────────────────────────────────────

pub fn cmd_geopos(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("geopos")); }
    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(z) => {
                let results: Vec<Resp> = args[2..].iter().map(|member| {
                    z.score(member).map(|score| {
                        let (lon, lat) = decode_geohash(score);
                        Resp::Array(Some(vec![
                            Resp::BulkString(Some(format!("{:.17}", lon).into_bytes())),
                            Resp::BulkString(Some(format!("{:.17}", lat).into_bytes())),
                        ]))
                    }).unwrap_or(Resp::nil_array())
                }).collect();
                Ok(Resp::Array(Some(results)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(args[2..].iter().map(|_| Resp::nil_array()).collect()))),
    }
}

// ── GEOSEARCH ────────────────────────────────────────────────────────────────

pub fn cmd_geosearch(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    geosearch_impl(args, db, None)
}

pub fn cmd_geosearchstore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 { return Err(CacheError::wrong_arity("geosearchstore")); }
    let dst = args[1].clone();
    geosearch_impl(&args[1..], db, Some(dst))
}

fn geosearch_impl(args: &[Vec<u8>], db: &mut Db, dst: Option<Vec<u8>>) -> CacheResult<Resp> {
    if args.len() < 6 { return Err(CacheError::wrong_arity("geosearch")); }
    let key = &args[1];

    let mut center_lon: f64 = 0.0;
    let mut center_lat: f64 = 0.0;
    let mut radius_m: f64 = 0.0;
    let mut is_circle = true;
    let mut count: Option<usize> = None;
    let mut ascending = true;
    let mut withcoord = false;
    let mut withdist = false;
    let mut withscore = false;

    let mut i = 2;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"FROMMEMBER" => {
                i += 1;
                let pos = get_geo_pos(db, key, &args[i])?
                    .ok_or_else(|| CacheError::generic("ERR Could not perform this operation on a key that doesn't exist"))?;
                center_lon = pos.0;
                center_lat = pos.1;
                i += 1;
            }
            b"FROMLONLAT" => {
                center_lon = bytes_to_f64(&args[i + 1]).ok_or(CacheError::NotFloat)?;
                center_lat = bytes_to_f64(&args[i + 2]).ok_or(CacheError::NotFloat)?;
                i += 3;
            }
            b"BYRADIUS" => {
                let r = bytes_to_f64(&args[i + 1]).ok_or(CacheError::NotFloat)?;
                let unit = parse_unit(&args[i + 2])?;
                radius_m = r * unit;
                is_circle = true;
                i += 3;
            }
            b"BYBOX" => {
                let w = bytes_to_f64(&args[i + 1]).ok_or(CacheError::NotFloat)?;
                let h = bytes_to_f64(&args[i + 2]).ok_or(CacheError::NotFloat)?;
                let unit = parse_unit(&args[i + 3])?;
                // Convert box to bounding circle for simplicity
                radius_m = ((w / 2.0).hypot(h / 2.0)) * unit;
                is_circle = false;
                i += 4;
            }
            b"ASC" => { ascending = true; i += 1; }
            b"DESC" => { ascending = false; i += 1; }
            b"COUNT" => { count = Some(bytes_to_i64(&args[i + 1]).ok_or(CacheError::NotInteger)? as usize); i += 2; }
            b"WITHCOORD" => { withcoord = true; i += 1; }
            b"WITHDIST" => { withdist = true; i += 1; }
            b"WITHSCORE" => { withscore = true; i += 1; }
            _ => { i += 1; }
        }
    }

    // Find all members within radius
    let members = match db.get_typed(key, "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(z) => z.iter_asc().map(|(m, s)| (m.clone(), s)).collect::<Vec<_>>(),
            _ => unreachable!(),
        },
        None => vec![],
    };

    let mut results: Vec<(Vec<u8>, f64, f64, f64)> = members.into_iter()
        .filter_map(|(member, score)| {
            let (lon, lat) = decode_geohash(score);
            let dist = haversine(center_lat, center_lon, lat, lon);
            if dist <= radius_m {
                Some((member, dist, lon, lat))
            } else {
                None
            }
        })
        .collect();

    if ascending {
        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    } else {
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    }

    if let Some(n) = count {
        results.truncate(n);
    }

    if let Some(dst_key) = dst {
        // Store as sorted set
        let mut new_zset = ZSet::new();
        for (member, _, lon, lat) in &results {
            new_zset.add(member.clone(), encode_geohash(*lon, *lat));
        }
        let len = new_zset.len() as i64;
        db.insert(dst_key, Entry::new(Value::ZSet(new_zset)));
        return Ok(Resp::Integer(len));
    }

    let resp: Vec<Resp> = results.into_iter().map(|(member, dist, lon, lat)| {
        if !withdist && !withcoord && !withscore {
            Resp::BulkString(Some(member))
        } else {
            let mut parts = vec![Resp::BulkString(Some(member))];
            if withdist {
                parts.push(Resp::BulkString(Some(format!("{:.4}", dist / 1000.0).into_bytes())));
            }
            if withcoord {
                parts.push(Resp::Array(Some(vec![
                    Resp::BulkString(Some(format!("{:.17}", lon).into_bytes())),
                    Resp::BulkString(Some(format!("{:.17}", lat).into_bytes())),
                ])));
            }
            Resp::Array(Some(parts))
        }
    }).collect();

    Ok(Resp::Array(Some(resp)))
}

// ── Geo math ─────────────────────────────────────────────────────────────────

/// Encode (lon, lat) as a 52-bit interleaved integer cast to f64.
pub fn encode_geohash(lon: f64, lat: f64) -> f64 {
    let lon_norm = (lon - GEO_LON_MIN) / (GEO_LON_MAX - GEO_LON_MIN);
    let lat_norm = (lat - GEO_LAT_MIN) / (GEO_LAT_MAX - GEO_LAT_MIN);

    let lon_bits = (lon_norm * (1u64 << 26) as f64) as u64 & ((1u64 << 26) - 1);
    let lat_bits = (lat_norm * (1u64 << 26) as f64) as u64 & ((1u64 << 26) - 1);

    let mut hash: u64 = 0;
    for i in 0..26 {
        hash |= ((lon_bits >> i) & 1) << (i * 2);
        hash |= ((lat_bits >> i) & 1) << (i * 2 + 1);
    }
    hash as f64
}

/// Decode a geohash score back to (lon, lat).
pub fn decode_geohash(score: f64) -> (f64, f64) {
    let hash = score as u64;
    let mut lon_bits: u64 = 0;
    let mut lat_bits: u64 = 0;

    for i in 0..26 {
        lon_bits |= ((hash >> (i * 2)) & 1) << i;
        lat_bits |= ((hash >> (i * 2 + 1)) & 1) << i;
    }

    let lon = (lon_bits as f64 / (1u64 << 26) as f64) * (GEO_LON_MAX - GEO_LON_MIN) + GEO_LON_MIN;
    let lat = (lat_bits as f64 / (1u64 << 26) as f64) * (GEO_LAT_MAX - GEO_LAT_MIN) + GEO_LAT_MIN;
    (lon, lat)
}

/// Haversine distance in meters.
pub fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = EARTH_RADIUS_M;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let lat1 = lat1.to_radians();
    let lat2 = lat2.to_radians();

    let a = (dlat / 2.0).sin().powi(2) + (dlon / 2.0).sin().powi(2) * lat1.cos() * lat2.cos();
    let c = 2.0 * a.sqrt().asin();
    r * c
}

fn parse_unit(u: &[u8]) -> CacheResult<f64> {
    match u.to_ascii_lowercase().as_slice() {
        b"m" => Ok(1.0),
        b"km" => Ok(1000.0),
        b"mi" => Ok(1609.344),
        b"ft" => Ok(0.3048),
        _ => Err(CacheError::generic("ERR unsupported unit provided. please use M, KM, FT, MI")),
    }
}

/// Base32 encode for geohash display format.
fn base32_encode_geohash(lat: f64, lon: f64) -> String {
    const BASE32: &[u8] = b"0123456789bcdefghjkmnpqrstuvwxyz";
    let hash = encode_geohash(lon, lat) as u64;
    let mut result = String::new();
    // Use top 40 bits for an 8-character geohash
    for i in (0..8).rev() {
        let idx = ((hash >> (i * 5)) & 0x1F) as usize;
        result.push(BASE32[idx] as char);
    }
    result
}
