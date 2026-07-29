# infraBot

Secure Telegram authorization and notification gateway for [`infraCLI`](https://github.com/0x0sky/infraCLI).

`infraBot` keeps its routing contract in its own `.infra` file. The standard `project` block describes how infraCLI runs the API service; the application-owned `infrabot` block declares trusted CLI sources, their addresses, Telegram recipients, event subscriptions, policy, and secret references.

## topology

The first deployment colocates infraBot with the existing market workloads on `vps-spaceship-01`:

```text
0xda-market       ─┐
                   ├──► infraBot API ───► one Telegram bot
0xda-market-bot   ─┘           │
                               └──► configured recipients

shared edge: https://0xda-market.nilx.one/infra/*
```

Every source has an independent pairing session, verifier, source-bound access token, and local credential file. Authorizing `0xda-market-bot` does not overwrite or revoke `0xda-market`.

## `.infra` registry

```text
infra 1

project "infrabot" {
    runtime = "docker"

    service "api" {
        source = "."
        build = "Dockerfile"
        expose = 8787
        health = "/health"
        environment = ".env"
    }
}

infrabot {
    bind = "0.0.0.0:8787"
    public_url = "https://0xda-market.nilx.one/infra"
    pairing_ttl_seconds = 300
    access_token_ttl_seconds = 2592000
    poll_interval_seconds = 2
    max_active_pairings = 1000
    max_exchange_attempts = 20

    telegram {
        bot_username = env("TELEGRAM_BOT_USERNAME")
        bot_token = env("TELEGRAM_BOT_TOKEN")
        webhook_secret = env("TELEGRAM_WEBHOOK_SECRET")
        signing_secret = env("INFRABOT_SIGNING_SECRET")
        allowed_user_ids = env("TELEGRAM_ALLOWED_USER_IDS")
    }

    source "0xda-market" {
        address = "https://0xda-market.nilx.one"
    }

    source "0xda-market-bot" {
        address = "https://0xda-market.nilx.one/bot"
    }

    recipient "owner" {
        chat_id = env("TELEGRAM_CHAT_ID")
        events = ["*"]
    }
}
```

Addresses, source IDs, recipients, event filters, and policy live directly in `.infra`; only secrets and Telegram identities are resolved from the environment.

The registry is loaded and validated when the process starts. Changing a source, recipient, event filter, or policy requires an infraBot restart; the first version does not hot-reload configuration. Unknown blocks or fields, duplicate identifiers, missing environment references, literal secret values, insecure remote URLs, empty source sets, and empty recipient sets fail startup.

`source.address` is routing metadata for future outbound operator commands. It does not authenticate inbound traffic. Inbound events are trusted only after verification of a signed token bound to the declared source ID.

Recipient event lists accept exact event kinds such as `service.failed` and `service.recovered`. `"*"` subscribes a recipient to every event. Multiple matching recipients with the same Telegram chat ID are deduplicated for each delivery.

## authorization flow

```text
infraCLI                         infraBot                         Telegram
   │                                │                                │
   ├─ source + verifier challenge ─►│                                │
   │◄─ session + one-time deep link ┤                                │
   ├─ open deep link ───────────────────────────────────────────────►│
   │                                │◄─ verified private /start ─────┤
   │                                ├─ allowlist + source approval   │
   ├─ exchange session + verifier ─►│                                │
   │◄─ source-bound access token ───┤                                │
```

The private verifier never appears in the Telegram deep link. Pairing state is short-lived and in memory; the issued token contains the approved Telegram user, source ID, `events:write` scope, unique token ID, and expiry.

Authorize each source separately:

```bash
infra auth telegram \
  --endpoint https://0xda-market.nilx.one/infra \
  --source 0xda-market

infra auth telegram \
  --endpoint https://0xda-market.nilx.one/infra \
  --source 0xda-market-bot \
  --no-open
```

A source must already exist in infraBot's `.infra` registry. `INFRABOT_URL` and `INFRA_SOURCE` may replace the corresponding flags.

## event delivery

Authenticated sources submit normalized events to:

```text
POST /v1/events
Authorization: Bearer <source-bound-token>
Content-Type: application/json
```

Example body:

```json
{
  "kind": "service.failed",
  "project": "market",
  "service": "api",
  "status": "unhealthy",
  "message": "health check failed"
}
```

infraBot verifies the signature, issuer, audience, expiry, `events:write` scope, Telegram allowlist membership, and source registry membership. It then applies recipient filters, deduplicates chat IDs, renders the event, and calls Telegram `sendMessage`.

The current delivery path is synchronous. It reports attempted and successful recipient counts but does not yet provide a durable queue, retry schedule, delivery history, or cross-restart deduplication. Those belong to the notification-state layer described in the infraCLI architecture proposal.

## security contract

- only Telegram users declared by `telegram.allowed_user_ids` may approve or keep using source tokens;
- approval is accepted only from a private Telegram chat;
- Telegram webhooks require `X-Telegram-Bot-Api-Secret-Token`;
- pairing links expire and are single-use;
- verifier and webhook-secret comparisons are constant-time;
- exchange attempts, polling rate, active sessions, and request bodies are bounded;
- tokens are signed with an independent secret and bound to one source;
- bot token, webhook secret, signing secret, verifier, and access token are never logged;
- secret fields must use `env("NAME")`; committed literal secrets are rejected;
- public and source URLs require HTTPS, except localhost development;
- source addresses are never treated as proof of identity.

## API

| Endpoint | Purpose |
| --- | --- |
| `GET /health` | process health |
| `POST /v1/pairings` | create a source-bound PKCE-style pairing session |
| `POST /v1/pairings/:id/exchange` | exchange an approved session for a token |
| `POST /v1/events` | deliver an authenticated source event to matching recipients |
| `POST /telegram/webhook` | receive verified Telegram updates |

Pending pairing state is intentionally in memory for the first single-replica deployment. Restarting infraBot invalidates pending sessions. Existing signed access tokens remain verifiable, but removing a source or Telegram user from `.infra` blocks that token at event submission time.

Before horizontal scaling, move pairing and future delivery state to a shared atomic store while preserving the API contract.

## environment

Start from `.env.example`. It supplies runtime values referenced by `.infra`:

```text
TELEGRAM_BOT_TOKEN
TELEGRAM_BOT_USERNAME
TELEGRAM_WEBHOOK_SECRET
TELEGRAM_ALLOWED_USER_IDS
TELEGRAM_CHAT_ID
INFRABOT_SIGNING_SECRET
MARKET_EDGE_NETWORK
```

`TELEGRAM_WEBHOOK_SECRET` must contain 16–256 URL-safe characters. `INFRABOT_SIGNING_SECRET` must contain at least 32 random bytes and must differ from the bot token and webhook secret. Rotating it invalidates issued tokens.

## run

```bash
cargo run
```

Local execution reads `./.infra` by default. Set `INFRABOT_CONFIG` to load another path.

Configure Telegram's webhook as:

```text
https://0xda-market.nilx.one/infra/telegram/webhook
```

Put the service behind HTTPS and rate-limit `POST /v1/pairings` and `POST /v1/events` at the edge.

## container

```bash
docker build -t infrabot:local .
docker run --rm --env-file .env -p 8787:8787 infrabot:local
```

The image contains the repository `.infra` at `/etc/infrabot/.infra`. Changing the baked registry requires a rebuild and restart. A mounted file may be used instead by overriding `INFRABOT_CONFIG`.

Run one replica until pairing state moves to a shared store.

## VPS deployment

The colocated VPS release contract, runtime layout, manual `validate/activate` workflow, rollback behavior, and activation order are documented in [`deploy/vps/README.md`](deploy/vps/README.md).

## development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
bash -n deploy/vps/deploy.sh deploy/vps/verify.sh
```
