//! Startup message and SSL request handling.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct SSLRequest;

impl SSLRequest {
    pub fn parse_from_bytes(data: &[u8]) -> Result<SSLRequest, String> {
        if data.len() != 4 {
            return Err("SSLRequest must be 4 bytes".to_string());
        }
        // Bytes 0-1: 1234, Bytes 2-3: 5679
        let code = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        if code == 80877103 {
            Ok(SSLRequest)
        } else {
            Err("invalid SSL request code".to_string())
        }
    }
}

#[derive(Debug, Clone)]
pub struct StartupMessage {
    pub protocol_version: (u16, u16),
    pub params: HashMap<String, String>,
}

impl StartupMessage {
    pub fn parse_from_bytes(data: &[u8]) -> Result<StartupMessage, String> {
        if data.len() < 8 {
            return Err("startup message too short".to_string());
        }
        let major = u16::from_be_bytes([data[0], data[1]]);
        let minor = u16::from_be_bytes([data[2], data[3]]);
        let mut params = HashMap::new();
        let mut offset = 4;
        loop {
            if offset >= data.len() {
                return Err("startup message not null-terminated".to_string());
            }
            if data[offset] == 0 {
                break;
            }
            let key_start = offset;
            while offset < data.len() && data[offset] != 0 {
                offset += 1;
            }
            if offset >= data.len() {
                return Err("key not null-terminated".to_string());
            }
            let key = String::from_utf8(data[key_start..offset].to_vec())
                .map_err(|_| "invalid utf8 in key".to_string())?;
            offset += 1; // skip null
            let val_start = offset;
            while offset < data.len() && data[offset] != 0 {
                offset += 1;
            }
            if offset >= data.len() {
                return Err("value not null-terminated".to_string());
            }
            let value = String::from_utf8(data[val_start..offset].to_vec())
                .map_err(|_| "invalid utf8 in value".to_string())?;
            offset += 1; // skip null
            params.insert(key, value);
        }
        Ok(StartupMessage {
            protocol_version: (major, minor),
            params,
        })
    }

    pub fn user(&self) -> Option<&str> {
        self.params.get("user").map(|s| s.as_str())
    }

    pub fn database(&self) -> Option<&str> {
        self.params.get("database").map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssl_request_valid() {
        let data = 80877103u32.to_be_bytes();
        let req = SSLRequest::parse_from_bytes(&data);
        assert!(req.is_ok());
    }

    #[test]
    fn test_startup_message_parse() {
        let mut data = Vec::new();
        data.extend_from_slice(&3u16.to_be_bytes()); // major
        data.extend_from_slice(&0u16.to_be_bytes()); // minor
        data.extend_from_slice(b"user\0alice\0database\0postgres\0\0");
        let msg = StartupMessage::parse_from_bytes(&data).unwrap();
        assert_eq!(msg.user(), Some("alice"));
        assert_eq!(msg.database(), Some("postgres"));
    }
}
