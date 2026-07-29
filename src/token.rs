use anyhow::{Context, Result, bail};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Deserialize, Serialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: String,
    source: String,
    jti: String,
    iat: u64,
    exp: u64,
    scope: String,
}

pub struct IssuedToken {
    pub access_token: String,
    pub expires_at: u64,
}

pub struct AuthorizedSource {
    pub telegram_user_id: i64,
    pub source: String,
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
        iss: "infraBot".to_owned(),
        aud: "infraCLI".to_owned(),
        sub: telegram_user_id.to_string(),
        source: source.to_owned(),
        jti: Uuid::new_v4().to_string(),
        iat: issued_at,
        exp: expires_at,
        scope: "events:write".to_owned(),
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

pub fn verify_access_token(secret: &str, access_token: &str) -> Result<AuthorizedSource> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&["infraCLI"]);
    validation.set_issuer(&["infraBot"]);
    let claims = decode::<Claims>(
        access_token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .context("verify access token")?
    .claims;

    if !claims
        .scope
        .split_whitespace()
        .any(|scope| scope == "events:write")
    {
        bail!("access token does not grant events:write");
    }
    let telegram_user_id = claims
        .sub
        .parse::<i64>()
        .context("access token subject is not a Telegram user id")?;
    if claims.source.is_empty() {
        bail!("access token has no source");
    }

    Ok(AuthorizedSource {
        telegram_user_id,
        source: claims.source,
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
    fn issues_and_verifies_source_bound_token() {
        let secret = "x".repeat(32);
        let issued = issue_access_token(&secret, 42, "primary", 300).unwrap();
        let authorized = verify_access_token(&secret, &issued.access_token).unwrap();
        assert_eq!(authorized.telegram_user_id, 42);
        assert_eq!(authorized.source, "primary");
        assert!(issued.expires_at > unix_now());
    }

    #[test]
    fn rejects_token_signed_by_another_key() {
        let issued = issue_access_token(&"x".repeat(32), 42, "primary", 300).unwrap();
        assert!(verify_access_token(&"y".repeat(32), &issued.access_token).is_err());
    }
}
