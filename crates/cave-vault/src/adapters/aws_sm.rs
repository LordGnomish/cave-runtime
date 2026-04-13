//! AWS Secrets Manager adapter.
//!
//! Delegates secret operations to AWS Secrets Manager via the JSON HTTP API,
//! signed with AWS Signature V4 (no SDK dependency — uses reqwest + sha2/hmac).
//!
//! # Configuration
//!
//! ```toml
//! [vault]
//! backend    = "aws_secrets_manager"
//! aws_region = "us-east-1"
//! aws_prefix = "/cave/"   # optional path prefix for all secret names
//! # Credentials via env: AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_SESSION_TOKEN
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::backend::{SecretValue, SecretsEngine, SecretsError, SecretsResult};

#[derive(Debug, Clone, Deserialize)]
pub struct AwsSecretsManagerConfig {
    pub region: String,
    /// Prefix prepended to all secret names, e.g. `/cave/prod/`.
    pub prefix: String,
}

impl AwsSecretsManagerConfig {
    fn endpoint(&self) -> String {
        format!("https://secretsmanager.{}.amazonaws.com/", self.region)
    }
}

// ─── AWS Sig V4 ────────────────────────────────────────────────────────────

struct AwsCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

impl AwsCredentials {
    fn from_env() -> Option<Self> {
        Some(Self {
            access_key_id: std::env::var("AWS_ACCESS_KEY_ID").ok()?,
            secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").ok()?,
            session_token: std::env::var("AWS_SESSION_TOKEN").ok(),
        })
    }
}

fn sign_request(
    creds: &AwsCredentials,
    region: &str,
    body: &str,
    target: &str,
) -> HashMap<String, String> {
    use hmac::Mac;
    use sha2::Digest;

    let now = chrono::Utc::now();
    let date_stamp = now.format("%Y%m%d").to_string();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

    let host = format!("secretsmanager.{}.amazonaws.com", region);
    let service = "secretsmanager";

    let body_hash = format!("{:x}", sha2::Sha256::digest(body.as_bytes()));

    let canonical_headers_base = format!(
        "content-type:application/x-amz-json-1.1\nhost:{host}\nx-amz-date:{amz_date}\nx-amz-target:{target}\n"
    );
    let signed_headers_base = "content-type;host;x-amz-date;x-amz-target";

    let (canonical_headers, signed_headers) = if let Some(ref tok) = creds.session_token {
        (
            format!("{}x-amz-security-token:{tok}\n", canonical_headers_base),
            format!("{};x-amz-security-token", signed_headers_base),
        )
    } else {
        (canonical_headers_base, signed_headers_base.to_string())
    };

    let canonical_request =
        format!("POST\n/\n\n{canonical_headers}\n{signed_headers}\n{body_hash}");

    let credential_scope = format!("{date_stamp}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{:x}",
        sha2::Sha256::digest(canonical_request.as_bytes())
    );

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(
        format!("AWS4{}", creds.secret_access_key).as_bytes(),
    )
    .unwrap();
    mac.update(date_stamp.as_bytes());
    let dk = mac.finalize().into_bytes();

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&dk).unwrap();
    mac.update(region.as_bytes());
    let rk = mac.finalize().into_bytes();

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&rk).unwrap();
    mac.update(service.as_bytes());
    let sk = mac.finalize().into_bytes();

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&sk).unwrap();
    mac.update(b"aws4_request");
    let signing_key = mac.finalize().into_bytes();

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&signing_key).unwrap();
    mac.update(string_to_sign.as_bytes());
    let signature = format!("{:x}", mac.finalize().into_bytes());

    let auth = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        creds.access_key_id, credential_scope, signed_headers, signature
    );

    let mut headers = HashMap::new();
    headers.insert("Authorization".into(), auth);
    headers.insert("x-amz-date".into(), amz_date);
    if let Some(tok) = &creds.session_token {
        headers.insert("x-amz-security-token".into(), tok.clone());
    }
    headers
}

// ─── Response types ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetSecretValueResponse {
    secret_string: Option<String>,
    version_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListSecretsResponse {
    secret_list: Vec<SecretListEntry>,
    next_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SecretListEntry {
    name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct CreateSecretRequest<'a> {
    name: &'a str,
    secret_string: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct PutSecretValueRequest<'a> {
    secret_id: &'a str,
    secret_string: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct DeleteSecretRequest<'a> {
    secret_id: &'a str,
    /// Skip the 30-day recovery window.
    force_delete_without_recovery: bool,
}

// ─── Adapter ─────────────────────────────────────────────────────────────────

/// AWS Secrets Manager adapter.
pub struct AwsSecretsManagerAdapter {
    config: AwsSecretsManagerConfig,
    client: reqwest::Client,
}

impl AwsSecretsManagerAdapter {
    pub fn new(config: AwsSecretsManagerConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    fn secret_name(&self, path: &str) -> String {
        format!("{}{}", self.config.prefix, path)
    }

    /// POST a JSON body to Secrets Manager with Sig V4 auth.
    async fn call(
        &self,
        creds: &AwsCredentials,
        target: &str,
        body: &str,
    ) -> SecretsResult<bytes::Bytes> {
        let sig_headers = sign_request(creds, &self.config.region, body, target);

        let mut req = self
            .client
            .post(self.config.endpoint())
            .header("Content-Type", "application/x-amz-json-1.1")
            .header("x-amz-target", target)
            .body(body.to_string());

        for (k, v) in &sig_headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| SecretsError::EngineError(format!("SM request failed: {e}")))?;

        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| SecretsError::EngineError(format!("SM response read failed: {e}")))?;

        if status == 404 {
            // ResourceNotFoundException comes back as 400 in SM; 404 is unexpected but handle it.
            let msg = String::from_utf8_lossy(&bytes).to_string();
            return Err(SecretsError::EngineError(format!("SM 404: {msg}")));
        }

        if status == 400 {
            // Check for ResourceNotFoundException in the error body.
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let code = v
                    .get("__type")
                    .or_else(|| v.get("code"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                if code.contains("ResourceNotFoundException") {
                    return Err(SecretsError::NotFound {
                        path: "unknown (SM 400 ResourceNotFound)".into(),
                    });
                }
                let msg = v
                    .get("Message")
                    .or_else(|| v.get("message"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                return Err(SecretsError::EngineError(format!("SM error ({code}): {msg}")));
            }
        }

        if !status.is_success() {
            let msg = String::from_utf8_lossy(&bytes).to_string();
            return Err(SecretsError::EngineError(format!("SM returned {status}: {msg}")));
        }

        Ok(bytes)
    }
}

#[async_trait]
impl SecretsEngine for AwsSecretsManagerAdapter {
    async fn read(&self, path: &str) -> SecretsResult<SecretValue> {
        let Some(creds) = AwsCredentials::from_env() else {
            return Err(SecretsError::EngineError(
                "AWS credentials not found. Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY.".into(),
            ));
        };

        let name = self.secret_name(path);
        let body = serde_json::json!({ "SecretId": name }).to_string();

        let bytes = self
            .call(&creds, "secretsmanager.GetSecretValue", &body)
            .await
            .map_err(|e| match e {
                SecretsError::NotFound { .. } => SecretsError::NotFound { path: path.to_string() },
                other => other,
            })?;

        let gsvr: GetSecretValueResponse = serde_json::from_slice(&bytes)
            .map_err(|e| SecretsError::EngineError(format!("SM parse error: {e}")))?;

        let secret_str = gsvr.secret_string.ok_or_else(|| {
            SecretsError::EngineError(format!("SM secret '{name}' has no SecretString (binary secret not supported)"))
        })?;

        // Try to parse as JSON object first (our write format), fall back to raw string.
        let data = if secret_str.trim_start().starts_with('{') {
            serde_json::from_str::<HashMap<String, String>>(&secret_str).unwrap_or_else(|_| {
                let mut m = HashMap::new();
                m.insert("value".into(), secret_str.clone());
                m
            })
        } else {
            let mut m = HashMap::new();
            m.insert("value".into(), secret_str);
            m
        };

        // VersionId is a UUID; use its length as a version proxy (always 36).
        let version = gsvr.version_id.as_deref().map(|v| v.len() as u64);

        Ok(SecretValue {
            data,
            version,
            lease_id: None,
            lease_duration: None,
            renewable: false,
        })
    }

    async fn write(&self, path: &str, data: HashMap<String, String>) -> SecretsResult<()> {
        let Some(creds) = AwsCredentials::from_env() else {
            return Err(SecretsError::EngineError(
                "AWS credentials not found.".into(),
            ));
        };

        let name = self.secret_name(path);
        let secret_string = serde_json::to_string(&data)
            .map_err(|e| SecretsError::EngineError(format!("Serialization error: {e}")))?;

        // Try PutSecretValue first; fall back to CreateSecret if not found.
        let put_body = serde_json::to_string(&PutSecretValueRequest {
            secret_id: &name,
            secret_string: &secret_string,
        })
        .unwrap();

        let result = self.call(&creds, "secretsmanager.PutSecretValue", &put_body).await;

        match result {
            Ok(_) => Ok(()),
            Err(SecretsError::NotFound { .. }) => {
                // Secret doesn't exist yet — create it.
                let create_body = serde_json::to_string(&CreateSecretRequest {
                    name: &name,
                    secret_string: &secret_string,
                })
                .unwrap();
                self.call(&creds, "secretsmanager.CreateSecret", &create_body).await?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    async fn delete(&self, path: &str) -> SecretsResult<()> {
        let Some(creds) = AwsCredentials::from_env() else {
            return Err(SecretsError::EngineError(
                "AWS credentials not found.".into(),
            ));
        };

        let name = self.secret_name(path);
        let body = serde_json::to_string(&DeleteSecretRequest {
            secret_id: &name,
            force_delete_without_recovery: true,
        })
        .unwrap();

        match self.call(&creds, "secretsmanager.DeleteSecret", &body).await {
            Ok(_) | Err(SecretsError::NotFound { .. }) => Ok(()), // idempotent
            Err(e) => Err(e),
        }
    }

    async fn list(&self, path: &str) -> SecretsResult<Vec<String>> {
        let Some(creds) = AwsCredentials::from_env() else {
            return Err(SecretsError::EngineError(
                "AWS credentials not found.".into(),
            ));
        };

        let prefix = format!("{}{}", self.config.prefix, path);
        let mut names: Vec<String> = Vec::new();
        let mut next_token: Option<String> = None;

        loop {
            let mut req_body = serde_json::json!({
                "Filters": [{ "Key": "name", "Values": [&prefix] }],
                "MaxResults": 100,
            });
            if let Some(ref tok) = next_token {
                req_body["NextToken"] = serde_json::Value::String(tok.clone());
            }

            let bytes = self
                .call(&creds, "secretsmanager.ListSecrets", &req_body.to_string())
                .await?;

            let list: ListSecretsResponse = serde_json::from_slice(&bytes)
                .map_err(|e| SecretsError::EngineError(format!("SM list parse error: {e}")))?;

            for entry in list.secret_list {
                // Strip the prefix to return the logical path.
                let logical = entry
                    .name
                    .strip_prefix(&self.config.prefix)
                    .unwrap_or(&entry.name)
                    .to_string();
                names.push(logical);
            }

            next_token = list.next_token;
            if next_token.is_none() {
                break;
            }
        }

        Ok(names)
    }

    fn name(&self) -> &'static str {
        "aws-secrets-manager"
    }
}
