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

edge_target="${EDGE_TARGET:-vps-spaceship-01}"
edge_network="${MARKET_EDGE_NETWORK:-nilx-edge}"
monitor_network="zero-x-infrabot-${edge_target}-monitor"
credentials_dir="${INFRABOT_CREDENTIALS_DIR:-/opt/infrabot/credentials}"
credential_file="$credentials_dir/vps-spaceship-01.json"

curl --fail --silent --show-error \
  --retry 12 --retry-delay 3 --retry-connrefused \
  http://127.0.0.1:8787/health | grep -qx 'ok'

docker run --rm --network "$edge_network" alpine:3.22 \
  wget -q -T 10 -O- http://infra-bot:8787/health | grep -qx 'ok'

docker run --rm --network "$monitor_network" alpine:3.22 \
  wget -q -T 10 -O- 'http://docker-api:2375/containers/json?all=1' | grep -q '^\['

if [[ -f "$credential_file" ]]; then
  curl --fail --silent --show-error \
    --retry 12 --retry-delay 3 --retry-connrefused \
    http://127.0.0.1:9090/health | grep -qx 'ok'
  docker compose ps --status running infra-agent | grep -q infra-agent
else
  echo "infra agent credential is not present; agent verification skipped"
fi

if [[ "${VERIFY_PUBLIC_HTTPS:-0}" == "1" ]]; then
  curl --fail --silent --show-error --location \
    --retry 12 --retry-all-errors --retry-delay 5 \
    --connect-timeout 10 --max-time 30 \
    https://0xda-market.nilx.one/infra/health | grep -qx 'ok'
fi

echo "infraBot input/output runtime verification passed"
