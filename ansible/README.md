# Ethrex Deployment Ansible

This set of Ansible playbooks automate provisioning and deployment of the Ethrex components.



## 📋 General Usage

Set the corresponding ENVVARS (detailed below) for any given component and run the corresponding `make` target.

To list all available targets:

```bash
make help
```

---

# ⚙️ Targets

Each target corresponds to a component, and expects certain environment variables to be defined before running.


### `make inventory`

**Generates the `inventory.ini` file for Ansible**

Requires at least one of the following environment variables:

- `DATABASE_IP`
- `EXPLORER_FRONTEND_IP`
- `EXPLORER_BACKEND_IP`
- `L1_IP`
- `L2_IP`
- `PROVER_EXEC_IP`
- `PROVER_SP1_IP`
- `METRICS_IP`

Multiple IPs should be comma-separated:
```bash
export L2_IP=1.2.3.4,11.33.55.77,...
```

---

### `make database`

**Installs and configures the Postgres database**

Env vars:
- `PGUSER`
- `PGPASSWORD`

---

### `make ethrex-l1`

**Deploys the Ethrex Layer 1 node**

Env vars:
- `NETWORK`
- `EVM`
- `BOOTNODES`

---

### `make ethrex-l2`

**Deploys the Ethrex Layer 2 node**

Env vars:
- `GENESIS_JSON_FILE`
- `COMMON_BRIDGE_ADDRESS`
- `ON_CHAIN_PROPOSER_ADDRESS`
- `COMMITTER_PK`
- `PROOF_SENDER_PK`
- `COINBASE_ADDRESS`
- `TESTNET_RPC`
- `INFURA_API_KEY`

---

### `make ethrex-prover-exec`

**Installs the prover execution node**

Env vars:
- `ETHREX_SEQUENCER_ADDRESS`
- `ETHREX_SEQUENCER_PORT`

---

### `make ethrex-prover-sp1`

**Installs the prover SP1 node**

Env vars:
- `ETHREX_SEQUENCER_ADDRESS`
- `ETHREX_SEQUENCER_PORT`

---

### `make explorer_backend`

**Installs the Explorer backend service**

Env vars:
- `DATABASE_URL`
- `ETHEREUM_JSONRPC_VARIANT`
- `ETHEREUM_JSONRPC_HTTP_URL`
- `COIN`
- `COIN_NAME`

Other values are internally set, but they can be modified inside the `Makefile`
- `PORT=3001`
- `API_V2_ENABLED=true`
- `MIX_ENV=prod`
- etc.

---

### `make explorer_frontend`

**Installs the Explorer frontend service**

Env vars:
- `NEXT_PUBLIC_API_HOST`
- `NEXT_PUBLIC_APP_HOST`
- `NEXT_PUBLIC_NETWORK_NAME`
- `NEXT_PUBLIC_NETWORK_LOGO`
- `NEXT_PUBLIC_NETWORK_LOGO_DARK`
- `NEXT_PUBLIC_NETWORK_ICON`
- `NEXT_PUBLIC_NETWORK_ICON_DARK`
- `NEXT_PUBLIC_STATS_API_HOST`

Hardcoded defaults include ports, protocol, banner/ads settings, etc.

---

### `make metrics`

**Installs monitoring stack (Grafana, Prometheus, Loki, etc)**

Env vars:
- `ALERTS_SLACK_CHANNEL`
- `ALERTS_SLACK_TOKEN`
- `GRAFANA_PASSWORD`

---

## 📎 Notes

- All Ansible runs use the generated `inventory.ini`.
- Most targets assume the remote machine supports Python 3 at `/usr/bin/python3`.



## 🧪 Example usage

Typical usage example

```bash
# Create ansible inventory
make inventory L1_IP=1.2.3.4,9.8.7.6,ethrex-l2-1 DATABASE_IP=11.33.55.77 METRICS_IP=ethrex-metrics-prod

# Execute L1 ansible
make ethrex-l1 NETWORK=hoodi EVM=levm BOOTNODES=enode://...

# Install PostrgeSQL database
make database DATABASE_IP=10.0.0.10 PGUSER=postgres PGPASSWORD=mysecret

# Install Metrics
make metrics GRAFANA_PASSWORD=mysecret ALERTS_SLACK_CHANNEL=#testALERTS_SLACK_TOKEN
```
```
