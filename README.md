# infraBot

Secure Telegram authorization and notification gateway for [`infraCLI`](https://github.com/0x0sky/infraCLI).

The first implementation provides a fast one-tap pairing flow between the CLI and an allowlisted Telegram account. Notification delivery and operator commands remain separate follow-up capabilities.

## authorization topology

One infraBot deployment and one Telegram bot may authorize multiple independent infraCLI installations:

```text
infraCLI · host-a ─┐
                   ├──► infraBot ───► one Telegram bot/account
infraCLI · host-b ─┘
```

Every CLI creates its own pairing session, verifier, and token exchange. Sessions are keyed independently, issued tokens have unique IDs, and credentials remain local to each CLI host. Authorizing a second CLI does not overwrite or revoke the first CLI.

The current contract identifies authorization by Telegram account and pairing session. A persistent installation identity is deliberately deferred until infraBot adds per-device listing and revocation; it is not required for safely connecting multiple CLI instances.

## authorization flow

```text
infraCLI                         infraBot                         Telegram
   │                                │                                │
   ├─ create verifier + challenge ─►│                                │
   │◄─ session + one-time deep link ┤                                │
   ├─ open deep link ───────────────────────────────────────────────►│
   │                                │◄─ signed webhook /start token ─┤
   │                                ├─ allowlist + session approval   │
   ├─ exchange session + verifier ─►│                                │
   │◄─ scoped access token ─────────┤                                │
```

The CLI private verifier never leaves the device until token exchange. infraBot stores only its SHA-256 challenge. The Telegram deep link contains a separate random, five-minute, one-time secret and never contains the access token or verifier.

## security contract

- only Telegram user IDs listed in `TELEGRAM_ALLOWED_USER_IDS` may approve pairing;
- approval is accepted only from a private Telegram chat;
- Telegram webhooks require `X-Telegram-Bot-Api-Secret-Token`;
- pairing links expire and are single-use;
- verifier comparison is constant-time and exchange attempts are bounded;
- repeated polling is throttled per pairing session;
- the access token is scoped and signed with an independent secret;
- bot token, webhook secret, signing secret, verifier, and access token are never logged;
- production traffic must terminate TLS before reaching infraBot;
- active pairing capacity is bounded to limit unauthenticated resource consumption.

`TELEGRAM_ALLOWED_USER_IDS` is mandatory. This is not an open Telegram login service.

## API

| Endpoint | Purpose |
| --- | --- |
| `GET /health` | process health |
| `POST /v1/pairings` | create a PKCE-style pairing session |
| `POST /v1/pairings/:id/exchange` | poll and exchange an approved session |
| `POST /telegram/webhook` | receive verified Telegram updates |

Pairing state is intentionally in-memory for the first single-replica deployment. Restarting infraBot invalidates only pending five-minute sessions. Issued access tokens remain independently verifiable through `INFRABOT_SIGNING_SECRET`.

Before horizontal scaling, replace the in-memory store with a shared atomic store and preserve the same pairing contract.

## configuration

Start from `.env.example`.

Required variables:

```text
TELEGRAM_BOT_TOKEN
TELEGRAM_BOT_USERNAME
TELEGRAM_WEBHOOK_SECRET
TELEGRAM_ALLOWED_USER_IDS
INFRABOT_SIGNING_SECRET
```

`TELEGRAM_ALLOWED_USER_IDS` is a comma-separated list of numeric Telegram user IDs.

`TELEGRAM_WEBHOOK_SECRET` must contain 16–256 URL-safe characters. Configure Telegram to send it as the webhook secret token for:

```text
https://<infrabot-host>/telegram/webhook
```

`INFRABOT_SIGNING_SECRET` must contain at least 32 random bytes and must be different from the Telegram bot token and webhook secret. Rotating it invalidates all issued access tokens.

Optional policy variables:

```text
PAIRING_TTL_SECONDS=300
PAIRING_POLL_INTERVAL_SECONDS=2
ACCESS_TOKEN_TTL_SECONDS=2592000
MAX_ACTIVE_PAIRINGS=1000
MAX_EXCHANGE_ATTEMPTS=20
```

## run

```bash
cargo run
```

The service listens on `0.0.0.0:8787` by default. Put it behind the shared HTTPS edge and apply request-rate limiting to `POST /v1/pairings` at that edge.

Then authorize from every infraCLI host independently:

```bash
# host-a
infra auth telegram --endpoint https://<infrabot-host>

# host-b
infra auth telegram --endpoint https://<infrabot-host> --no-open
```

Both commands target the same infraBot URL and the same Telegram bot. Each Telegram approval completes only the matching one-time pairing session.

## container

```bash
docker build -t infrabot:local .
docker run --rm --env-file .env -p 8787:8787 infrabot:local
```

Run the container as a single replica until pairing state moves to a shared store.

## development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```
