use anyhow::{Context, Result};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'static str,
    aud: &'static str,
    sub: String,
    source: &'a str,
    jti: String,
    iat: u64,
    exp: u64,
    scope: &'static str,
}

pub struct IssuedToken {
    pub access_token: String,
    pub expires_at: u64,
}

pub fn issue_access_token(
    secret: &str,
    telegram_user_id: i64,
    source: &str,
    ttl_seconds: u64,
) -> Result<IssuedToken> {
    let issued_at = unix_now();
    let expires_at = issued_at.saturating_add(ttl_seconds);
    let claims = Claims {
        iss: "infraBot",
        aud: "infraCLI",
        sub: telegram_user_id.to_string(),
        source,
        jti: Uuid::new_v4().to_string(),
        iat: issued_at,
        exp: expires_at,
        scope: "events:write",
    };
    let header = Header::new(Algorithm::HS256);
    let access_token = encode(
        &header,
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .context("sign access token")?;

    Ok(IssuedToken {
        access_token,
        expires_at,
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issues_source_bound_token() {
        let secret = "x".repeat(32);
        let issued = issue_access_token(&secret, 42, "primary", 300).unwrap();
        assert!(!issued.access_token.is_empty());
        assert!(issued.expires_at > unix_now());
    }
}
