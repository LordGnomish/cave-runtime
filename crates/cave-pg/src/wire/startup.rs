//! Startup phase — StartupMessage / SSLRequest / GSSENCRequest / Cancel.
//!
//! The first frame on a connection has **no type byte**; the codec reads the
//! length and the next int32 to discriminate:
//!
//! | first int32   | meaning                |
//! |---------------|------------------------|
//! | 196608 (0x30000) | StartupMessage v3.0 |
//! | 80877103      | SSLRequest             |
//! | 80877104      | GSSENCRequest          |
//! | 80877102      | CancelRequest          |
//!
//! After authentication the server sends `ParameterStatus`, `BackendKeyData`
//! and finally `ReadyForQuery`. Modeled on `pgwire::messages::startup`.

use bytes::{Buf, BufMut, BytesMut};

use super::codec::{
    FrontendMessage, ProtocolError, ProtocolResult, put_cstr, read_cstr,
};

pub const PROTOCOL_VERSION_3_0: i32 = 196608;
pub const SSL_REQUEST_CODE: i32 = 80877103;
pub const GSS_ENC_REQUEST_CODE: i32 = 80877104;
pub const CANCEL_REQUEST_CODE: i32 = 80877102;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupMessage {
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub parameters: Vec<(String, String)>,
}

impl StartupMessage {
    pub fn new(user: &str, database: &str) -> Self {
        Self {
            protocol_major: 3,
            protocol_minor: 0,
            parameters: vec![
                ("user".into(), user.into()),
                ("database".into(), database.into()),
            ],
        }
    }

    pub fn user(&self) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(k, _)| k == "user")
            .map(|(_, v)| v.as_str())
    }

    pub fn database(&self) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(k, _)| k == "database")
            .map(|(_, v)| v.as_str())
            .or_else(|| self.user())
    }

    pub fn application_name(&self) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(k, _)| k == "application_name")
            .map(|(_, v)| v.as_str())
    }

    pub fn options(&self) -> Vec<(&str, &str)> {
        self.parameters
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }
}

pub fn encode_startup(msg: &StartupMessage, buf: &mut BytesMut) {
    let len_pos = buf.len();
    buf.put_i32(0); // placeholder length (covers itself)
    let version = ((msg.protocol_major as i32) << 16) | (msg.protocol_minor as i32);
    buf.put_i32(version);
    for (k, v) in &msg.parameters {
        put_cstr(buf, k);
        put_cstr(buf, v);
    }
    buf.put_u8(0); // trailing nul
    let total_len = (buf.len() - len_pos) as i32;
    buf[len_pos..len_pos + 4].copy_from_slice(&total_len.to_be_bytes());
}

/// Parse the first frame on a connection: discriminates Startup, SSL, GSS,
/// and Cancel by inspecting the int32 that follows the length prefix.
pub fn decode_startup_frame(buf: &mut BytesMut) -> ProtocolResult<FrontendMessage> {
    if buf.len() < 8 {
        return Err(ProtocolError::Incomplete {
            needed: 8 - buf.len(),
        });
    }
    let len = i32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if len < 8 {
        return Err(ProtocolError::InvalidLength(len));
    }
    let total = len as usize;
    if buf.len() < total {
        return Err(ProtocolError::Incomplete {
            needed: total - buf.len(),
        });
    }
    let code = i32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let mut frame = buf.split_to(total);
    frame.advance(8);
    match code {
        SSL_REQUEST_CODE => Ok(FrontendMessage::SslRequest),
        GSS_ENC_REQUEST_CODE => Ok(FrontendMessage::GssEncRequest),
        CANCEL_REQUEST_CODE => {
            if frame.len() < 8 {
                return Err(ProtocolError::Incomplete {
                    needed: 8 - frame.len(),
                });
            }
            let process_id = frame.get_i32();
            let secret_key = frame.get_i32();
            Ok(FrontendMessage::Cancel {
                process_id,
                secret_key,
            })
        }
        version => {
            let major = (version >> 16) as u16;
            let minor = (version & 0xFFFF) as u16;
            if major != 3 {
                return Err(ProtocolError::UnsupportedVersion { major, minor });
            }
            let mut parameters = Vec::new();
            while !frame.is_empty() && frame[0] != 0 {
                let k = read_cstr(&mut frame)?;
                let v = read_cstr(&mut frame)?;
                parameters.push((k, v));
            }
            // optional trailing nul
            if !frame.is_empty() && frame[0] == 0 {
                frame.advance(1);
            }
            Ok(FrontendMessage::Startup(StartupMessage {
                protocol_major: major,
                protocol_minor: minor,
                parameters,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_round_trip() {
        let msg = StartupMessage::new("alice", "shop");
        let mut buf = BytesMut::new();
        encode_startup(&msg, &mut buf);
        let decoded = decode_startup_frame(&mut buf).unwrap();
        assert_eq!(decoded, FrontendMessage::Startup(msg));
        assert!(buf.is_empty());
    }

    #[test]
    fn startup_includes_application_name() {
        let mut msg = StartupMessage::new("u", "db");
        msg.parameters
            .push(("application_name".into(), "psql".into()));
        let mut buf = BytesMut::new();
        encode_startup(&msg, &mut buf);
        let decoded = decode_startup_frame(&mut buf).unwrap();
        match decoded {
            FrontendMessage::Startup(m) => {
                assert_eq!(m.application_name(), Some("psql"));
                assert_eq!(m.user(), Some("u"));
                assert_eq!(m.database(), Some("db"));
            }
            other => panic!("expected Startup, got {:?}", other),
        }
    }

    #[test]
    fn startup_database_falls_back_to_user() {
        let msg = StartupMessage {
            protocol_major: 3,
            protocol_minor: 0,
            parameters: vec![("user".into(), "alice".into())],
        };
        assert_eq!(msg.database(), Some("alice"));
    }

    #[test]
    fn ssl_request_decodes() {
        let mut buf = BytesMut::new();
        buf.put_i32(8);
        buf.put_i32(SSL_REQUEST_CODE);
        let decoded = decode_startup_frame(&mut buf).unwrap();
        assert_eq!(decoded, FrontendMessage::SslRequest);
    }

    #[test]
    fn gss_enc_request_decodes() {
        let mut buf = BytesMut::new();
        buf.put_i32(8);
        buf.put_i32(GSS_ENC_REQUEST_CODE);
        let decoded = decode_startup_frame(&mut buf).unwrap();
        assert_eq!(decoded, FrontendMessage::GssEncRequest);
    }

    #[test]
    fn cancel_request_decodes() {
        let mut buf = BytesMut::new();
        buf.put_i32(16);
        buf.put_i32(CANCEL_REQUEST_CODE);
        buf.put_i32(4711);
        buf.put_i32(987654321);
        let decoded = decode_startup_frame(&mut buf).unwrap();
        assert_eq!(
            decoded,
            FrontendMessage::Cancel {
                process_id: 4711,
                secret_key: 987654321,
            }
        );
    }

    #[test]
    fn unsupported_protocol_errors() {
        let mut buf = BytesMut::new();
        buf.put_i32(8);
        buf.put_i32(((4_i32) << 16) | 0);
        let err = decode_startup_frame(&mut buf).unwrap_err();
        assert_eq!(
            err,
            ProtocolError::UnsupportedVersion {
                major: 4,
                minor: 0
            }
        );
    }

    #[test]
    fn startup_invalid_length_errors() {
        let mut buf = BytesMut::new();
        buf.put_i32(4);
        buf.put_i32(PROTOCOL_VERSION_3_0);
        let err = decode_startup_frame(&mut buf).unwrap_err();
        assert_eq!(err, ProtocolError::InvalidLength(4));
    }

    #[test]
    fn startup_incomplete_errors() {
        let mut buf = BytesMut::from(&[0, 0, 0, 8][..]);
        let err = decode_startup_frame(&mut buf).unwrap_err();
        match err {
            ProtocolError::Incomplete { .. } => {}
            other => panic!("expected Incomplete, got {:?}", other),
        }
    }

    #[test]
    fn startup_with_many_options_round_trips() {
        let msg = StartupMessage {
            protocol_major: 3,
            protocol_minor: 0,
            parameters: vec![
                ("user".into(), "bob".into()),
                ("database".into(), "warehouse".into()),
                ("client_encoding".into(), "UTF8".into()),
                ("application_name".into(), "psql".into()),
                ("DateStyle".into(), "ISO, MDY".into()),
            ],
        };
        let mut buf = BytesMut::new();
        encode_startup(&msg, &mut buf);
        let decoded = decode_startup_frame(&mut buf).unwrap();
        assert_eq!(decoded, FrontendMessage::Startup(msg));
    }

    #[test]
    fn startup_back_to_back_with_query() {
        let mut buf = BytesMut::new();
        encode_startup(&StartupMessage::new("u", "db"), &mut buf);
        // simulate concurrent buffering — extra bytes should NOT be consumed
        buf.put_u8(0xFF);
        buf.put_u8(0xFE);
        let _ = decode_startup_frame(&mut buf).unwrap();
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn protocol_version_constant_correct() {
        assert_eq!(PROTOCOL_VERSION_3_0, 3 << 16);
    }
}
