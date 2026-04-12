//! PostgreSQL string functions.

use crate::error::{Error, PgError, Result, SqlState};
use crate::types::{oid, PgValue};
use regex::Regex;

fn require_text(name: &str, v: PgValue) -> Result<Option<String>> {
    match v {
        PgValue::Null => Ok(None),
        PgValue::Text(s) | PgValue::Varchar(s) | PgValue::Char(s) => Ok(Some(s)),
        other => Ok(Some(other.to_text())),
    }
}

pub fn length(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Text(s) | PgValue::Varchar(s) | PgValue::Char(s) => {
            Ok(PgValue::Int4(s.chars().count() as i32))
        }
        other => Ok(PgValue::Int4(other.to_text().chars().count() as i32)),
    }
}

pub fn octet_length(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Text(s) | PgValue::Varchar(s) | PgValue::Char(s) => {
            Ok(PgValue::Int4(s.len() as i32))
        }
        PgValue::Bytea(b) => Ok(PgValue::Int4(b.len() as i32)),
        other => Ok(PgValue::Int4(other.to_text().len() as i32)),
    }
}

pub fn bit_length(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Text(s) | PgValue::Varchar(s) => Ok(PgValue::Int4((s.len() * 8) as i32)),
        PgValue::Bytea(b) => Ok(PgValue::Int4((b.len() * 8) as i32)),
        other => Ok(PgValue::Int4((other.to_text().len() * 8) as i32)),
    }
}

pub fn upper(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Text(args[0].to_text().to_uppercase()))
}

pub fn lower(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Text(args[0].to_text().to_lowercase()))
}

pub fn initcap(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let result: String = s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + &chars.as_str().to_lowercase(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    Ok(PgValue::Text(result))
}

pub fn trim(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let chars_to_trim = if args.len() > 1 {
        args[1].to_text()
    } else {
        " ".to_string()
    };
    let result = s.trim_matches(|c: char| chars_to_trim.contains(c)).to_string();
    Ok(PgValue::Text(result))
}

pub fn ltrim(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let chars_to_trim = if args.len() > 1 { args[1].to_text() } else { " ".to_string() };
    Ok(PgValue::Text(s.trim_start_matches(|c: char| chars_to_trim.contains(c)).to_string()))
}

pub fn rtrim(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let chars_to_trim = if args.len() > 1 { args[1].to_text() } else { " ".to_string() };
    Ok(PgValue::Text(s.trim_end_matches(|c: char| chars_to_trim.contains(c)).to_string()))
}

pub fn lpad(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 { return Ok(PgValue::Null); }
    if args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let len = args[1].to_i64().unwrap_or(0) as usize;
    let pad = if args.len() > 2 { args[2].to_text() } else { " ".to_string() };
    if s.chars().count() >= len {
        return Ok(PgValue::Text(s.chars().take(len).collect()));
    }
    let need = len - s.chars().count();
    let pad_str: String = pad.chars().cycle().take(need).collect();
    Ok(PgValue::Text(pad_str + &s))
}

pub fn rpad(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 { return Ok(PgValue::Null); }
    if args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let len = args[1].to_i64().unwrap_or(0) as usize;
    let pad = if args.len() > 2 { args[2].to_text() } else { " ".to_string() };
    if s.chars().count() >= len {
        return Ok(PgValue::Text(s.chars().take(len).collect()));
    }
    let need = len - s.chars().count();
    let pad_str: String = pad.chars().cycle().take(need).collect();
    Ok(PgValue::Text(s + &pad_str))
}

pub fn left(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let n = args[1].to_i64().unwrap_or(0);
    let chars: Vec<char> = s.chars().collect();
    let result: String = if n >= 0 {
        chars.iter().take(n as usize).collect()
    } else {
        let skip = (-n) as usize;
        chars.iter().take(chars.len().saturating_sub(skip)).collect()
    };
    Ok(PgValue::Text(result))
}

pub fn right(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let n = args[1].to_i64().unwrap_or(0);
    let chars: Vec<char> = s.chars().collect();
    let result: String = if n >= 0 {
        chars.iter().rev().take(n as usize).collect::<Vec<_>>().into_iter().rev().collect()
    } else {
        let skip = (-n) as usize;
        chars.iter().skip(skip).collect()
    };
    Ok(PgValue::Text(result))
}

pub fn reverse(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Text(args[0].to_text().chars().rev().collect()))
}

pub fn repeat_str(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let n = args[1].to_i64().unwrap_or(0).max(0) as usize;
    Ok(PgValue::Text(s.repeat(n)))
}

pub fn substring(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let chars: Vec<char> = s.chars().collect();

    // Check for regex variant: substring(str FROM pattern)
    // In sqlparser this becomes a function call with 2 args where second is a regex
    // We treat both cases: substring(str, start [, len]) and substring(str FROM pattern)
    if args.len() == 2 {
        // Could be regex or numeric
        match &args[1] {
            PgValue::Text(pat) | PgValue::Varchar(pat) => {
                // Regex form: extract first match group or whole match
                match Regex::new(pat) {
                    Ok(re) => {
                        if let Some(cap) = re.captures(&s) {
                            let result = cap.get(1).unwrap_or(cap.get(0).unwrap()).as_str();
                            return Ok(PgValue::Text(result.to_string()));
                        }
                        return Ok(PgValue::Null);
                    }
                    Err(_) => {
                        // fall through to numeric
                    }
                }
            }
            _ => {}
        }
    }

    let start = if args.len() >= 2 {
        (args[1].to_i64().unwrap_or(1) - 1).max(0) as usize
    } else {
        0
    };
    let len = if args.len() >= 3 {
        args[2].to_i64().unwrap_or(0).max(0) as usize
    } else {
        chars.len()
    };
    let result: String = chars.iter().skip(start).take(len).collect();
    Ok(PgValue::Text(result))
}

pub fn position(args: Vec<PgValue>) -> Result<PgValue> {
    // position(substr IN str) → two args: [substr, str]
    if args.len() < 2 { return Ok(PgValue::Int4(0)); }
    if args[0].is_null() || args[1].is_null() { return Ok(PgValue::Null); }
    let substr = args[0].to_text();
    let s = args[1].to_text();
    match s.find(substr.as_str()) {
        None => Ok(PgValue::Int4(0)),
        Some(byte_pos) => {
            let char_pos = s[..byte_pos].chars().count() + 1;
            Ok(PgValue::Int4(char_pos as i32))
        }
    }
}

pub fn strpos(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 { return Ok(PgValue::Int4(0)); }
    if args[0].is_null() || args[1].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let substr = args[1].to_text();
    match s.find(substr.as_str()) {
        None => Ok(PgValue::Int4(0)),
        Some(byte_pos) => {
            let char_pos = s[..byte_pos].chars().count() + 1;
            Ok(PgValue::Int4(char_pos as i32))
        }
    }
}

pub fn replace(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 3 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let from = args[1].to_text();
    let to = args[2].to_text();
    Ok(PgValue::Text(s.replace(from.as_str(), &to)))
}

pub fn concat(args: Vec<PgValue>) -> Result<PgValue> {
    let mut result = String::new();
    for a in args {
        if !a.is_null() { result.push_str(&a.to_text()); }
    }
    Ok(PgValue::Text(result))
}

pub fn concat_ws(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let sep = args[0].to_text();
    let parts: Vec<String> = args[1..].iter()
        .filter(|a| !a.is_null())
        .map(|a| a.to_text())
        .collect();
    Ok(PgValue::Text(parts.join(&sep)))
}

pub fn split_part(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 3 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let delim = args[1].to_text();
    let n = args[2].to_i64().unwrap_or(1) as usize;
    let parts: Vec<&str> = s.split(delim.as_str()).collect();
    if n == 0 || n > parts.len() {
        Ok(PgValue::Text(String::new()))
    } else {
        Ok(PgValue::Text(parts[n - 1].to_string()))
    }
}

pub fn string_to_array(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let delim = args[1].to_text();
    let null_str = args.get(2).filter(|v| !v.is_null()).map(|v| v.to_text());
    let elements: Vec<PgValue> = if delim.is_empty() {
        s.chars().map(|c| PgValue::Text(c.to_string())).collect()
    } else {
        s.split(delim.as_str()).map(|part| {
            if let Some(ns) = &null_str {
                if part == ns.as_str() { return PgValue::Null; }
            }
            PgValue::Text(part.to_string())
        }).collect()
    };
    Ok(PgValue::Array { element_oid: oid::TEXT, elements })
}

pub fn array_to_string(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let delim = args[1].to_text();
    let null_str = args.get(2).filter(|v| !v.is_null()).map(|v| v.to_text());
    match &args[0] {
        PgValue::Array { elements, .. } => {
            let parts: Vec<String> = elements.iter().filter_map(|e| {
                if e.is_null() {
                    null_str.clone()
                } else {
                    Some(e.to_text())
                }
            }).collect();
            Ok(PgValue::Text(parts.join(&delim)))
        }
        _ => Ok(PgValue::Text(args[0].to_text())),
    }
}

pub fn regexp_match(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let pat = args[1].to_text();
    let flags = args.get(2).map(|v| v.to_text()).unwrap_or_default();
    let mut rb = regex::RegexBuilder::new(&pat);
    if flags.contains('i') { rb.case_insensitive(true); }
    if flags.contains('s') { rb.dot_matches_new_line(true); }
    match rb.build() {
        Err(e) => Err(Error::Pg(PgError::error(SqlState::INVALID_REGULAR_EXPRESSION, e.to_string()))),
        Ok(re) => {
            match re.captures(&s) {
                None => Ok(PgValue::Null),
                Some(caps) => {
                    let matches: Vec<PgValue> = (1..=re.captures_len().saturating_sub(1))
                        .map(|i| caps.get(i).map(|m| PgValue::Text(m.as_str().to_string())).unwrap_or(PgValue::Null))
                        .collect();
                    if matches.is_empty() {
                        // No capture groups — return whole match
                        Ok(PgValue::Array {
                            element_oid: oid::TEXT,
                            elements: vec![PgValue::Text(caps.get(0).unwrap().as_str().to_string())],
                        })
                    } else {
                        Ok(PgValue::Array { element_oid: oid::TEXT, elements: matches })
                    }
                }
            }
        }
    }
}

pub fn regexp_replace(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 3 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let pat = args[1].to_text();
    let repl = args[2].to_text();
    let flags = args.get(3).map(|v| v.to_text()).unwrap_or_default();
    let mut rb = regex::RegexBuilder::new(&pat);
    if flags.contains('i') { rb.case_insensitive(true); }
    match rb.build() {
        Err(e) => Err(Error::Pg(PgError::error(SqlState::INVALID_REGULAR_EXPRESSION, e.to_string()))),
        Ok(re) => {
            let result = if flags.contains('g') {
                re.replace_all(&s, repl.as_str()).to_string()
            } else {
                re.replace(&s, repl.as_str()).to_string()
            };
            Ok(PgValue::Text(result))
        }
    }
}

pub fn regexp_split_to_array(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let pat = args[1].to_text();
    let flags = args.get(2).map(|v| v.to_text()).unwrap_or_default();
    let mut rb = regex::RegexBuilder::new(&pat);
    if flags.contains('i') { rb.case_insensitive(true); }
    match rb.build() {
        Err(e) => Err(Error::Pg(PgError::error(SqlState::INVALID_REGULAR_EXPRESSION, e.to_string()))),
        Ok(re) => {
            let parts: Vec<PgValue> = re.split(&s).map(|p| PgValue::Text(p.to_string())).collect();
            Ok(PgValue::Array { element_oid: oid::TEXT, elements: parts })
        }
    }
}

pub fn encode(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let bytes = match &args[0] {
        PgValue::Bytea(b) => b.clone(),
        other => other.to_text().into_bytes(),
    };
    let fmt = args[1].to_text();
    match fmt.as_str() {
        "hex" => Ok(PgValue::Text(hex::encode(&bytes))),
        "base64" => {
            use base64::Engine as _;
            Ok(PgValue::Text(base64::engine::general_purpose::STANDARD.encode(&bytes)))
        }
        "escape" => {
            let mut s = String::new();
            for b in &bytes {
                if *b == b'\\' { s.push_str("\\\\"); }
                else if *b < 32 || *b > 126 {
                    s.push_str(&format!("\\{:03o}", b));
                } else {
                    s.push(*b as char);
                }
            }
            Ok(PgValue::Text(s))
        }
        _ => Err(Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE, format!("unrecognized encoding: {fmt}")))),
    }
}

pub fn decode(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let fmt = args[1].to_text();
    match fmt.as_str() {
        "hex" => {
            let bytes = hex::decode(s.trim())
                .map_err(|e| Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE, e.to_string())))?;
            Ok(PgValue::Bytea(bytes))
        }
        "base64" => {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD.decode(s.trim())
                .map_err(|e| Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE, e.to_string())))?;
            Ok(PgValue::Bytea(bytes))
        }
        "escape" => {
            Ok(PgValue::Bytea(s.into_bytes()))
        }
        _ => Err(Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE, format!("unrecognized encoding: {fmt}")))),
    }
}

pub fn md5(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    use md5::Digest;
    let mut h = md5::Md5::new();
    h.update(args[0].to_text().as_bytes());
    Ok(PgValue::Text(format!("{:x}", h.finalize())))
}

pub fn sha256(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    match &args[0] {
        PgValue::Bytea(b) => h.update(b),
        other => h.update(other.to_text().as_bytes()),
    }
    Ok(PgValue::Bytea(h.finalize().to_vec()))
}

pub fn format_str(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let fmt = args[0].to_text();
    let mut result = String::new();
    let mut arg_idx = 1usize;
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            i += 1;
            match chars[i] {
                's' | 'I' | 'L' => {
                    if arg_idx < args.len() {
                        let v = &args[arg_idx];
                        let s = if chars[i - 1] == 'I' {
                            format!("\"{}\"", v.to_text().replace('"', "\"\""))
                        } else if chars[i - 1] == 'L' {
                            format!("'{}'", v.to_text().replace('\'', "''"))
                        } else {
                            v.to_text()
                        };
                        result.push_str(&s);
                        arg_idx += 1;
                    }
                }
                '%' => result.push('%'),
                _ => { result.push('%'); result.push(chars[i]); }
            }
        } else {
            result.push(chars[i]);
        }
        i += 1;
    }
    Ok(PgValue::Text(result))
}

pub fn quote_ident(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let needs_quoting = s.chars().any(|c| !c.is_alphanumeric() && c != '_')
        || s.chars().next().map(|c| c.is_numeric()).unwrap_or(false)
        || s.is_empty();
    if needs_quoting {
        Ok(PgValue::Text(format!("\"{}\"", s.replace('"', "\"\""))))
    } else {
        Ok(PgValue::Text(s.to_lowercase()))
    }
}

pub fn quote_literal(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() { return Ok(PgValue::Null); }
    if args[0].is_null() { return Ok(PgValue::Text("NULL".to_string())); }
    let s = args[0].to_text();
    Ok(PgValue::Text(format!("'{}'", s.replace('\'', "''"))))
}

pub fn chr_fn(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let n = args[0].to_i64().unwrap_or(0) as u32;
    match char::from_u32(n) {
        Some(c) => Ok(PgValue::Text(c.to_string())),
        None => Err(Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE,
            format!("chr({n}) is not a valid character")))),
    }
}

pub fn ascii_fn(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    Ok(PgValue::Int4(s.chars().next().map(|c| c as i32).unwrap_or(0)))
}

pub fn translate(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 3 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let from: Vec<char> = args[1].to_text().chars().collect();
    let to: Vec<char> = args[2].to_text().chars().collect();
    let result: String = s.chars().filter_map(|c| {
        if let Some(pos) = from.iter().position(|&f| f == c) {
            to.get(pos).copied()
        } else {
            Some(c)
        }
    }).collect();
    Ok(PgValue::Text(result))
}

pub fn overlay(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 3 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let repl = args[1].to_text();
    let start = (args[2].to_i64().unwrap_or(1) - 1).max(0) as usize;
    let count = if args.len() > 3 {
        args[3].to_i64().unwrap_or(repl.chars().count() as i64) as usize
    } else {
        repl.chars().count()
    };
    let chars: Vec<char> = s.chars().collect();
    let mut result: Vec<char> = chars[..start.min(chars.len())].to_vec();
    result.extend(repl.chars());
    let skip_end = (start + count).min(chars.len());
    result.extend_from_slice(&chars[skip_end..]);
    Ok(PgValue::Text(result.iter().collect()))
}

pub fn starts_with(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() || args[1].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Bool(args[0].to_text().starts_with(args[1].to_text().as_str())))
}

pub fn ends_with_fn(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() || args[1].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Bool(args[0].to_text().ends_with(args[1].to_text().as_str())))
}

pub fn to_hex(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let n = args[0].to_i64().unwrap_or(0);
    Ok(PgValue::Text(format!("{n:x}")))
}

pub fn convert_encoding(args: Vec<PgValue>) -> Result<PgValue> {
    // No-op: we only support UTF-8
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(args[0].clone())
}
