#!/usr/bin/env bash
# Disable env-only proxy: remove the sourceable env file.

set -euo pipefail

ENV_DIR="${HOME}/.config/offline-gateway"
ENV_FILE="${ENV_DIR}/env.sh"

rm -f "$ENV_FILE" 2>/dev/null || true
rmdir "$ENV_DIR" 2>/dev/null || true

echo "offline-gateway-proxy disabled (env only)"
echo "Unset in current shells if needed:"
echo "  unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY ALL_PROXY all_proxy NO_PROXY no_proxy"
