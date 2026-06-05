use anyhow::Context;
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use jsonwebtoken::Algorithm;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use jsonwebtoken::Validation;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedApiToken {
    pub plaintext: String,
    pub token_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedHandoffToken {
    pub jwt: String,
    pub jti: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerHandoffClaims {
    pub sub: String,
    pub workspace_id: String,
    pub session_id: String,
    pub worker_id: String,
    pub exp: usize,
    pub jti: String,
    pub aud: String,
}

// Scaffold-only password hashing until the Argon2 dependency can be resolved
// and wired in. This must not ship as the production enterprise password path.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = Uuid::new_v4().to_string();
    Ok(format!(
        "sha256:{salt}:{}",
        hash_password_with_salt(password, &salt)
    ))
}

pub fn verify_password(password: &str, encoded_hash: &str) -> Result<bool> {
    let Some(("sha256", rest)) = encoded_hash.split_once(':') else {
        return Ok(false);
    };
    let Some((salt, expected_hash)) = rest.split_once(':') else {
        return Ok(false);
    };
    Ok(hash_password_with_salt(password, salt) == expected_hash)
}

fn hash_password_with_salt(password: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

pub fn issue_api_token(label: &str) -> Result<IssuedApiToken> {
    let token = format!(
        "lce_{label}_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    Ok(IssuedApiToken {
        token_hash: hash_api_token(&token),
        plaintext: token,
    })
}

pub fn verify_api_token(plaintext: &str, expected_hash: &str) -> bool {
    hash_api_token(plaintext) == expected_hash
}

fn hash_api_token(plaintext: &str) -> String {
    let digest = Sha256::digest(plaintext.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

pub fn issue_worker_handoff_token(
    user_id: &str,
    workspace_id: &str,
    session_id: &str,
    worker_id: &str,
    ttl: Duration,
    secret: &str,
) -> Result<IssuedHandoffToken> {
    let exp = Utc::now()
        .timestamp()
        .saturating_add(ttl.as_secs() as i64)
        .max(0) as usize;
    let jti = Uuid::new_v4().to_string();
    let claims = WorkerHandoffClaims {
        sub: user_id.to_string(),
        workspace_id: workspace_id.to_string(),
        session_id: session_id.to_string(),
        worker_id: worker_id.to_string(),
        exp,
        jti: jti.clone(),
        aud: "codex-worker-ws".to_string(),
    };
    let jwt = jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .context("encode worker handoff token")?;
    Ok(IssuedHandoffToken { jwt, jti })
}

pub fn decode_worker_handoff_token(jwt: &str, secret: &str) -> Result<WorkerHandoffClaims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&["codex-worker-ws"]);
    let token = jsonwebtoken::decode::<WorkerHandoffClaims>(
        jwt,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .context("decode worker handoff token")?;
    Ok(token.claims)
}
