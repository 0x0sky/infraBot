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

edge_network="${MARKET_EDGE_NETWORK:-nilx-edge}"

curl --fail --silent --show-error \
  --retry 12 --retry-delay 3 --retry-connrefused \
  http://127.0.0.1:8787/health | grep -qx 'ok'

docker run --rm --network "$edge_network" alpine:3.22 \
  wget -q -T 10 -O- http://infra-bot:8787/health | grep -qx 'ok'

if [[ "${VERIFY_PUBLIC_HTTPS:-0}" == "1" ]]; then
  curl --fail --silent --show-error --location \
    --retry 12 --retry-all-errors --retry-delay 5 \
    --connect-timeout 10 --max-time 30 \
    https://0xda-market.nilx.one/infra/health | grep -qx 'ok'
fi

echo "infraBot verification passed"
