# VPS deployment

`infraBot` runs on `vps-spaceship-01` beside the existing `0xda-market` and `0xda-market-bot` workloads. It is an independent Compose project and joins the shared external `nilx-edge` network through the stable alias `infra-bot`.

The public edge remains owned by `0x0sky/infra`. It exposes infraBot at:

```text
https://0xda-market.nilx.one/infra/*
```

Caddy strips `/infra` before forwarding requests to `infra-bot:8787`.

## Ownership

`0x0sky/infraBot` owns:

- the infraBot image and release lifecycle;
- the `infra-bot:8787` workload contract;
- Telegram pairing, source authorization, routing policy, and event delivery;
- the host build of `infraCLI` installed with the release;
- local and internal health verification.

`0x0sky/infra` owns:

- Caddy and public ports `80/443`;
- the `nilx-edge` network contract;
- `/infra/*` routing and public HTTPS verification.

The market repositories continue to own their existing application containers and deployment environments. infraBot deployment never rebuilds, restarts, or switches them.

## VPS layout

```text
/opt/infrabot/
├── bin/
│   └── infra
└── targets/
    └── vps-spaceship-01/
        ├── current -> releases/<sha>
        ├── releases/
        │   └── <sha>/
        └── shared/
            └── .env

/opt/0xda-market/
/opt/0xda-market-bot/
/opt/infra/
```

The workflow retains the three newest activated infraBot releases. `infraCLI` is built in GitHub Actions from the selected repository ref and installed atomically at `/opt/infrabot/bin/infra` only after infraBot activation succeeds.

## GitHub Environment

Create the `vps-spaceship-01` environment in `0x0sky/infraBot`.

Secrets:

- `SSH_HOST`
- `SSH_USER`
- `SSH_PRIVATE_KEY`

Variables:

- `SSH_PORT=22022`
- `DEPLOY_PATH=/opt/infrabot`

## Runtime file

Create:

```text
/opt/infrabot/targets/vps-spaceship-01/shared/.env
```

Start from `env.example`:

```env
EDGE_TARGET=vps-spaceship-01
MARKET_EDGE_NETWORK=nilx-edge
TELEGRAM_BOT_TOKEN=<infra bot token>
TELEGRAM_BOT_USERNAME=<infra bot username>
TELEGRAM_WEBHOOK_SECRET=<random webhook secret>
TELEGRAM_ALLOWED_USER_IDS=<comma-separated Telegram user IDs>
TELEGRAM_CHAT_ID=<notification chat ID>
INFRABOT_SIGNING_SECRET=<at least 32 random bytes>
VERIFY_PUBLIC_HTTPS=0
```

Protect it:

```bash
chown deploy:deploy /opt/infrabot/targets/vps-spaceship-01/shared/.env
chmod 0600 /opt/infrabot/targets/vps-spaceship-01/shared/.env
```

## Deployment sequence

1. Merge compatible infraCLI and infraBot changes.
2. Run `Deploy infraBot to VPS` with `mode=validate`.
3. Run it with `mode=activate` and confirmation `activate-infrabot`.
4. Verify local and internal health:

   ```bash
   curl -fsS http://127.0.0.1:8787/health
   docker run --rm --network nilx-edge alpine:3.22 \
     wget -q -O- http://infra-bot:8787/health
   ```

5. Merge and activate the companion `0x0sky/infra` edge change.
6. Set `VERIFY_PUBLIC_HTTPS=1` and run `verify.sh`.
7. Register Telegram's webhook separately at:

   ```text
   https://0xda-market.nilx.one/infra/telegram/webhook
   ```

8. Authorize both declared sources:

   ```bash
   /opt/infrabot/bin/infra auth telegram \
     --endpoint https://0xda-market.nilx.one/infra \
     --source 0xda-market

   /opt/infrabot/bin/infra auth telegram \
     --endpoint https://0xda-market.nilx.one/infra \
     --source 0xda-market-bot \
     --no-open
   ```

## Safety

- `validate` builds and validates a release without changing the active container or host binary.
- `activate` requires the exact confirmation token `activate-infrabot`.
- failed activation attempts to restore the previous infraBot release;
- the workflow never changes DNS, Caddy, market services, databases, or Telegram webhook state;
- public activation of `/infra/*` must happen only after `infra-bot:8787` is healthy on `nilx-edge`.
