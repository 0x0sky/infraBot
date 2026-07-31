# VPS deployment

`infraBot` and the `infra` service monitor run as independent containers on `vps-spaceship-01`, beside the existing `0xda-market` and `0xda-market-bot` workloads.

```text
Docker daemon
   │
   ▼
docker-api · restricted read proxy
   │ input
   ▼
infra-agent · .infra adapters
   │ output
   ▼
infra-bot:8787
   │
   ▼
@infra_services_bot
```

The public edge remains owned by `0x0sky/infra` and exposes:

```text
https://0xda-market.nilx.one/infra/*
```

Caddy strips `/infra` before forwarding to `infra-bot:8787`.

## Ownership

`0x0sky/infraBot` owns:

- the infraBot API image and release lifecycle;
- the `infra-bot:8787` workload contract;
- the containerized infra-agent image built from a selected `0x0sky/infraCLI` ref;
- the private Docker API input proxy;
- the `input/output` adapter configuration in `infra-agent.infra`;
- Telegram pairing, source authorization, routing, and event delivery.

`0x0sky/infra` owns Caddy, public ports `80/443`, `nilx-edge`, `/infra/*` routing, and public HTTPS verification.

The market repositories remain the sole owners of their application containers and environments. This deployment reads their state but never rebuilds, restarts, stops, or switches them.

## Compose topology

```text
zero-x-infrabot-vps-spaceship-01
├── infrabot
│   ├── edge alias: infra-bot
│   └── loopback: 127.0.0.1:8787
├── docker-api
│   ├── raw Docker socket is mounted only here
│   ├── GET container API only
│   └── private monitor network only
├── infra-agent
│   ├── no Docker socket
│   ├── input: http://docker-api:2375
│   ├── output: http://infra-bot:8787
│   ├── state volume: /var/lib/infra
│   └── loopback health: 127.0.0.1:9090
└── infra-auth · one-off tools profile
```

The socket proxy is pinned to `ghcr.io/tecnativa/docker-socket-proxy:0.4.2`. Only `CONTAINERS=1` is enabled and mutating `POST` requests are disabled. The proxy is never attached to `nilx-edge` or a host port.

## `.infra` agent contract

`deploy/vps/infra-agent.infra` declares one Docker input and one infraBot output:

```text
input "market-docker" {
    driver = "docker"
    addresses = ["http://docker-api:2375"]
    projects = [
        "zero-x-da-market-development",
        "zero-x-da-market-bot-development",
        "zero-x-infrabot-vps-spaceship-01"
    ]
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
```

The first poll establishes a silent baseline. Later polls emit only transitions and persist state in a named volume.

## VPS layout

```text
/opt/infrabot/
├── credentials/
│   └── vps-spaceship-01.json
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

No host `infra` binary is installed. The same infraCLI executable runs inside `infra-agent` and the one-off `infra-auth` container.

## GitHub Environment

Create environment `vps-spaceship-01` in `0x0sky/infraBot`.

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
DOCKER_SOCKET_PROXY_IMAGE=ghcr.io/tecnativa/docker-socket-proxy:0.4.2
INFRABOT_CREDENTIALS_DIR=/opt/infrabot/credentials

TELEGRAM_BOT_TOKEN=<token for @infra_services_bot>
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

## Rollout sequence

1. Merge compatible infraCLI and infraBot changes.
2. Run `Deploy infraBot to VPS` with `mode=validate`.
3. Run it with `mode=activate` and confirmation `activate-infrabot`.
   - infraBot and docker-api start;
   - infra-agent remains stopped until its credential exists.
4. Merge and activate companion `0x0sky/infra#8`.
5. Register Telegram webhook:

   ```text
   https://0xda-market.nilx.one/infra/telegram/webhook
   ```

6. Pair the single agent source:

   ```bash
   cd /opt/infrabot/targets/vps-spaceship-01/current/deploy/vps

   docker compose --profile tools run --rm infra-auth \
     auth telegram \
     --endpoint https://0xda-market.nilx.one/infra \
     --source vps-spaceship-01 \
     --no-open
   ```

   The command writes `/opt/infrabot/credentials/vps-spaceship-01.json`.

7. Run `Deploy infraBot to VPS` with `mode=activate` again. The deployment now starts and health-checks `infra-agent`.
8. Set `VERIFY_PUBLIC_HTTPS=1` and run `verify.sh`.

## Verification

```bash
curl -fsS http://127.0.0.1:8787/health
curl -fsS http://127.0.0.1:9090/health

docker run --rm --network nilx-edge alpine:3.22 \
  wget -q -O- http://infra-bot:8787/health

docker run --rm --network zero-x-infrabot-vps-spaceship-01-monitor alpine:3.22 \
  wget -q -O- 'http://docker-api:2375/containers/json?all=1'
```

## Safety

- `validate` builds both application images without changing live containers;
- activation requires the exact token `activate-infrabot`;
- the agent never receives the raw Docker socket;
- the Docker proxy has no public or shared-edge network;
- the source credential is stored outside releases with directory mode `0700` and file mode `0600`;
- failed activation attempts to restore the previous release;
- DNS, Caddy, market services, databases, and Telegram webhook state remain separate operations.


## Telegram subscribers

`TELEGRAM_ALLOWED_USER_IDS` contains operators allowed to approve infra pairing. `TELEGRAM_SUBSCRIBER_USER_IDS` contains every Telegram user allowed to join the broadcast with `/start`; use comma-separated numeric IDs. Subscription state is persisted in the `infrabot-state` volume and survives container replacement and release rollback.
