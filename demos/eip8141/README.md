# EIP-8141 Frame Transaction Demo

Interactive demo of EIP-8141 frame transactions on a local ethrex node. Users authenticate with biometrics (Touch ID, Face ID) to sign Ethereum transactions — no seed phrases or browser extensions needed.

## What it does

Frame transactions bundle multiple execution frames into a single transaction:

- **VERIFY frame** — runs on-chain signature verification (WebAuthn P256)
- **SENDER frame** — executes the intended operation (transfer, contract call, etc.)
- **DEFAULT frame** — general-purpose execution (deploy, batch ops)

The demo has four tabs showing different patterns:

| Tab | What it demonstrates |
|-----|---------------------|
| **Simple Send** | Create a passkey account, send ETH to an address |
| **Sponsored ERC20 Send** | Send ERC20 tokens with gas paid by a sponsor contract |
| **Batch Ops** | Execute multiple operations atomically in one tx |
| **Deploy + Execute** | Deploy a contract and call it in a single tx |

## Architecture

```
Browser (React + WebAuthn)         http://localhost:5173
    |
    |-- reads balances ----> ethrex node             http://localhost:8545
    |
    +-- signs & submits ---> TypeScript Backend      http://localhost:3000
                               |
                               +---> ethrex node (eth_sendRawTransaction)

Blockscout Explorer (optional)     https://localhost:8083 (local) or https://<your-domain>:8082 (prod)
    |
    +-- indexes blocks ----> ethrex node
```

## Prerequisites

| Tool | Version | What for |
|------|---------|----------|
| **Rust** | stable | Building ethrex |
| **Node.js** | 18+ | Backend and frontend |
| **solc** | 0.8.28+ | Contract compilation (only if regenerating genesis) |
| **Docker** | latest | Blockscout (optional) |

## Quick Start

You need **3 terminals** (4 if running Blockscout).

### Terminal 1: ethrex node

```bash
cd demos/eip8141
make start-node
```

This runs `cargo run --bin ethrex -- --network genesis.json --http.port 8545 --dev` from the ethrex repo root. The `--dev` flag enables auto-mining (one block per transaction).

Wait until you see `Blockchain synced` or similar before proceeding.

### Terminal 2: Backend

```bash
cd demos/eip8141
make install       # first time only — installs npm dependencies
make start-backend
```

Starts the TypeScript backend (Hono + Node) on port 3000. You should see:

```
[eip8141-backend] Listening on http://localhost:3000
```

### Terminal 3: Frontend

```bash
cd demos/eip8141
make start-frontend
```

Starts the React frontend (Vite) on HTTPS port 5173. Open **https://localhost:5173** in your browser. Transaction hashes automatically link to Blockscout at `https://<hostname>:8083`. To override, set `VITE_BLOCKSCOUT_URL` in `frontend/.env`.

### Terminal 4: Blockscout (optional)

```bash
# Clone the patched Blockscout backend (one-time setup)
git clone -b eip-8141-support git@github.com:lambdaclass/ethrex-blockscout.git

# Clone the upstream Blockscout frontend as a sibling directory (one-time setup)
git clone https://github.com/blockscout/frontend.git blockscout-frontend
cd blockscout-frontend && git checkout 3cb3c3122 && cd ..

# Apply the custom EIP-8141 Frames tab overlay
cp ethrex-blockscout/frontend/ui/pages/Transaction.tsx blockscout-frontend/ui/pages/Transaction.tsx
cp ethrex-blockscout/frontend/ui/tx/TxFrames.tsx blockscout-frontend/ui/tx/TxFrames.tsx
mkdir -p blockscout-frontend/ui/tx/frames
cp ethrex-blockscout/frontend/ui/tx/frames/*.tsx blockscout-frontend/ui/tx/frames/
cp ethrex-blockscout/frontend/types/api/transaction.ts blockscout-frontend/types/api/transaction.ts
```

**Step 1: Required ethrex compatibility settings.** Add these to `ethrex-blockscout/docker-compose/envs/common-blockscout.env`:
```
INDEXER_DISABLE_EMPTY_BLOCKS_SANITIZER=true
INDEXER_DISABLE_INTERNAL_TRANSACTIONS_FETCHER=true
```

Without these, the Blockscout backend will crash-loop:
- `EmptyBlocksSanitizer` crashes because ethrex returns `nil` instead of `[]` for the transactions field on empty blocks.
- `InternalTransactionsFetcher` spams `debug_traceTransaction` errors because ethrex can't trace old blocks.

**Step 2: Set the RPC URL for Docker networking.** In `ethrex-blockscout/docker-compose/envs/common-blockscout.env`, the `ETHEREUM_JSONRPC_HTTP_URL` and related URLs must point to the ethrex node as reachable from inside the Docker container. On Linux, use the Docker Compose network gateway (typically `172.18.0.1`), NOT `host.docker.internal` (macOS only) or `localhost` (resolves to the container itself). Find the correct gateway with:
```bash
docker network inspect $(docker compose -f docker-compose/docker-compose.yml ps -q backend | head -1 | xargs docker inspect --format '{{range .NetworkSettings.Networks}}{{.NetworkID}}{{end}}') --format '{{range .IPAM.Config}}{{.Gateway}}{{end}}'
```
Or after starting containers: `docker inspect backend --format '{{range .NetworkSettings.Networks}}{{.Gateway}}{{end}}'`

Set all RPC URLs to this gateway:
```
ETHEREUM_JSONRPC_HTTP_URL=http://172.18.0.1:8545/
ETHEREUM_JSONRPC_FALLBACK_HTTP_URL=http://172.18.0.1:8545/
ETHEREUM_JSONRPC_TRACE_URL=http://172.18.0.1:8545/
ETHEREUM_JSONRPC_FALLBACK_TRACE_URL=http://172.18.0.1:8545/
```

**Step 3: Set the frontend host/protocol.** Edit `ethrex-blockscout/docker-compose/envs/common-frontend.env`. The `NEXT_PUBLIC_*` vars are baked into the frontend at Docker build time. They must match the URL users will access Blockscout from in their browser. If you set these to `localhost`, the page will only work from the server itself — browsers on other machines will fail with "Something went wrong" because client-side API calls go to `localhost` (their own machine).

For local development:
```
NEXT_PUBLIC_API_HOST=localhost:8082
NEXT_PUBLIC_API_PROTOCOL=http
NEXT_PUBLIC_APP_HOST=localhost:8082
NEXT_PUBLIC_APP_PROTOCOL=http
```

For remote/Tailscale access (replace with your hostname):
```
NEXT_PUBLIC_API_HOST=your-server.tail12345.ts.net:8082
NEXT_PUBLIC_API_PROTOCOL=http
NEXT_PUBLIC_APP_HOST=your-server.tail12345.ts.net:8082
NEXT_PUBLIC_APP_PROTOCOL=http
```

For HTTPS deployments behind a reverse proxy:
```
NEXT_PUBLIC_API_HOST=demo.eip-8141.ethrex.xyz:8082
NEXT_PUBLIC_API_PROTOCOL=https
NEXT_PUBLIC_APP_HOST=demo.eip-8141.ethrex.xyz:8082
NEXT_PUBLIC_APP_PROTOCOL=https
NEXT_PUBLIC_API_WEBSOCKET_PROTOCOL=wss
```

**Changing `NEXT_PUBLIC_*` vars requires rebuilding the frontend image** (`docker compose build --no-cache frontend`). Restarting the container is not enough.

**Step 4: Use the patched frontend, not the upstream image.** Edit `ethrex-blockscout/docker-compose/services/frontend.yml`. Replace the upstream image with a local build pointing to the patched frontend source:
```yaml
services:
  frontend:
    build:
      context: /path/to/blockscout-frontend
      dockerfile: Dockerfile
    restart: always
    container_name: 'frontend'
    env_file:
      -  ../envs/common-frontend.env
```

If `frontend.yml` uses `image: ghcr.io/blockscout/frontend:latest`, the upstream frontend will be pulled instead of the patched one. The upstream frontend does not handle type 6 transactions and will show "Something went wrong" on frame transaction pages.

If using a reverse proxy (e.g. Caddy) in front of Blockscout, remap the nginx port to avoid conflicts. In `docker-compose/services/nginx.yml`, change `published: 8082` to a non-conflicting port (e.g. `18082`) and configure the proxy to forward to it. Set `VITE_BLOCKSCOUT_URL` in `frontend/.env` to the public Blockscout URL (e.g. `https://demo.eip-8141.ethrex.xyz:8082`).

**Step 5: Build and start.**
```bash
cd ethrex-blockscout
docker compose -f docker-compose/docker-compose.yml up -d --build
```

First run builds backend (Elixir, ~5 min) and frontend (Next.js, ~10 min, requires **8 GB+ RAM** or swap). After starting, verify the backend can reach ethrex:
```bash
docker logs backend 2>&1 | grep -i 'eaddrnotavail\|connection refused'
```
If you see connection errors, the RPC URL is wrong (see Step 2). After the proxy starts, verify the frontend loads:
```bash
# From the server
curl -s -o /dev/null -w '%{http_code}' http://localhost:8082
# Should return 200. If 502, restart the proxy:
docker compose -f docker-compose/docker-compose.yml restart proxy
```

Frame transactions show a **Frames** tab on the transaction detail page with per-frame mode, target, gas, status, and decoded calldata.

The nginx proxy serves HTTPS on port 8083 using a self-signed certificate. To avoid browser warnings, trust the cert in your OS keychain:
```bash
# macOS
sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain \
  ethrex-blockscout/docker-compose/proxy/certs/selfsigned.crt
```

To check status:
```bash
docker compose -f docker-compose/docker-compose.yml ps
```

To stop:
```bash
docker compose -f docker-compose/docker-compose.yml down
```

## Using the Demo

### 1. Register a passkey

On the **Simple Send** tab, click **Create Passkey Account**. Your browser prompts for biometric authentication (Touch ID, Face ID, etc.). This creates a P256 key pair and deploys a new WebAuthnP256Account contract via the factory.

The account panel on the right shows your account address, ETH balance, and DEMO token balance. The dev account auto-funds new accounts with ETH and 1,000,000 DEMO tokens.

### 2. Simple Send

Enter a recipient address and amount, click **Send**. The browser prompts for biometric auth again — this signs the transaction's sig_hash as a WebAuthn challenge. The backend builds a 2-frame transaction:

1. **VERIFY** — WebAuthnP256Account checks the P256 signature on-chain
2. **SENDER** — Executes the ETH transfer from your account

### 3. Sponsored ERC20 Send

Same flow, but sends ERC20 tokens instead of ETH. The GasSponsor contract pays for gas. The transaction has 3 frames:

1. **VERIFY** — WebAuthnP256Account verifies your signature
2. **VERIFY** — GasSponsor checks you hold DEMO tokens, approves as gas payer
3. **SENDER** — Your account calls `MockERC20.transfer()`

### 4. Batch Ops

Add multiple operations (address + value + calldata), submit them all atomically. One VERIFY frame + N SENDER frames.

### 5. Deploy + Execute

Deploy a contract and call it in the same transaction using the [deterministic deployment proxy](https://github.com/Arachnid/deterministic-deployment-proxy) (CREATE2 factory). Three frames: VERIFY + DEFAULT (deploy via proxy) + SENDER (execute on deployed contract).

### Transaction History

Each tab shows a transaction history with per-frame results. Every frame shows its mode (VERIFY/SENDER/DEFAULT), status (OK/REVERTED), and gas used.

## Pre-deployed Contracts

These contracts are injected into `genesis.json` at fixed addresses:

| Contract | Address | Description |
|----------|---------|-------------|
| GasSponsor | `0x1000000000000000000000000000000000000001` | Verifies sender holds ERC20 tokens, approves as gas payer (scope=1). Pre-funded with 100 ETH. |
| MockERC20 | `0x1000000000000000000000000000000000000002` | Minimal ERC20 token ("DEMO"). No access control on `mint()`. |
| WebAuthnVerifier | `0x1000000000000000000000000000000000000004` | Helper that wraps the WebAuthn verification logic for Yul contracts. |
| WebAuthnP256AccountFactory | `0x1000000000000000000000000000000000000005` | Factory that deploys per-user WebAuthnP256Account contracts. Initialized at backend startup with the account initcode. Each passkey registration deploys a new account via `factory.deploy(pubKeyX, pubKeyY)`. |
| Deterministic Deployment Proxy | `0x4e59b44847b379578588920ca78fbf26c0b4956c` | CREATE2 factory ([Arachnid](https://github.com/Arachnid/deterministic-deployment-proxy)). Used by deploy-execute to deploy contracts from frame transactions. |

The GasSponsor and WebAuthnP256Account are compiled from **Yul** (`contracts/yul/`) because they use `verbatim` for EIP-8141 custom opcodes (APPROVE `0xAA`, TXPARAMLOAD `0xB0`). The factory is also Yul. MockERC20 and WebAuthnVerifier are standard Solidity compiled with `--via-ir`.

## Dev Account

The genesis pre-funds a dev account that the backend uses to sign and submit transactions, fund new passkey accounts, and mint tokens:

- **Address:** `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` (Hardhat #0)
- **Private key:** `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80`

This is a well-known test key. Never use it on mainnet.

## Blockscout Details

### What's patched

The `eip-8141-support` branch of [lambdaclass/ethrex-blockscout](https://github.com/lambdaclass/ethrex-blockscout/tree/eip-8141-support) adds:

- **Type 6 frame transaction parsing** — Blockscout's indexer crashes on unknown tx types. The patch adds a `do_elixir_to_params` clause in `transaction.ex` for type 6 (no `gas`, `input`, `value`, or `to` fields; no ECDSA signature).
- **Local smart-contract-verifier** — Enables contract verification with `solc --via-ir`.
- **Demo configuration** — Chain ID 1729, RPC port 8545, no ads.

### Contract verification

MockERC20 and WebAuthnVerifier are verifiable via the Blockscout API. After Blockscout is running and has indexed the genesis contracts, verify them:

```bash
cd demos/eip8141
BLOCKSCOUT_URL=http://localhost:8082 make verify
```

This uses the v1 API (`verifysourcecode` with `solidity-standard-json-input` format) because the contracts are compiled with `--via-ir`, and the v2 API's standard-json-input endpoint is unavailable in some Blockscout versions.

The Yul contracts (GasSponsor, WebAuthnP256Account) use `verbatim` custom opcodes and can't be auto-verified — Blockscout's verifier doesn't support `solc --strict-assembly`.

### Services

| Service | Port | Description |
|---------|------|-------------|
| Blockscout frontend (nginx) | 8082 (HTTP), 8083 (HTTPS) | Web UI |
| Blockscout backend | 4000 (internal) | Indexer + API |
| Smart contract verifier | 8050 (internal) | Solidity/Vyper verification |
| PostgreSQL | 7432 (internal) | Blockscout database |
| Redis | 6379 (internal) | Caching |

## Regenerating genesis.json

If you modify the contracts, regenerate the genesis:

```bash
cd demos/eip8141
make genesis
```

This compiles all contracts and runs `scripts/build-genesis.sh`, which injects the runtime bytecodes into the L1 dev genesis with chain ID 1729.

## Project Structure

```
demos/eip8141/
  contracts/
    src/           Solidity sources (documented, used for verification)
    lib/           Solidity libraries (FrameOps, WebAuthnP256, ECDSA, P256)
    yul/           Yul sources (compiled for genesis — use verbatim opcodes)
    deps/          Third-party deps (solady Base64)
  backend/
    src/
      index.ts           Hono server entry point
      frame-tx.ts        Frame transaction RLP encoding
      dev-account.ts     Dev account utilities (fund, mint tokens)
      rpc.ts             JSON-RPC helpers
      webauthn.ts        WebAuthn credential encoding
      routes/
        register.ts        POST /api/register — create passkey account
        sig-hash.ts        POST /api/sig-hash — get tx skeleton + sig hash
        simple-send.ts     POST /api/simple-send — submit simple send
        sponsored-send.ts  POST /api/sponsored-send — submit sponsored send
        batch-ops.ts       POST /api/batch-ops — submit batch operations
        deploy-execute.ts  POST /api/deploy-execute — submit deploy+execute
  frontend/
    src/
      App.tsx              Tab layout with 4 demo tabs
      components/
        AccountPanel.tsx   Shows address, ETH balance, token balance
        SimpleSend.tsx     Simple ETH send tab
        SponsoredSend.tsx  Sponsored ERC20 send tab
        BatchOps.tsx       Batch operations tab
        DeployExecute.tsx  Deploy + execute tab
        TxResult.tsx       Per-frame receipt display
      lib/
        api.ts             Backend API client
        chain.ts           Viem chain config (reads from ethrex)
        passkey.ts         WebAuthn passkey creation and signing
  genesis.json             Pre-built genesis with compiled contracts
  jwt.hex                  JWT secret for ethrex auth
  scripts/
    build-genesis.sh       Compiles contracts and injects into genesis
    verify-contracts.sh    Verifies MockERC20 and WebAuthnVerifier on Blockscout
    verify-contracts.py    Python verification logic (called by .sh)
  Makefile                 Build and run commands
```

## Redeploying

To kill everything, wipe databases, and restart fresh:

```bash
cd demos/eip8141

# Without Blockscout
make redeploy

# With Blockscout (wipes Blockscout DB too)
make redeploy-full BLOCKSCOUT_REPO=/path/to/ethrex-blockscout
```

**Important paths:**
- ethrex dev DB: `~/Library/Application Support/ethrex/dev/` (macOS) or `~/.local/share/ethrex/dev/` (Linux)
- Blockscout Postgres data: `ethrex-blockscout/docker-compose/services/blockscout-db-data/` (bind mount relative to `services/` subdir, NOT `docker-compose/`)

## Troubleshooting

**"Failed to execute transaction" in backend logs**
The VERIFY frame likely failed — check that the passkey account was registered and has the correct public key. Re-register by refreshing the page.

**Blockscout backend crash-looping**
Check `docker logs backend` for `Protocol.UndefinedError: protocol Enumerable not implemented for nil`. This means the ethrex compatibility settings are missing. Add `INDEXER_DISABLE_EMPTY_BLOCKS_SANITIZER=true` and `INDEXER_DISABLE_INTERNAL_TRANSACTIONS_FETCHER=true` to `common-blockscout.env`, then recreate the container with `docker compose up -d backend` (NOT `docker compose restart` — `restart` does not re-read env files).

**Blockscout transaction page shows "Something went wrong"**
Two possible causes:
1. The frontend is using the upstream Blockscout image (`ghcr.io/blockscout/frontend:latest`) instead of the patched one. Check `docker-compose/services/frontend.yml`: it must use `build:` pointing to the patched frontend directory, not `image:`. See Step 4 in the Blockscout setup above.
2. The `NEXT_PUBLIC_API_HOST` in `common-frontend.env` is set to `localhost`. When a browser on a different machine loads the page, client-side JavaScript sends API requests to `localhost` (the user's machine), which fails. Set `NEXT_PUBLIC_API_HOST` to the server's hostname or IP, then rebuild the frontend image. See Step 3 in the Blockscout setup above.

**Blockscout backend can't connect to ethrex (`eaddrnotavail` or `connection refused` in logs)**
The `ETHEREUM_JSONRPC_HTTP_URL` in `common-blockscout.env` points to the wrong address. On Linux, `host.docker.internal` does not work (macOS only). Use the Docker Compose network gateway instead. Find it with `docker inspect backend --format '{{range .NetworkSettings.Networks}}{{.Gateway}}{{end}}'` (typically `172.18.0.1` for docker-compose networks). Update all `JSONRPC` URLs, then recreate the backend with `docker compose up -d backend`.

**Blockscout shows empty blocks / no transactions**
Blockscout needs a few seconds to catch up. Wait 10-15 seconds after submitting a transaction, then refresh.

**Blockscout 502 Bad Gateway**
The nginx proxy container resolves other containers by Docker-internal IP. If you rebuild or recreate a container (e.g., `docker compose up -d --force-recreate frontend`), the proxy still has the old IP cached. Fix by restarting the proxy after any container recreate:
```bash
cd ethrex-blockscout
docker compose -f docker-compose/docker-compose.yml restart proxy
```
If the 502 persists, check that the backend and frontend containers are actually running (`docker compose ps`) and inspect their logs (`docker logs frontend`, `docker logs backend`).

**Blockscout frontend env changes not taking effect**
The Next.js frontend bakes `NEXT_PUBLIC_*` env vars at build time. Changing `common-frontend.env` and restarting the container is **not enough** — you must rebuild the image:
```bash
cd ethrex-blockscout
docker compose -f docker-compose/docker-compose.yml build --no-cache frontend
docker compose -f docker-compose/docker-compose.yml up -d --force-recreate frontend
docker compose -f docker-compose/docker-compose.yml restart proxy  # required after recreate
```

**ethrex stuck / not mining blocks**
If the demo is stuck at "Deploying smart account" or similar, check if blocks are advancing:
```bash
curl -s -X POST http://localhost:8545 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```
If the block number doesn't change, restart ethrex. Also check `txpool_content` for stuck pending transactions. After restarting ethrex, any pending transactions from before the restart will need to be resubmitted (refresh the demo page).

**"JSON Parse error: Unexpected identifier 'Version'" in frontend**
The demo backend is returning an HTML/text error instead of JSON. This usually happens after a DB wipe — the WebAuthnP256AccountFactory goes back to uninitialized state, so `deployAccount()` reverts with "not initialized". Fix by restarting the backend (`make start-backend`), which calls `ensureFactoryInitialized()` at startup.

**"Exceeded max amount of blocks to re-execute for tracing"**
ethrex limitation for `debug_traceTransaction` on old blocks. Non-blocking — Blockscout still indexes transactions, just can't show internal transaction details for older blocks.

**Contract not showing as "Contract" in Blockscout**
Genesis-deployed contracts aren't automatically detected. Send any transaction to the contract address to trigger detection.
