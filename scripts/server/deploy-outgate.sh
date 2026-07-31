#!/usr/bin/env bash
# Deploy OutGate CLI + shell hook so `outgate on/off` apply to the current shell.
# Expects:
#   OUTGATE_B64   base64 of libexec outgate script
#   OUTGATE_HTTP / OUTGATE_SOCKS / OUTGATE_NO_PROXY  optional config
set -euo pipefail

OUTGATE_HOME="${HOME}/.outgate"
BIN_DIR="${OUTGATE_HOME}/bin"
LIBEXEC="${OUTGATE_HOME}/libexec"
BIN_FILE="${LIBEXEC}/outgate"
PATH_FILE="${OUTGATE_HOME}/path.sh"
CONFIG_FILE="${OUTGATE_HOME}/config"
MARKER="# outgate managed"

: "${OUTGATE_B64:?OUTGATE_B64 is required}"

mkdir -p "$BIN_DIR" "$LIBEXEC"

decode() {
  if printf '%s' "$1" | base64 -d >/dev/null 2>&1; then
    printf '%s' "$1" | base64 -d
  elif printf '%s' "$1" | base64 -D >/dev/null 2>&1; then
    printf '%s' "$1" | base64 -D
  else
    printf '%s' "$1" | openssl base64 -d -A
  fi
}

decode "$OUTGATE_B64" >"$BIN_FILE"
chmod 755 "$BIN_FILE"

# Shell function hook — `outgate on/off` mutate *current* shell env.
cat >"$PATH_FILE" <<'EOF'
# outgate managed
outgate() {
  local _og_home="${OUTGATE_HOME:-$HOME/.outgate}"
  local _og_cmd="${_og_home}/libexec/outgate"
  if [[ ! -x "$_og_cmd" ]]; then
    echo "outgate: missing ${_og_cmd}; reconnect from OutGate desktop" >&2
    return 127
  fi
  case "${1:-}" in
    on)
      "$_og_cmd" "$@" || return $?
      # shellcheck disable=SC1090
      [[ -f "${_og_home}/env.sh" ]] && . "${_og_home}/env.sh"
      echo "outgate: proxy enabled in this shell"
      ;;
    off)
      "$_og_cmd" "$@" || return $?
      unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY ALL_PROXY all_proxy NO_PROXY no_proxy 2>/dev/null || true
      echo "outgate: proxy disabled in this shell"
      ;;
    *)
      "$_og_cmd" "$@"
      ;;
  esac
}
case ":$PATH:" in
  *":$HOME/.outgate/bin:"*) ;;
  *) export PATH="$HOME/.outgate/bin:$PATH" ;;
esac
EOF

# Shim if someone runs binary without loading hook
cat >"${BIN_DIR}/outgate" <<'EOF'
#!/usr/bin/env bash
echo "outgate: shell hook not loaded in this shell." >&2
echo "  Run:  source ~/.outgate/path.sh" >&2
echo "  Then: outgate $*" >&2
exit 1
EOF
chmod 755 "${BIN_DIR}/outgate"

hook_line='[ -f "$HOME/.outgate/path.sh" ] && . "$HOME/.outgate/path.sh"  # outgate managed'
for f in "${HOME}/.bashrc" "${HOME}/.zshrc" "${HOME}/.profile"; do
  touch "$f"
  if ! grep -qF "outgate managed" "$f" 2>/dev/null; then
    printf '\n%s\n' "$hook_line" >>"$f"
  fi
done

HTTP_URL="${OUTGATE_HTTP:-http://127.0.0.1:17890}"
SOCKS_URL="${OUTGATE_SOCKS:-socks5h://127.0.0.1:17891}"
NO_PROXY_LIST="${OUTGATE_NO_PROXY:-127.0.0.1,localhost,::1}"
cat >"$CONFIG_FILE" <<EOF
${MARKER}
http=${HTTP_URL}
socks=${SOCKS_URL}
no_proxy=${NO_PROXY_LIST}
EOF
chmod 644 "$CONFIG_FILE"

echo "outgate: deployed"
echo "  libexec=${BIN_FILE}"
echo "  hook=${PATH_FILE}"
echo "  config=${CONFIG_FILE}"
echo "  http=${HTTP_URL}"
echo "Load hook in this shell once:"
echo "  source ~/.outgate/path.sh"
echo "Then: outgate on / outgate off  (applies to current shell)"
