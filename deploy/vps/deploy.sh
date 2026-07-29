#!/usr/bin/env bash
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

if [[ ! -f .env ]]; then
  echo "deploy/vps/.env is missing" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1091
source .env
set +a

deploy_mode="${DEPLOY_MODE:-activate}"
edge_target="${EDGE_TARGET:-vps-spaceship-01}"
edge_network="${MARKET_EDGE_NETWORK:-nilx-edge}"

case "$deploy_mode" in
  validate|activate) ;;
  *)
    echo "unsupported DEPLOY_MODE: $deploy_mode" >&2
    exit 1
    ;;
esac

if [[ "$edge_target" != "vps-spaceship-01" ]]; then
  echo "EDGE_TARGET must be vps-spaceship-01" >&2
  exit 1
fi

if [[ "$edge_network" != "nilx-edge" ]]; then
  echo "MARKET_EDGE_NETWORK must be nilx-edge" >&2
  exit 1
fi

required_variables=(
  TELEGRAM_BOT_TOKEN
  TELEGRAM_BOT_USERNAME
  TELEGRAM_WEBHOOK_SECRET
  TELEGRAM_ALLOWED_USER_IDS
  TELEGRAM_CHAT_ID
  INFRABOT_SIGNING_SECRET
)

for name in "${required_variables[@]}"; do
  if [[ -z "${!name:-}" ]]; then
    echo "$name is required" >&2
    exit 1
  fi
done

if ! docker network inspect "$edge_network" >/dev/null 2>&1; then
  echo "external Docker network $edge_network is missing" >&2
  exit 1
fi

docker compose config --quiet
docker compose build --pull infrabot

if [[ "$deploy_mode" == "validate" ]]; then
  echo "infraBot release validated for $edge_target"
  exit 0
fi

docker compose up -d --wait --remove-orphans infrabot

curl --fail --silent --show-error \
  --retry 12 --retry-delay 3 --retry-connrefused \
  http://127.0.0.1:8787/health | grep -qx 'ok'

docker run --rm --network "$edge_network" alpine:3.22 \
  wget -q -T 10 -O- http://infra-bot:8787/health | grep -qx 'ok'

echo "infraBot is healthy on 127.0.0.1:8787 and nilx-edge"
