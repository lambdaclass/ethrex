#!/bin/sh
# Generate config.json from environment variables (injected via env_file from deployed addresses)
#
# Network-aware: supports local L1 (default) and external L1 (testnet or mainnet).
# Set L1_RPC_URL/L1_CHAIN_ID/L1_EXPLORER_URL/L1_NETWORK_NAME to use an external L1.
# IS_EXTERNAL_L1 is auto-detected from L1_RPC_URL presence but can be overridden.

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

cat > /usr/share/nginx/html/config.json << EOF
{
  "bridge_address": "${ETHREX_WATCHER_BRIDGE_ADDRESS:-}",
  "on_chain_proposer_address": "${ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS:-}",
  "timelock_address": "${ETHREX_TIMELOCK_ADDRESS:-}",
  "sp1_verifier_address": "${ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS:-}",
  "bridge_l2_address": "0x000000000000000000000000000000000000ffff",
  "l1_rpc": "${L1_RPC_RESOLVED}",
  "l2_rpc": "http://localhost:${TOOLS_L2_RPC_PORT:-1729}",
  "l1_explorer": "${L1_EXPLORER_RESOLVED}",
  "l2_explorer": "http://localhost:${TOOLS_L2_EXPLORER_PORT:-8082}",
  "l1_chain_id": ${L1_CHAIN_ID_RESOLVED},
  "l2_chain_id": ${L2_CHAIN_ID_RESOLVED},
  "l1_network_name": "${L1_NETWORK_NAME_RESOLVED}",
  "is_external_l1": ${IS_EXTERNAL_L1_RESOLVED},
  "metrics_url": "http://localhost:${TOOLS_METRICS_PORT:-3702}/metrics"
}
EOF

echo "[entrypoint] Generated config.json with bridge_address=${ETHREX_WATCHER_BRIDGE_ADDRESS:-<not set>}, is_external_l1=${IS_EXTERNAL_L1_RESOLVED}, l1_chain_id=${L1_CHAIN_ID_RESOLVED}, l1_network=${L1_NETWORK_NAME_RESOLVED}"
exec nginx -g "daemon off;"
