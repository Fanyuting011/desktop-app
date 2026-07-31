#!/usr/bin/env bash
# Enable proxy via environment variables only.
# Does not modify apt/npm/git/docker/.bashrc — only writes a sourceable env file.
#
# Injected by Windows gateway:
#   GATEWAY_HTTP_PROXY  e.g. http://127.0.0.1:17890
#   GATEWAY_ALL_PROXY   e.g. socks5h://127.0.0.1:17891
#   GATEWAY_NO_PROXY    e.g. 127.0.0.1,localhost,::1

set -euo pipefail

HTTP_PROXY_URL="${GATEWAY_HTTP_PROXY:?GATEWAY_HTTP_PROXY is required}"
ALL_PROXY_URL="${GATEWAY_ALL_PROXY:-}"
NO_PROXY_LIST="${GATEWAY_NO_PROXY:-127.0.0.1,localhost,::1}"

ENV_DIR="${HOME}/.config/offline-gateway"
ENV_FILE="${ENV_DIR}/env.sh"

mkdir -p "$ENV_DIR"
cat >"$ENV_FILE" <<EOF
# offline-gateway-proxy managed — source this file to enable proxy in the current shell
export http_proxy="${HTTP_PROXY_URL}"
export https_proxy="${HTTP_PROXY_URL}"
export HTTP_PROXY="${HTTP_PROXY_URL}"
export HTTPS_PROXY="${HTTP_PROXY_URL}"
export ALL_PROXY="${ALL_PROXY_URL}"
export all_proxy="${ALL_PROXY_URL}"
export NO_PROXY="${NO_PROXY_LIST}"
export no_proxy="${NO_PROXY_LIST}"
EOF
chmod 644 "$ENV_FILE"

echo "offline-gateway-proxy enabled (env only)"
echo "  HTTP_PROXY=${HTTP_PROXY_URL}"
echo "  ALL_PROXY=${ALL_PROXY_URL}"
echo "  NO_PROXY=${NO_PROXY_LIST}"
echo "  env_file=${ENV_FILE}"
echo "Run in each shell that needs outbound access:"
echo "  source ${ENV_FILE}"
