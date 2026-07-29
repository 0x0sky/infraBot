# infraBot

Secure Telegram authorization and notification gateway for [`infraCLI`](https://github.com/0x0sky/infraCLI).

The first deployment uses Telegram bot username `@infra_services_bot` and colocates an infra-agent container with the existing market workloads on `vps-spaceship-01`.

## topology

```text
Docker daemon
   │
   ▼
restricted docker-api proxy
   │ input
   ▼
infra-agent · source vps-spaceship-01
   │ output
   ▼
infraBot API
   │
   ▼
@infra_services_bot
```

The shared edge exposes infraBot at:

```text
https://0xda-market.nilx.one/infra/*
```

The agent is one authorized source. `0xda-market`, `0xda-market-bot`, and infraBot containers are observations inside its Docker input, not separate Telegram credentials.

## infraBot `.infra`

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

    telegram {
        bot_username = "infra_services_bot"
        bot_token = env("TELEGRAM_BOT_TOKEN")
        webhook_secret = env("TELEGRAM_WEBHOOK_SECRET")
        signing_secret = env("INFRABOT_SIGNING_SECRET")
        allowed_user_ids = env("TELEGRAM_ALLOWED_USER_IDS")
    }

    source "vps-spaceship-01" {
        address = "https://0xda-market.nilx.one"
    }

    recipient "owner" {
        chat_id = env("TELEGRAM_CHAT_ID")
        events = ["service.failed", "service.recovered", "service.started", "service.removed"]
    }
}
```

Source IDs, addresses, recipients, event filters, and non-secret policy live in `.infra`. Tokens, signing material, webhook secrets, allowlisted Telegram IDs, and chat IDs remain in the protected environment.

`source.address` is routing metadata for future commands. It is never accepted as authentication proof. Inbound events require a signed source-bound token.

## infra-agent adapters

The companion agent configuration is `deploy/vps/infra-agent.infra`:

```text
agent {
    input "market-docker" {
        driver = "docker"
        addresses = ["http://docker-api:2375"]
        projects = ["zero-x-da-market-development", "zero-x-da-market-bot-development", "zero-x-infrabot-vps-spaceship-01"]
        services = []
        fields = ["input", "docker_address", "container", "container_id", "project", "service", "image", "state", "health"]
    }

    output "infra_services_bot" {
        driver = "infrabot"
        endpoint = "http://infra-bot:8787"
        source = "vps-spaceship-01"
        credential = env("INFRA_CREDENTIALS_FILE")
        events = ["service.failed", "service.recovered", "service.started", "service.removed"]
        fields = ["kind", "project", "service", "container", "image", "state", "health", "previous_status", "status"]
    }
}
```

The agent receives no raw Docker socket. A private socket-proxy container exposes only the container-reading API on a Docker-internal network. The agent establishes a silent baseline, persists state, and emits only later transitions.

## authorization flow

```text
infra-auth                       infraBot                         Telegram
    │                                │                                │
    ├─ source + verifier challenge ─►│                                │
    │◄─ session + one-time deep link ┤                                │
    ├─ print deep link ──────────────────────────────────────────────►│
    │                                │◄─ verified private /start ─────┤
    │                                ├─ allowlist + source approval   │
    ├─ exchange session + verifier ─►│                                │
    │◄─ source-bound access token ───┤                                │
```

Pair the single agent source after the public route and Telegram webhook exist:

```bash
docker compose --profile tools run --rm infra-auth \
  auth telegram \
  --endpoint https://0xda-market.nilx.one/infra \
  --source vps-spaceship-01 \
  --no-open
```

The credential is written to `/opt/infrabot/credentials/vps-spaceship-01.json` and mounted read-only into infra-agent.

## event API

```text
POST /v1/events
Authorization: Bearer <source-bound-token>
Content-Type: application/json
```

```json
{
  "kind": "service.failed",
  "project": "zero-x-da-market-development",
  "service": "api",
  "status": "failed",
  "message": "api: healthy -> failed"
}
```

infraBot verifies token signature, issuer, audience, expiry, scope, Telegram allowlist membership, and current source registry membership. It then applies recipient filters, deduplicates chat IDs, and delivers through Telegram `sendMessage`.

Delivery is synchronous in version 1. Durable queues, retry schedules, history, and horizontal shared state remain later layers.

## security contract

- only Telegram IDs declared by `telegram.allowed_user_ids` may approve or continue using source tokens;
- approval is accepted only from a private Telegram chat;
- webhooks require `X-Telegram-Bot-Api-Secret-Token`;
- pairing links expire and are single-use;
- verifier and webhook-secret comparisons are constant-time;
- tokens are signed with an independent secret and bound to one source;
- secret values are never logged and must not be committed literally;
- the raw Docker socket is isolated inside the restricted proxy;
- Docker inputs and infraBot output communicate only over private container networks;
- public authorization and webhook traffic terminate HTTPS at the shared edge.

## API

| Endpoint | Purpose |
| --- | --- |
| `GET /health` | process health |
| `POST /v1/pairings` | create a source-bound pairing session |
| `POST /v1/pairings/:id/exchange` | exchange approval for a token |
| `POST /v1/events` | deliver an authenticated source transition |
| `POST /telegram/webhook` | receive verified Telegram updates |

Pending pairing state is in memory for the first single-replica deployment. Restarting infraBot invalidates only pending sessions. Existing signed tokens remain verifiable, while removing the source or Telegram user from `.infra` blocks future submissions.

## environment

```text
TELEGRAM_BOT_TOKEN
TELEGRAM_WEBHOOK_SECRET
TELEGRAM_ALLOWED_USER_IDS
TELEGRAM_CHAT_ID
INFRABOT_SIGNING_SECRET
MARKET_EDGE_NETWORK
```

`TELEGRAM_WEBHOOK_SECRET` must contain 16–256 URL-safe characters. `INFRABOT_SIGNING_SECRET` must contain at least 32 random bytes and differ from the bot token and webhook secret.

## webhook

```text
https://0xda-market.nilx.one/infra/telegram/webhook
```

## VPS deployment

The container topology, private Docker API, two-phase rollout, credentials, verification, and rollback model are documented in [`deploy/vps/README.md`](deploy/vps/README.md).

## development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
bash -n deploy/vps/deploy.sh deploy/vps/verify.sh
```
