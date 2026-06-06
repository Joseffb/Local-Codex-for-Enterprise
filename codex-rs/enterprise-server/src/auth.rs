use anyhow::Context;
use anyhow::Result;
use argon2::Argon2;
use argon2::PasswordHash;
use argon2::PasswordHasher;
use argon2::PasswordVerifier;
use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
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

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .context("hash password with argon2")?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, encoded_hash: &str) -> Result<bool> {
    let parsed = match PasswordHash::new(encoded_hash) {
        Ok(parsed) => parsed,
        Err(_) => return Ok(false),
    };

    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

pub fn issue_api_token(label: &str) -> Result<IssuedApiToken> {
    let token = format!(
        "lce_{label}_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    Ok(IssuedApiToken {
        token_hash: api_token_hash(&token),
        plaintext: token,
    })
}

pub fn verify_api_token(plaintext: &str, expected_hash: &str) -> bool {
    api_token_hash(plaintext) == expected_hash
}

pub fn api_token_hash(plaintext: &str) -> String {
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
