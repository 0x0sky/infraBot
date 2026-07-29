use anyhow::{bail, Context, Result};
use std::{collections::HashSet, env, net::SocketAddr};

#[derive(Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub telegram_bot_token: String,
    pub telegram_bot_username: String,
    pub telegram_webhook_secret: String,
    pub signing_secret: String,
    pub allowed_user_ids: HashSet<i64>,
    pub pairing_ttl_seconds: u64,
    pub access_token_ttl_seconds: u64,
    pub poll_interval_seconds: u64,
    pub max_active_pairings: usize,
    pub max_exchange_attempts: u8,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind_addr = env_value("BIND_ADDR")
            .unwrap_or_else(|| "0.0.0.0:8787".to_owned())
            .parse()
            .context("BIND_ADDR must be a socket address")?;
        let telegram_bot_token = required("TELEGRAM_BOT_TOKEN")?;
        let telegram_bot_username = required("TELEGRAM_BOT_USERNAME")?
            .trim_start_matches('@')
            .to_owned();
        let telegram_webhook_secret = required("TELEGRAM_WEBHOOK_SECRET")?;
        let signing_secret = required("INFRABOT_SIGNING_SECRET")?;
        let allowed_user_ids = parse_allowed_users(&required("TELEGRAM_ALLOWED_USER_IDS")?)?;

        if telegram_bot_username.is_empty()
            || !telegram_bot_username
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || value == '_')
        {
            bail!("TELEGRAM_BOT_USERNAME is invalid");
        }
        if !(16..=256).contains(&telegram_webhook_secret.len())
            || !telegram_webhook_secret
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-'))
        {
            bail!("TELEGRAM_WEBHOOK_SECRET must be 16-256 URL-safe characters");
        }
        if signing_secret.as_bytes().len() < 32 {
            bail!("INFRABOT_SIGNING_SECRET must contain at least 32 bytes");
        }

        Ok(Self {
            bind_addr,
            telegram_bot_token,
            telegram_bot_username,
            telegram_webhook_secret,
            signing_secret,
            allowed_user_ids,
            pairing_ttl_seconds: parse_u64("PAIRING_TTL_SECONDS", 300, 60, 900)?,
            access_token_ttl_seconds: parse_u64(
                "ACCESS_TOKEN_TTL_SECONDS",
                2_592_000,
                300,
                31_536_000,
            )?,
            poll_interval_seconds: parse_u64("PAIRING_POLL_INTERVAL_SECONDS", 2, 1, 10)?,
            max_active_pairings: parse_usize("MAX_ACTIVE_PAIRINGS", 1_000, 1, 10_000)?,
            max_exchange_attempts: parse_u8("MAX_EXCHANGE_ATTEMPTS", 20, 3, 100)?,
        })
    }
}

fn required(name: &str) -> Result<String> {
    env_value(name).with_context(|| format!("{name} is required"))
}

fn env_value(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn parse_allowed_users(value: &str) -> Result<HashSet<i64>> {
    let users = value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::parse::<i64>)
        .collect::<std::result::Result<HashSet<_>, _>>()
        .context("TELEGRAM_ALLOWED_USER_IDS must be comma-separated integers")?;

    if users.is_empty() {
        bail!("TELEGRAM_ALLOWED_USER_IDS must contain at least one user id");
    }
    Ok(users)
}

fn parse_u64(name: &str, default: u64, minimum: u64, maximum: u64) -> Result<u64> {
    let value = env_value(name)
        .map(|raw| raw.parse::<u64>())
        .transpose()
        .with_context(|| format!("{name} must be an integer"))?
        .unwrap_or(default);
    if !(minimum..=maximum).contains(&value) {
        bail!("{name} must be between {minimum} and {maximum}");
    }
    Ok(value)
}

fn parse_usize(name: &str, default: usize, minimum: usize, maximum: usize) -> Result<usize> {
    let value = env_value(name)
        .map(|raw| raw.parse::<usize>())
        .transpose()
        .with_context(|| format!("{name} must be an integer"))?
        .unwrap_or(default);
    if !(minimum..=maximum).contains(&value) {
        bail!("{name} must be between {minimum} and {maximum}");
    }
    Ok(value)
}

fn parse_u8(name: &str, default: u8, minimum: u8, maximum: u8) -> Result<u8> {
    let value = env_value(name)
        .map(|raw| raw.parse::<u8>())
        .transpose()
        .with_context(|| format!("{name} must be an integer"))?
        .unwrap_or(default);
    if !(minimum..=maximum).contains(&value) {
        bail!("{name} must be between {minimum} and {maximum}");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_allowed_user_ids() {
        let users = parse_allowed_users("42, 77").unwrap();
        assert!(users.contains(&42));
        assert!(users.contains(&77));
    }

    #[test]
    fn rejects_empty_allowlist() {
        assert!(parse_allowed_users(" , ").is_err());
    }
}
