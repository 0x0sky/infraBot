use anyhow::{Context, Result, bail};
use reqwest::Url;
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    net::SocketAddr,
    path::PathBuf,
};

#[derive(Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub public_url: String,
    pub telegram_bot_token: String,
    pub telegram_bot_username: String,
    pub telegram_webhook_secret: String,
    pub signing_secret: String,
    pub allowed_user_ids: HashSet<i64>,
    pub subscriber_user_ids: HashSet<i64>,
    pub subscriber_store: PathBuf,
    pub subscriber_events: HashSet<String>,
    pub sources: HashMap<String, String>,
    pub recipients: Vec<RecipientConfig>,
    pub pairing_ttl_seconds: u64,
    pub access_token_ttl_seconds: u64,
    pub poll_interval_seconds: u64,
    pub max_active_pairings: usize,
    pub max_exchange_attempts: u8,
}

#[derive(Clone)]
pub struct RecipientConfig {
    pub id: String,
    pub chat_id: i64,
    pub events: HashSet<String>,
}

#[derive(Clone)]
enum ValueRef {
    Literal(String),
    Environment(String),
}

#[derive(Default)]
struct InfraBotBuilder {
    bind: Option<ValueRef>,
    public_url: Option<ValueRef>,
    pairing_ttl_seconds: Option<u64>,
    access_token_ttl_seconds: Option<u64>,
    poll_interval_seconds: Option<u64>,
    max_active_pairings: Option<usize>,
    max_exchange_attempts: Option<u8>,
    telegram: Option<TelegramBuilder>,
    sources: Vec<SourceBuilder>,
    recipients: Vec<RecipientBuilder>,
}

#[derive(Default)]
struct TelegramBuilder {
    bot_username: Option<ValueRef>,
    bot_token: Option<ValueRef>,
    webhook_secret: Option<ValueRef>,
    signing_secret: Option<ValueRef>,
    allowed_user_ids: Option<ValueRef>,
    subscriber_user_ids: Option<ValueRef>,
    subscriber_store: Option<ValueRef>,
    subscriber_events: Option<Vec<String>>,
}

struct SourceBuilder {
    id: String,
    address: Option<ValueRef>,
}

struct RecipientBuilder {
    id: String,
    chat_id: Option<ValueRef>,
    events: Option<Vec<String>>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = env::var_os("INFRABOT_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".infra"));
        let input = fs::read_to_string(&path)
            .with_context(|| format!("read infraBot configuration from {}", path.display()))?;
        parse_document(&input, &|name| env_value(name))
            .with_context(|| format!("parse infraBot configuration from {}", path.display()))
    }
}

fn parse_document<F>(input: &str, resolver: &F) -> Result<Config>
where
    F: Fn(&str) -> Option<String>,
{
    let lines = significant_lines(input);
    let Some(start) = lines.iter().position(|(_, line)| *line == "infrabot {") else {
        bail!("missing infrabot block");
    };
    let mut index = start + 1;
    let builder = parse_infrabot(&lines, &mut index)?;
    build_config(builder, resolver)
}

fn significant_lines(input: &str) -> Vec<(usize, &str)> {
    input
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            (!line.is_empty() && !line.starts_with('#') && !line.starts_with("//"))
                .then_some((index + 1, line))
        })
        .collect()
}

fn parse_infrabot(lines: &[(usize, &str)], index: &mut usize) -> Result<InfraBotBuilder> {
    let mut builder = InfraBotBuilder::default();
    while let Some((line_number, line)) = lines.get(*index).copied() {
        *index += 1;
        if line == "}" {
            return Ok(builder);
        }
        if line == "telegram {" {
            if builder.telegram.is_some() {
                bail!("line {line_number}: duplicate telegram block");
            }
            builder.telegram = Some(parse_telegram(lines, index)?);
            continue;
        }
        if let Some(id) = named_block(line, "source") {
            builder.sources.push(parse_source(lines, index, id)?);
            continue;
        }
        if let Some(id) = named_block(line, "recipient") {
            builder.recipients.push(parse_recipient(lines, index, id)?);
            continue;
        }

        let (key, value) = assignment(line_number, line)?;
        match key {
            "bind" => set_once(
                &mut builder.bind,
                value_ref(line_number, value)?,
                line_number,
                key,
            )?,
            "public_url" => set_once(
                &mut builder.public_url,
                value_ref(line_number, value)?,
                line_number,
                key,
            )?,
            "pairing_ttl_seconds" => set_once(
                &mut builder.pairing_ttl_seconds,
                integer(line_number, value)?,
                line_number,
                key,
            )?,
            "access_token_ttl_seconds" => set_once(
                &mut builder.access_token_ttl_seconds,
                integer(line_number, value)?,
                line_number,
                key,
            )?,
            "poll_interval_seconds" => set_once(
                &mut builder.poll_interval_seconds,
                integer(line_number, value)?,
                line_number,
                key,
            )?,
            "max_active_pairings" => set_once(
                &mut builder.max_active_pairings,
                integer(line_number, value)?,
                line_number,
                key,
            )?,
            "max_exchange_attempts" => set_once(
                &mut builder.max_exchange_attempts,
                integer(line_number, value)?,
                line_number,
                key,
            )?,
            _ => bail!("line {line_number}: unknown infrabot field {key}"),
        }
    }
    bail!("infrabot block is not closed")
}

fn parse_telegram(lines: &[(usize, &str)], index: &mut usize) -> Result<TelegramBuilder> {
    let mut builder = TelegramBuilder::default();
    while let Some((line_number, line)) = lines.get(*index).copied() {
        *index += 1;
        if line == "}" {
            return Ok(builder);
        }
        let (key, raw) = assignment(line_number, line)?;
        match key {
            "subscriber_events" => set_once(
                &mut builder.subscriber_events,
                string_list(line_number, raw)?,
                line_number,
                key,
            )?,
            "bot_username" => set_once(
                &mut builder.bot_username,
                value_ref(line_number, raw)?,
                line_number,
                key,
            )?,
            "bot_token" => set_once(
                &mut builder.bot_token,
                value_ref(line_number, raw)?,
                line_number,
                key,
            )?,
            "webhook_secret" => set_once(
                &mut builder.webhook_secret,
                value_ref(line_number, raw)?,
                line_number,
                key,
            )?,
            "signing_secret" => set_once(
                &mut builder.signing_secret,
                value_ref(line_number, raw)?,
                line_number,
                key,
            )?,
            "allowed_user_ids" => set_once(
                &mut builder.allowed_user_ids,
                value_ref(line_number, raw)?,
                line_number,
                key,
            )?,
            "subscriber_user_ids" => set_once(
                &mut builder.subscriber_user_ids,
                value_ref(line_number, raw)?,
                line_number,
                key,
            )?,
            "subscriber_store" => set_once(
                &mut builder.subscriber_store,
                value_ref(line_number, raw)?,
                line_number,
                key,
            )?,
            _ => bail!("line {line_number}: unknown telegram field {key}"),
        }
    }
    bail!("telegram block is not closed")
}

fn parse_source(lines: &[(usize, &str)], index: &mut usize, id: String) -> Result<SourceBuilder> {
    validate_identifier("source", &id)?;
    let mut builder = SourceBuilder { id, address: None };
    while let Some((line_number, line)) = lines.get(*index).copied() {
        *index += 1;
        if line == "}" {
            return Ok(builder);
        }
        let (key, value) = assignment(line_number, line)?;
        match key {
            "address" => set_once(
                &mut builder.address,
                value_ref(line_number, value)?,
                line_number,
                key,
            )?,
            _ => bail!("line {line_number}: unknown source field {key}"),
        }
    }
    bail!("source block is not closed")
}

fn parse_recipient(
    lines: &[(usize, &str)],
    index: &mut usize,
    id: String,
) -> Result<RecipientBuilder> {
    validate_identifier("recipient", &id)?;
    let mut builder = RecipientBuilder {
        id,
        chat_id: None,
        events: None,
    };
    while let Some((line_number, line)) = lines.get(*index).copied() {
        *index += 1;
        if line == "}" {
            return Ok(builder);
        }
        let (key, value) = assignment(line_number, line)?;
        match key {
            "chat_id" => set_once(
                &mut builder.chat_id,
                value_ref(line_number, value)?,
                line_number,
                key,
            )?,
            "events" => set_once(
                &mut builder.events,
                string_list(line_number, value)?,
                line_number,
                key,
            )?,
            _ => bail!("line {line_number}: unknown recipient field {key}"),
        }
    }
    bail!("recipient block is not closed")
}

fn build_config<F>(builder: InfraBotBuilder, resolver: &F) -> Result<Config>
where
    F: Fn(&str) -> Option<String>,
{
    let telegram = builder.telegram.context("missing telegram block")?;
    let bind_addr = resolve_or(builder.bind, "0.0.0.0:8787", "bind", resolver)?
        .parse()
        .context("bind must be a socket address")?;
    let public_url = resolve_required(builder.public_url, "public_url", resolver)?;
    validate_url("public_url", &public_url)?;

    let telegram_bot_username =
        resolve_required(telegram.bot_username, "telegram.bot_username", resolver)?
            .trim_start_matches('@')
            .to_owned();
    if telegram_bot_username.is_empty()
        || !telegram_bot_username
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || value == '_')
    {
        bail!("telegram.bot_username is invalid");
    }

    let telegram_bot_token = resolve_secret(telegram.bot_token, "telegram.bot_token", resolver)?;
    let telegram_webhook_secret =
        resolve_secret(telegram.webhook_secret, "telegram.webhook_secret", resolver)?;
    let signing_secret =
        resolve_secret(telegram.signing_secret, "telegram.signing_secret", resolver)?;
    let allowed_user_ids = parse_i64_set(&resolve_required(
        telegram.allowed_user_ids,
        "telegram.allowed_user_ids",
        resolver,
    )?)
    .context("telegram.allowed_user_ids must be comma-separated integers")?;

    if allowed_user_ids.is_empty() {
        bail!("telegram.allowed_user_ids must contain at least one user id");
    }
    let subscriber_user_ids = match telegram.subscriber_user_ids {
        Some(value) => parse_i64_set(&value.resolve("telegram.subscriber_user_ids", resolver)?)
            .context("telegram.subscriber_user_ids must be comma-separated integers")?,
        None => allowed_user_ids.clone(),
    };
    if subscriber_user_ids.is_empty() {
        bail!("telegram.subscriber_user_ids must contain at least one user id");
    }
    let subscriber_store = PathBuf::from(resolve_or(
        telegram.subscriber_store,
        "/var/lib/infrabot/subscribers.json",
        "telegram.subscriber_store",
        resolver,
    )?);
    if !subscriber_store.is_absolute() {
        bail!("telegram.subscriber_store must be an absolute path");
    }
    let subscriber_events = telegram
        .subscriber_events
        .context("telegram.subscriber_events is required")?
        .into_iter()
        .collect::<HashSet<_>>();
    if subscriber_events.is_empty() {
        bail!("telegram.subscriber_events must contain at least one event");
    }
    if !(16..=256).contains(&telegram_webhook_secret.len())
        || !telegram_webhook_secret
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-'))
    {
        bail!("telegram.webhook_secret must be 16-256 URL-safe characters");
    }
    if signing_secret.len() < 32 {
        bail!("telegram.signing_secret must contain at least 32 bytes");
    }

    let mut sources = HashMap::new();
    for source in builder.sources {
        let address = resolve_required(source.address, "source.address", resolver)
            .with_context(|| format!("source {}", source.id))?;
        validate_url("source.address", &address)
            .with_context(|| format!("source {}", source.id))?;
        if sources.insert(source.id.clone(), address).is_some() {
            bail!("duplicate source {}", source.id);
        }
    }
    if sources.is_empty() {
        bail!("infrabot must declare at least one source");
    }

    let mut recipient_ids = HashSet::new();
    let mut recipients = Vec::new();
    for recipient in builder.recipients {
        if !recipient_ids.insert(recipient.id.clone()) {
            bail!("duplicate recipient {}", recipient.id);
        }
        let chat_id = resolve_required(recipient.chat_id, "recipient.chat_id", resolver)
            .with_context(|| format!("recipient {}", recipient.id))?
            .parse::<i64>()
            .with_context(|| format!("recipient {} chat_id must be an integer", recipient.id))?;
        if chat_id == 0 {
            bail!("recipient {} chat_id must not be zero", recipient.id);
        }
        let events = recipient
            .events
            .context("recipient.events is required")?
            .into_iter()
            .collect::<HashSet<_>>();
        if events.is_empty() {
            bail!(
                "recipient {} must subscribe to at least one event",
                recipient.id
            );
        }
        recipients.push(RecipientConfig {
            id: recipient.id,
            chat_id,
            events,
        });
    }
    Ok(Config {
        bind_addr,
        public_url,
        telegram_bot_token,
        telegram_bot_username,
        telegram_webhook_secret,
        signing_secret,
        allowed_user_ids,
        subscriber_user_ids,
        subscriber_store,
        subscriber_events,
        sources,
        recipients,
        pairing_ttl_seconds: bounded_u64(
            "pairing_ttl_seconds",
            builder.pairing_ttl_seconds.unwrap_or(300),
            60,
            900,
        )?,
        access_token_ttl_seconds: bounded_u64(
            "access_token_ttl_seconds",
            builder.access_token_ttl_seconds.unwrap_or(2_592_000),
            300,
            31_536_000,
        )?,
        poll_interval_seconds: bounded_u64(
            "poll_interval_seconds",
            builder.poll_interval_seconds.unwrap_or(2),
            1,
            10,
        )?,
        max_active_pairings: bounded_usize(
            "max_active_pairings",
            builder.max_active_pairings.unwrap_or(1_000),
            1,
            10_000,
        )?,
        max_exchange_attempts: bounded_u8(
            "max_exchange_attempts",
            builder.max_exchange_attempts.unwrap_or(20),
            3,
            100,
        )?,
    })
}

fn named_block(line: &str, kind: &str) -> Option<String> {
    let prefix = format!("{kind} \"");
    let suffix = "\" {";
    line.strip_prefix(&prefix)
        .and_then(|value| value.strip_suffix(suffix))
        .map(str::to_owned)
}

fn assignment(line_number: usize, line: &str) -> Result<(&str, &str)> {
    let (key, value) = line
        .split_once('=')
        .with_context(|| format!("line {line_number}: expected assignment or block"))?;
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() || value.is_empty() {
        bail!("line {line_number}: invalid assignment");
    }
    Ok((key, value))
}

fn value_ref(line_number: usize, raw: &str) -> Result<ValueRef> {
    if let Some(name) = raw
        .strip_prefix("env(\"")
        .and_then(|value| value.strip_suffix("\")"))
    {
        if name.is_empty()
            || !name.chars().all(|character| {
                character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
            })
        {
            bail!("line {line_number}: invalid environment variable reference");
        }
        return Ok(ValueRef::Environment(name.to_owned()));
    }
    if let Some(value) = raw
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        return Ok(ValueRef::Literal(value.to_owned()));
    }
    bail!("line {line_number}: value must be quoted or use env(\"NAME\")")
}

fn string_list(line_number: usize, raw: &str) -> Result<Vec<String>> {
    let body = raw
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .with_context(|| format!("line {line_number}: expected a string list"))?
        .trim();
    if body.is_empty() {
        return Ok(Vec::new());
    }
    body.split(',')
        .map(|entry| {
            let entry = entry.trim();
            entry
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .map(str::to_owned)
                .with_context(|| format!("line {line_number}: list values must be quoted"))
        })
        .collect()
}

fn integer<T>(line_number: usize, raw: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    raw.parse::<T>()
        .with_context(|| format!("line {line_number}: expected an integer"))
}

fn set_once<T>(slot: &mut Option<T>, value: T, line_number: usize, field: &str) -> Result<()> {
    if slot.replace(value).is_some() {
        bail!("line {line_number}: duplicate field {field}");
    }
    Ok(())
}

fn resolve_required<F>(value: Option<ValueRef>, field: &str, resolver: &F) -> Result<String>
where
    F: Fn(&str) -> Option<String>,
{
    value
        .with_context(|| format!("{field} is required"))?
        .resolve(field, resolver)
}

fn resolve_secret<F>(value: Option<ValueRef>, field: &str, resolver: &F) -> Result<String>
where
    F: Fn(&str) -> Option<String>,
{
    let value = value.with_context(|| format!("{field} is required"))?;
    if !matches!(&value, ValueRef::Environment(_)) {
        bail!("{field} must use env(\"NAME\") and must not be committed literally");
    }
    value.resolve(field, resolver)
}

fn resolve_or<F>(
    value: Option<ValueRef>,
    default: &str,
    field: &str,
    resolver: &F,
) -> Result<String>
where
    F: Fn(&str) -> Option<String>,
{
    match value {
        Some(value) => value.resolve(field, resolver),
        None => Ok(default.to_owned()),
    }
}

impl ValueRef {
    fn resolve<F>(self, field: &str, resolver: &F) -> Result<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let value = match self {
            Self::Literal(value) => value,
            Self::Environment(name) => resolver(&name).with_context(|| {
                format!("{field} references missing environment variable {name}")
            })?,
        };
        let value = value.trim().to_owned();
        if value.is_empty() {
            bail!("{field} must not be empty");
        }
        Ok(value)
    }
}

fn env_value(name: &str) -> Option<String> {
    env::var(name).ok()
}

fn parse_i64_set(value: &str) -> Result<HashSet<i64>> {
    let values = value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::parse::<i64>)
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    Ok(values)
}

fn validate_identifier(kind: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        bail!("{kind} id must contain 1-64 ASCII letters, digits, dots, dashes, or underscores");
    }
    Ok(())
}

fn validate_url(field: &str, value: &str) -> Result<()> {
    let parsed = Url::parse(value).with_context(|| format!("{field} must be a valid URL"))?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("{field} must not contain credentials, a query, or a fragment");
    }
    let host = parsed
        .host_str()
        .with_context(|| format!("{field} has no host"))?;
    let secure = parsed.scheme() == "https";
    let local = parsed.scheme() == "http" && matches!(host, "localhost" | "127.0.0.1" | "::1");
    if !secure && !local {
        bail!("{field} must use HTTPS; HTTP is allowed only for localhost");
    }
    Ok(())
}

fn bounded_u64(name: &str, value: u64, minimum: u64, maximum: u64) -> Result<u64> {
    if !(minimum..=maximum).contains(&value) {
        bail!("{name} must be between {minimum} and {maximum}");
    }
    Ok(value)
}

fn bounded_usize(name: &str, value: usize, minimum: usize, maximum: usize) -> Result<usize> {
    if !(minimum..=maximum).contains(&value) {
        bail!("{name} must be between {minimum} and {maximum}");
    }
    Ok(value)
}

fn bounded_u8(name: &str, value: u8, minimum: u8, maximum: u8) -> Result<u8> {
    if !(minimum..=maximum).contains(&value) {
        bail!("{name} must be between {minimum} and {maximum}");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver(name: &str) -> Option<String> {
        match name {
            "BOT_TOKEN" => Some("bot-token".into()),
            "WEBHOOK_SECRET" => Some("webhook-secret-1234".into()),
            "SIGNING_SECRET" => Some("x".repeat(32)),
            "ALLOWED_USERS" => Some("42".into()),
            "SUBSCRIBERS" => Some("42,77".into()),
            "OWNER_CHAT" => Some("42".into()),
            _ => None,
        }
    }

    fn document() -> &'static str {
        r#"
infra 1

project "infrabot" {
    runtime = "docker"
    service "api" {
        source = "."
        build = "Dockerfile"
        expose = 8787
        health = "/health"
    }
}

infrabot {
    public_url = "https://bot.example"

    telegram {
        bot_username = "infra_example_bot"
        bot_token = env("BOT_TOKEN")
        webhook_secret = env("WEBHOOK_SECRET")
        signing_secret = env("SIGNING_SECRET")
        allowed_user_ids = env("ALLOWED_USERS")
        subscriber_user_ids = env("SUBSCRIBERS")
        subscriber_store = "/var/lib/infrabot/subscribers.json"
        subscriber_events = ["service.failed", "service.recovered"]
    }

    source "primary" {
        address = "https://primary.example"
    }

    recipient "owner" {
        chat_id = env("OWNER_CHAT")
        events = ["service.failed", "service.recovered"]
    }
}
"#
    }

    #[test]
    fn parses_infrabot_extension() {
        let config = parse_document(document(), &resolver).unwrap();
        assert_eq!(config.public_url, "https://bot.example");
        assert_eq!(config.sources["primary"], "https://primary.example");
        assert_eq!(config.subscriber_user_ids.len(), 2);
        assert!(config.subscriber_events.contains("service.failed"));
        assert_eq!(config.recipients[0].chat_id, 42);
        assert!(config.recipients[0].events.contains("service.failed"));
    }

    #[test]
    fn rejects_literal_secrets() {
        let input = document().replace(
            "bot_token = env(\"BOT_TOKEN\")",
            "bot_token = \"committed-secret\"",
        );
        assert!(parse_document(&input, &resolver).is_err());
    }

    #[test]
    fn rejects_unknown_sources_during_pairing_lookup() {
        let config = parse_document(document(), &resolver).unwrap();
        assert!(config.sources.contains_key("primary"));
        assert!(!config.sources.contains_key("missing"));
    }

    #[test]
    fn rejects_insecure_remote_urls() {
        let input = document().replace("https://primary.example", "http://primary.example");
        assert!(parse_document(&input, &resolver).is_err());
    }
}
