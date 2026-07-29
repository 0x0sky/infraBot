mod config;
mod store;
mod telegram;
mod token;

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use config::{Config, RecipientConfig};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};
use store::{ApproveOutcome, CreateOutcome, ExchangeOutcome, PairingStore, decode_challenge};
use subtle::ConstantTimeEq;
use telegram::{TelegramUpdate, send_message, start_payload};
use token::{issue_access_token, verify_access_token};
use tokio::{net::TcpListener, sync::Mutex};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

const TELEGRAM_SECRET_HEADER: &str = "x-telegram-bot-api-secret-token";

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    store: Arc<Mutex<PairingStore>>,
    http: Client,
}

#[derive(Deserialize)]
struct CreatePairingRequest {
    code_challenge: String,
    client: String,
    client_version: String,
    source: String,
}

#[derive(Serialize)]
struct CreatePairingResponse {
    session_id: String,
    verification_uri_complete: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct ExchangeRequest {
    code_verifier: String,
}

#[derive(Serialize)]
struct PairingStatus {
    status: &'static str,
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_at: u64,
    telegram_user_id: i64,
    source: String,
}

#[derive(Deserialize)]
struct EventRequest {
    kind: String,
    project: String,
    service: Option<String>,
    status: Option<String>,
    message: String,
    #[serde(default)]
    fields: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct DeliveryResponse {
    source: String,
    attempted: usize,
    delivered: usize,
}

#[derive(Serialize)]
struct ApiError {
    error: &'static str,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("infrabot=info")),
        )
        .init();

    let config = Arc::new(Config::load()?);
    let route_count = config
        .recipients
        .iter()
        .map(|recipient| recipient.events.len())
        .sum::<usize>();
    let chat_count = config
        .recipients
        .iter()
        .map(|recipient| recipient.chat_id)
        .collect::<HashSet<_>>()
        .len();
    let recipient_names = config
        .recipients
        .iter()
        .map(|recipient| recipient.id.as_str())
        .collect::<Vec<_>>();
    info!(
        public_url = %config.public_url,
        sources = config.sources.len(),
        recipients = config.recipients.len(),
        chats = chat_count,
        routes = route_count,
        recipient_names = ?recipient_names,
        "infraBot configuration loaded"
    );

    let store = PairingStore::new(
        config.pairing_ttl_seconds,
        config.poll_interval_seconds,
        config.max_active_pairings,
        config.max_exchange_attempts,
    );
    let http = Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("infraBot/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build Telegram client")?;
    let state = AppState {
        config: Arc::clone(&config),
        store: Arc::new(Mutex::new(store)),
        http,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/pairings", post(create_pairing))
        .route("/v1/pairings/:session_id/exchange", post(exchange_pairing))
        .route("/v1/events", post(deliver_event))
        .route("/telegram/webhook", post(telegram_webhook))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(state);

    let listener = TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("bind {}", config.bind_addr))?;
    info!(address = %config.bind_addr, "infraBot listening");
    axum::serve(listener, app).await.context("serve infraBot")?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn create_pairing(
    State(state): State<AppState>,
    Json(request): Json<CreatePairingRequest>,
) -> Response {
    if request.client != "infraCLI"
        || request.client_version.is_empty()
        || request.client_version.len() > 64
    {
        return api_error(StatusCode::BAD_REQUEST, "invalid_client");
    }
    if !state.config.sources.contains_key(&request.source) {
        return api_error(StatusCode::FORBIDDEN, "unknown_source");
    }
    let Some(code_challenge) = decode_challenge(&request.code_challenge) else {
        return api_error(StatusCode::BAD_REQUEST, "invalid_code_challenge");
    };

    let created = {
        let mut store = state.store.lock().await;
        store.create(request.source, code_challenge)
    };
    let CreateOutcome::Created(created) = created else {
        return api_error(StatusCode::SERVICE_UNAVAILABLE, "pairing_capacity_exceeded");
    };

    let verification_uri_complete = format!(
        "https://t.me/{}?start={}",
        state.config.telegram_bot_username, created.start_token
    );
    (
        StatusCode::CREATED,
        Json(CreatePairingResponse {
            session_id: created.session_id.to_string(),
            verification_uri_complete,
            expires_in: created.expires_in,
            interval: created.interval,
        }),
    )
        .into_response()
}

async fn exchange_pairing(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<ExchangeRequest>,
) -> Response {
    if request.code_verifier.len() < 43 || request.code_verifier.len() > 128 {
        return api_error(StatusCode::UNAUTHORIZED, "invalid_verifier");
    }
    let Ok(session_id) = Uuid::parse_str(&session_id) else {
        return api_error(StatusCode::NOT_FOUND, "pairing_not_found");
    };

    let outcome = {
        let mut store = state.store.lock().await;
        store.exchange(session_id, &request.code_verifier)
    };

    match outcome {
        ExchangeOutcome::Pending => (
            StatusCode::ACCEPTED,
            Json(PairingStatus { status: "pending" }),
        )
            .into_response(),
        ExchangeOutcome::SlowDown => api_error(StatusCode::TOO_MANY_REQUESTS, "slow_down"),
        ExchangeOutcome::Approved {
            telegram_user_id,
            source,
        } => {
            match issue_access_token(
                &state.config.signing_secret,
                telegram_user_id,
                &source,
                state.config.access_token_ttl_seconds,
            ) {
                Ok(token) => Json(TokenResponse {
                    access_token: token.access_token,
                    token_type: "Bearer",
                    expires_at: token.expires_at,
                    telegram_user_id,
                    source,
                })
                .into_response(),
                Err(_) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "token_issue_failed"),
            }
        }
        ExchangeOutcome::InvalidVerifier => api_error(StatusCode::UNAUTHORIZED, "invalid_verifier"),
        ExchangeOutcome::TooManyAttempts => api_error(StatusCode::FORBIDDEN, "pairing_locked"),
        ExchangeOutcome::Expired => api_error(StatusCode::GONE, "pairing_expired"),
        ExchangeOutcome::Consumed => api_error(StatusCode::CONFLICT, "pairing_consumed"),
        ExchangeOutcome::NotFound => api_error(StatusCode::NOT_FOUND, "pairing_not_found"),
    }
}

async fn deliver_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<EventRequest>,
) -> Response {
    let Some(access_token) = bearer_token(&headers) else {
        return api_error(StatusCode::UNAUTHORIZED, "missing_access_token");
    };
    let Ok(authorized) = verify_access_token(&state.config.signing_secret, access_token) else {
        return api_error(StatusCode::UNAUTHORIZED, "invalid_access_token");
    };
    if !state
        .config
        .allowed_user_ids
        .contains(&authorized.telegram_user_id)
        || !state.config.sources.contains_key(&authorized.source)
    {
        return api_error(StatusCode::FORBIDDEN, "source_not_authorized");
    }
    if !valid_event(&request) {
        return api_error(StatusCode::BAD_REQUEST, "invalid_event");
    }

    let chat_ids = state
        .config
        .recipients
        .iter()
        .filter(|recipient| recipient_matches(recipient, &request.kind))
        .map(|recipient| recipient.chat_id)
        .collect::<HashSet<_>>();
    let attempted = chat_ids.len();
    let message = render_event(&authorized.source, &request);
    let mut delivered = 0;
    for chat_id in chat_ids {
        if send_message(
            &state.http,
            &state.config.telegram_bot_token,
            chat_id,
            &message,
        )
        .await
        {
            delivered += 1;
        } else {
            warn!(source = %authorized.source, chat_id, "Telegram event delivery failed");
        }
    }

    if attempted > 0 && delivered == 0 {
        return api_error(StatusCode::BAD_GATEWAY, "telegram_delivery_failed");
    }
    (
        StatusCode::OK,
        Json(DeliveryResponse {
            source: authorized.source,
            attempted,
            delivered,
        }),
    )
        .into_response()
}

async fn telegram_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(update): Json<TelegramUpdate>,
) -> Response {
    let supplied_secret = headers
        .get(TELEGRAM_SECRET_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !secure_eq(supplied_secret, &state.config.telegram_webhook_secret) {
        return api_error(StatusCode::UNAUTHORIZED, "invalid_webhook_secret");
    }

    let Some(message) = update.message else {
        return StatusCode::OK.into_response();
    };
    if message.chat.kind != "private" {
        return StatusCode::OK.into_response();
    }
    let Some(user) = message.from else {
        return StatusCode::OK.into_response();
    };
    let Some(text) = message.text.as_deref() else {
        return StatusCode::OK.into_response();
    };
    let Some(payload) = start_payload(text) else {
        return StatusCode::OK.into_response();
    };

    let reply = if !state.config.allowed_user_ids.contains(&user.id) {
        "this Telegram account is not authorized for infraCLI".to_owned()
    } else {
        let outcome = {
            let mut store = state.store.lock().await;
            store.approve(payload, user.id)
        };
        match outcome {
            ApproveOutcome::Approved(source) | ApproveOutcome::AlreadyApproved(source) => {
                format!("infraCLI source {source} authorized. return to the terminal")
            }
            ApproveOutcome::DifferentUser => {
                "this pairing belongs to another Telegram account".to_owned()
            }
            ApproveOutcome::Expired | ApproveOutcome::Consumed | ApproveOutcome::NotFound => {
                "pairing expired or already used. run infra auth again".to_owned()
            }
        }
    };

    if !send_message(
        &state.http,
        &state.config.telegram_bot_token,
        message.chat.id,
        &reply,
    )
    .await
    {
        warn!("Telegram sendMessage failed");
    }
    StatusCode::OK.into_response()
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|value| !value.is_empty())
}

fn valid_event(event: &EventRequest) -> bool {
    valid_token(&event.kind, 128)
        && valid_text(&event.project, 128)
        && event
            .service
            .as_deref()
            .is_none_or(|value| valid_text(value, 128))
        && event
            .status
            .as_deref()
            .is_none_or(|value| valid_text(value, 64))
        && valid_text(&event.message, 4_000)
        && event.fields.len() <= 24
        && event
            .fields
            .iter()
            .all(|(name, value)| valid_token(name, 64) && valid_text(value, 512))
        && event.fields.values().map(String::len).sum::<usize>() <= 4_096
}

fn valid_token(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

fn recipient_matches(recipient: &RecipientConfig, kind: &str) -> bool {
    recipient.events.contains("*") || recipient.events.contains(kind)
}

fn render_event(source: &str, event: &EventRequest) -> String {
    let mut lines = vec![
        format!("{} · {}", event.kind, event.project),
        format!("source: {source}"),
    ];
    if event.fields.is_empty() {
        if let Some(service) = event.service.as_deref() {
            lines.push(format!("service: {service}"));
        }
        if let Some(status) = event.status.as_deref() {
            lines.push(format!("status: {status}"));
        }
    } else {
        for (name, value) in &event.fields {
            lines.push(format!("{name}: {value}"));
        }
    }
    lines.push(String::new());
    lines.push(event.message.trim().to_owned());
    lines.join(
        "
",
    )
}

fn secure_eq(left: &str, right: &str) -> bool {
    left.len() == right.len() && bool::from(left.as_bytes().ct_eq(right.as_bytes()))
}

fn api_error(status: StatusCode, error: &'static str) -> Response {
    (status, Json(ApiError { error })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_webhook_secrets() {
        assert!(secure_eq("same-secret", "same-secret"));
        assert!(!secure_eq("same-secret", "other-secret"));
        assert!(!secure_eq("short", "longer"));
    }

    #[test]
    fn validates_and_renders_events() {
        let event = EventRequest {
            kind: "service.failed".into(),
            project: "market".into(),
            service: Some("api".into()),
            status: Some("unhealthy".into()),
            message: "health check failed".into(),
            fields: BTreeMap::from([
                ("image".into(), "market:latest".into()),
                ("previous_status".into(), "healthy".into()),
            ]),
        };
        assert!(valid_event(&event));
        let rendered = render_event("primary", &event);
        assert!(rendered.contains("service.failed · market"));
        assert!(rendered.contains("source: primary"));
        assert!(rendered.contains("image: market:latest"));
    }

    #[test]
    fn wildcard_recipient_matches_every_event() {
        let recipient = RecipientConfig {
            id: "owner".into(),
            chat_id: 42,
            events: HashSet::from(["*".into()]),
        };
        assert!(recipient_matches(&recipient, "service.failed"));
    }
}
