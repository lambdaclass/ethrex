#!/bin/sh
# Generate config.json from environment variables (injected via env_file from deployed addresses)
#
# Network-aware: supports local L1 (default) and external L1 (testnet or mainnet).
# Set L1_RPC_URL/L1_CHAIN_ID/L1_EXPLORER_URL/L1_NETWORK_NAME to use an external L1.
# IS_EXTERNAL_L1 is auto-detected from L1_RPC_URL presence but can be overridden.
#
# Public access: set PUBLIC_BASE_URL (e.g., https://l2.example.com) to generate URLs
# accessible from outside the host machine. When set, L2 RPC/Explorer/Metrics URLs
# use PUBLIC_BASE_URL instead of localhost. L1 RPC is proxied through /api/l1-rpc
# to protect API keys.

# Determine L1 RPC URL
L1_RPC_RESOLVED="${L1_RPC_URL:-http://localhost:${TOOLS_L1_RPC_PORT:-8545}}"

# Determine L1 Explorer URL
L1_EXPLORER_RESOLVED="${L1_EXPLORER_URL:-http://localhost:${TOOLS_L1_EXPLORER_PORT:-8083}}"

# Sanitize URLs for safe JSON embedding (strip quotes and backslashes)
L1_RPC_RESOLVED=$(echo "${L1_RPC_RESOLVED}" | tr -d '"\\')
L1_EXPLORER_RESOLVED=$(echo "${L1_EXPLORER_RESOLVED}" | tr -d '"\\')

# Determine L1 Chain ID (default: 9 for local) — must be numeric
L1_CHAIN_ID_RESOLVED="${L1_CHAIN_ID:-9}"
case "$L1_CHAIN_ID_RESOLVED" in
  ''|*[!0-9]*) echo "[entrypoint] WARNING: Invalid L1_CHAIN_ID '${L1_CHAIN_ID_RESOLVED}', defaulting to 9"; L1_CHAIN_ID_RESOLVED="9" ;;
esac

# Determine L2 Chain ID — must be numeric
L2_CHAIN_ID_RESOLVED="${L2_CHAIN_ID:-65536999}"
case "$L2_CHAIN_ID_RESOLVED" in
  ''|*[!0-9]*) echo "[entrypoint] WARNING: Invalid L2_CHAIN_ID '${L2_CHAIN_ID_RESOLVED}', defaulting to 65536999"; L2_CHAIN_ID_RESOLVED="65536999" ;;
esac

# Determine L1 Network Name — sanitize for JSON (strip quotes and backslashes)
L1_NETWORK_NAME_RESOLVED=$(echo "${L1_NETWORK_NAME:-Local}" | tr -d '"\\')

# External L1 flag: auto-detect from L1_RPC_URL, allow override via IS_EXTERNAL_L1
if [ "${IS_EXTERNAL_L1:-}" = "true" ]; then
  IS_EXTERNAL_L1_RESOLVED="true"
elif [ -n "${L1_RPC_URL:-}" ]; then
  IS_EXTERNAL_L1_RESOLVED="true"
else
  IS_EXTERNAL_L1_RESOLVED="false"
fi

# Public access mode: generate external URLs if PUBLIC_BASE_URL is set
PUBLIC_BASE=$(echo "${PUBLIC_BASE_URL:-}" | tr -d '"\\' | sed 's:/*$::')
if [ -n "$PUBLIC_BASE" ]; then
  IS_PUBLIC="true"
  PUBLIC_DOMAIN_RESOLVED=$(echo "${PUBLIC_DOMAIN:-}" | tr -d '"\\')
  # Per-service custom URLs (from Manager), falling back to PUBLIC_BASE + port
  L1_RPC_PUBLIC="${PUBLIC_BASE}/api/l1-rpc"
  L2_RPC_PUBLIC="${PUBLIC_L2_RPC_URL:-${PUBLIC_BASE}:${TOOLS_L2_RPC_PORT:-1729}}"
  L2_EXPLORER_PUBLIC="${PUBLIC_L2_EXPLORER_URL:-${PUBLIC_BASE}:${TOOLS_L2_EXPLORER_PORT:-8082}}"
  L1_EXPLORER_PUBLIC="${PUBLIC_L1_EXPLORER_URL:-${L1_EXPLORER_RESOLVED}}"
  DASHBOARD_PUBLIC="${PUBLIC_DASHBOARD_URL:-${PUBLIC_BASE}}"
  METRICS_PUBLIC="http://localhost:${TOOLS_METRICS_PORT:-3702}/metrics"
else
  IS_PUBLIC="false"
  PUBLIC_DOMAIN_RESOLVED=""
  L1_RPC_PUBLIC="${L1_RPC_RESOLVED}"
  L2_RPC_PUBLIC="http://localhost:${TOOLS_L2_RPC_PORT:-1729}"
  L2_EXPLORER_PUBLIC="http://localhost:${TOOLS_L2_EXPLORER_PORT:-8082}"
  L1_EXPLORER_PUBLIC="${L1_EXPLORER_RESOLVED}"
  DASHBOARD_PUBLIC=""
  METRICS_PUBLIC="http://localhost:${TOOLS_METRICS_PORT:-3702}/metrics"
fi

cat > /usr/share/nginx/html/config.json << EOF
{
  "bridge_address": "${ETHREX_WATCHER_BRIDGE_ADDRESS:-}",
  "on_chain_proposer_address": "${ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS:-}",
  "timelock_address": "${ETHREX_TIMELOCK_ADDRESS:-}",
  "sp1_verifier_address": "${ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS:-}",
  "bridge_l2_address": "0x000000000000000000000000000000000000ffff",
  "l1_rpc": "${L1_RPC_PUBLIC}",
  "l2_rpc": "${L2_RPC_PUBLIC}",
  "l1_explorer": "${L1_EXPLORER_PUBLIC}",
  "l2_explorer": "${L2_EXPLORER_PUBLIC}",
  "l1_chain_id": ${L1_CHAIN_ID_RESOLVED},
  "l2_chain_id": ${L2_CHAIN_ID_RESOLVED},
  "l1_network_name": "${L1_NETWORK_NAME_RESOLVED}",
  "is_external_l1": ${IS_EXTERNAL_L1_RESOLVED},
  "is_public": ${IS_PUBLIC},
  "public_domain": "${PUBLIC_DOMAIN_RESOLVED}",
  "dashboard_url": "${DASHBOARD_PUBLIC}",
  "metrics_url": "${METRICS_PUBLIC}"
}
EOF

echo "[entrypoint] Generated config.json: bridge=${ETHREX_WATCHER_BRIDGE_ADDRESS:-<not set>}, external_l1=${IS_EXTERNAL_L1_RESOLVED}, chain=${L1_CHAIN_ID_RESOLVED}, network=${L1_NETWORK_NAME_RESOLVED}, public=${IS_PUBLIC}"

# If public mode, generate nginx reverse proxy config for L1 RPC API key protection
if [ "$IS_PUBLIC" = "true" ]; then
  # Store L1 RPC proxy location for inclusion in the server block below
  L1_RPC_PROXY_BLOCK="
    location /api/l1-rpc {
        proxy_pass ${L1_RPC_RESOLVED};
        proxy_set_header Content-Type application/json;
        proxy_method POST;
        limit_req zone=l1rpc burst=20 nodelay;
    }"
  # Add rate limit zone to nginx.conf if not present
  if ! grep -q "limit_req_zone.*l1rpc" /etc/nginx/nginx.conf 2>/dev/null; then
    sed -i 's/http {/http {\n    limit_req_zone $binary_remote_addr zone=l1rpc:10m rate=10r\/s;/' /etc/nginx/nginx.conf 2>/dev/null || true
  fi
  echo "[entrypoint] Public mode: L1 RPC proxy enabled at /api/l1-rpc"
else
  L1_RPC_PROXY_BLOCK=""
fi

# Single server block: cache-busting for config.json + optional L1 RPC proxy
# Must be a server block since conf.d/*.conf is included at http level
cat > /etc/nginx/conf.d/app-server.conf << SERVEREOF
server {
    listen 80;
    server_name _;

    location = /config.json {
        root /usr/share/nginx/html;
        expires -1;
        add_header Cache-Control "no-cache, no-store, must-revalidate, max-age=0";
    }
${L1_RPC_PROXY_BLOCK}
    location / {
        root /usr/share/nginx/html;
        index index.html;
    }
}
SERVEREOF
# Remove default server to avoid port conflict
rm -f /etc/nginx/conf.d/default.conf

exec nginx -g "daemon off;"
