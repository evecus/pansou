//! JWT 认证工具

use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub username: String,
    pub exp: i64,
    pub iat: i64,
    pub iss: String,
}

pub fn generate_token(username: &str, secret: &str, expiry: Duration) -> anyhow::Result<String> {
    if username.is_empty() || secret.is_empty() {
        anyhow::bail!("username and secret cannot be empty");
    }
    let now = Utc::now().timestamp();
    let claims = Claims {
        username: username.to_string(),
        exp: now + expiry.as_secs() as i64,
        iat: now,
        iss: "pansou".to_string(),
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;
    Ok(token)
}

pub fn validate_token(token: &str, secret: &str) -> anyhow::Result<Claims> {
    if token.is_empty() || secret.is_empty() {
        anyhow::bail!("token and secret cannot be empty");
    }
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )?;
    Ok(data.claims)
}
