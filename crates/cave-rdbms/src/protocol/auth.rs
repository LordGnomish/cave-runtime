//! Authentication methods.

#[derive(Debug, Clone)]
pub enum AuthMethod {
    Trust,
    MD5Password,
    CleartextPassword,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_methods_exist() {
        let _trust = AuthMethod::Trust;
        let _md5 = AuthMethod::MD5Password;
        let _clear = AuthMethod::CleartextPassword;
    }
}
