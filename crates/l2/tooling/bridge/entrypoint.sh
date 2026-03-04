#!/bin/sh
# Generate config.json from environment variables (injected via env_file from deployed addresses)
cat > /usr/share/nginx/html/config.json << EOF
{
  "bridge_address": "${ETHREX_WATCHER_BRIDGE_ADDRESS:-}",
  "on_chain_proposer_address": "${ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS:-}",
  "timelock_address": "${ETHREX_TIMELOCK_ADDRESS:-}",
  "sp1_verifier_address": "${ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS:-}",
  "bridge_l2_address": "0x000000000000000000000000000000000000ffff",
  "l1_rpc": "http://localhost:8545",
  "l2_rpc": "http://localhost:1729",
  "l1_explorer": "http://localhost:8083",
  "l2_explorer": "http://localhost:8082",
  "l1_chain_id": 9,
  "l2_chain_id": 65536999
}
EOF

echo "[entrypoint] Generated config.json with bridge_address=${ETHREX_WATCHER_BRIDGE_ADDRESS:-<not set>}"
exec nginx -g "daemon off;"
