use std::error::Error;
use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::server::auth::permissions::Role;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JwtClaims {
    pub sub: String,
    pub username: String,
    pub role: Role,
    pub source: String,
    pub iat: i64,
    pub exp: i64,
    pub iss: String,
}

#[derive(Debug, Clone)]
pub struct JwtIssuer {
    issuer: String,
    secret: Vec<u8>,
    ttl_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct ApiTokenIssuer {
    hash_secret: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct IssuedApiToken {
    pub raw: String,
    pub prefix: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    InvalidSecret,
    InvalidFormat,
    InvalidSignature,
    Expired,
    Json(String),
}

impl JwtIssuer {
    pub fn new(
        issuer: impl Into<String>,
        secret: impl AsRef<str>,
        ttl_seconds: u64,
    ) -> Result<Self, TokenError> {
        let secret = secret.as_ref().trim();
        if secret.is_empty() || ttl_seconds == 0 {
            return Err(TokenError::InvalidSecret);
        }
        Ok(Self {
            issuer: issuer.into(),
            secret: secret.as_bytes().to_vec(),
            ttl_seconds,
        })
    }

    pub fn issue(
        &self,
        sub: &str,
        username: &str,
        role: Role,
        source: &str,
    ) -> Result<String, TokenError> {
        let now = Utc::now().timestamp();
        let claims = JwtClaims {
            sub: sub.to_owned(),
            username: username.to_owned(),
            role,
            source: source.to_owned(),
            iat: now,
            exp: now + self.ttl_seconds as i64,
            iss: self.issuer.clone(),
        };
        let header = serde_json::json!({ "alg": "HS256", "typ": "JWT" });
        let header = encode_json(&header)?;
        let payload = encode_json(&claims)?;
        let signing_input = format!("{header}.{payload}");
        let signature = sign(&self.secret, signing_input.as_bytes())?;
        Ok(format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(signature)
        ))
    }

    pub fn verify(&self, token: &str) -> Result<JwtClaims, TokenError> {
        let parts = token.split('.').collect::<Vec<_>>();
        if parts.len() != 3 {
            return Err(TokenError::InvalidFormat);
        }
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let expected = sign(&self.secret, signing_input.as_bytes())?;
        let provided = URL_SAFE_NO_PAD
            .decode(parts[2])
            .map_err(|_| TokenError::InvalidFormat)?;
        if expected.ct_eq(&provided).unwrap_u8() != 1 {
            return Err(TokenError::InvalidSignature);
        }
        let payload = URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| TokenError::InvalidFormat)?;
        let claims = serde_json::from_slice::<JwtClaims>(&payload)
            .map_err(|err| TokenError::Json(err.to_string()))?;
        if claims.iss != self.issuer {
            return Err(TokenError::InvalidFormat);
        }
        if claims.exp <= Utc::now().timestamp() {
            return Err(TokenError::Expired);
        }
        Ok(claims)
    }
}

impl ApiTokenIssuer {
    pub fn new(hash_secret: impl AsRef<str>) -> Result<Self, TokenError> {
        let hash_secret = hash_secret.as_ref().trim();
        if hash_secret.is_empty() {
            return Err(TokenError::InvalidSecret);
        }
        Ok(Self {
            hash_secret: hash_secret.as_bytes().to_vec(),
        })
    }

    pub fn issue(&self) -> Result<IssuedApiToken, TokenError> {
        let prefix = format!("pvk_{}", compact_uuid());
        let secret = format!("{}.{}", compact_uuid(), compact_uuid());
        let raw = format!("{prefix}.{secret}");
        let hash = self.hash(&raw)?;
        Ok(IssuedApiToken { raw, prefix, hash })
    }

    pub fn hash(&self, raw: &str) -> Result<String, TokenError> {
        let digest = sign(&self.hash_secret, raw.as_bytes())?;
        Ok(URL_SAFE_NO_PAD.encode(digest))
    }

    pub fn verify(&self, raw: &str, hash: &str) -> bool {
        let Ok(actual) = self.hash(raw) else {
            return false;
        };
        actual.as_bytes().ct_eq(hash.as_bytes()).unwrap_u8() == 1
    }
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSecret => f.write_str("invalid token secret"),
            Self::InvalidFormat => f.write_str("invalid token format"),
            Self::InvalidSignature => f.write_str("invalid token signature"),
            Self::Expired => f.write_str("token expired"),
            Self::Json(err) => write!(f, "token json error: {err}"),
        }
    }
}

impl Error for TokenError {}

fn encode_json(value: &impl Serialize) -> Result<String, TokenError> {
    let bytes = serde_json::to_vec(value).map_err(|err| TokenError::Json(err.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn sign(secret: &[u8], message: &[u8]) -> Result<Vec<u8>, TokenError> {
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| TokenError::InvalidSecret)?;
    mac.update(message);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn compact_uuid() -> String {
    let digest = Sha256::digest(Uuid::new_v4().as_bytes());
    URL_SAFE_NO_PAD.encode(&digest[..12])
}
