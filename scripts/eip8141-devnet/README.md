# EIP-8141 Frame Transactions Devnet

A 3-node ethrex devnet for testing [EIP-8141 Frame Transactions](https://eips.ethereum.org/EIPS/eip-8141).

## Network Info

| Field | Value |
|-------|-------|
| Chain ID | `3151908` (hex: `0x301824`) |
| RPC URL | `http://ethrex-mainnet-8:<RPC_PORT>` |
| Block Explorer (Dora) | `http://ethrex-mainnet-8:<DORA_PORT>` |
| Faucet | `http://ethrex-mainnet-8:8080` |
| Slot Time | 6 seconds |
| Fork | Osaka (Fulu) — frame tx opcodes enabled |

## Architecture

- 3x ethrex EL + lighthouse CL validator pairs (Kurtosis)
- Dora block explorer
- chainflag/eth-faucet for distributing test ETH
- CanonicalPaymaster deployed at `<PAYMASTER_ADDR>` (see deploy output)

## Connect MetaMask

1. Settings > Networks > Add Network
2. Fill in:
   - Network Name: `EIP-8141 Devnet`
   - RPC URL: `http://ethrex-mainnet-8:<RPC_PORT>`
   - Chain ID: `3151908`
   - Currency Symbol: `ETH`
3. Save and switch to the network

Note: MetaMask can send regular EIP-1559 txs on this devnet. Frame transactions (type 0x06) must be submitted programmatically.

## Get Test ETH

### Via Faucet UI

Open `http://ethrex-mainnet-8:8080` in your browser and enter your address.

### Via curl

```bash
curl http://ethrex-mainnet-8:8080/api/claim \
  -H "Content-Type: application/json" \
  -d '{"address": "0xYourAddress"}'
```

### Via rex (from a pre-funded account)

```bash
rex transfer --to 0xYourAddress --value 10ether \
  --private-key <PREFUNDED_KEY> \
  --rpc-url http://ethrex-mainnet-8:<RPC_PORT>
```

## Test Frame Transactions

### Using the viem fork

```bash
git clone -b frames https://github.com/ch4r10t33r/viem
cd viem
npm install

# Edit examples/frame-transactions/simple-self-verified.ts:
# - Set transport to http("http://ethrex-mainnet-8:<RPC_PORT>")
# - Set chain ID to 3151908
# - Set privateKey to a funded account key

npx tsx examples/frame-transactions/simple-self-verified.ts
```

Available examples:
- `simple-self-verified.ts` — VERIFY + SENDER (self-relay, EOA signs with ECDSA)
- `sponsored-transaction.ts` — Gas sponsorship via CanonicalPaymaster
- `atomic-batch.ts` — Two SENDER frames with atomic batching (approve + swap)

### Verify a transaction

```bash
# Check receipt — frame txs have extra fields: "payer" and "frameReceipts"
curl -s -X POST http://ethrex-mainnet-8:<RPC_PORT> \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getTransactionReceipt",
    "params": ["<TX_HASH>"],
    "id": 1
  }' | python3 -m json.tool
```

Expected receipt fields for frame txs:
```json
{
  "payer": "0x...",
  "frameReceipts": [
    { "status": true, "gasUsed": "0x...", "logs": [...] },
    { "status": true, "gasUsed": "0x...", "logs": [...] }
  ]
}
```

## Deploy Your Own Contracts

```bash
# Using rex
rex deploy \
  --contract-path MyContract.sol \
  --private-key <YOUR_KEY> \
  --rpc-url http://ethrex-mainnet-8:<RPC_PORT> \
  --print-address

# Check deployment
rex code <DEPLOYED_ADDR> --rpc-url http://ethrex-mainnet-8:<RPC_PORT>
```

## Health Checks

```bash
RPC="http://ethrex-mainnet-8:<RPC_PORT>"

# Block number (should increase every ~6s)
curl -s -X POST $RPC -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# Chain ID
curl -s -X POST $RPC -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'

# Peer count (should be 2 for a 3-node network)
curl -s -X POST $RPC -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}'
```

## Deployment

### From scratch

```bash
# On your local machine
cd ethrex
git checkout eip-8141-devnet
bash scripts/eip8141-devnet/deploy.sh
```

### Tear down

```bash
ssh admin@ethrex-mainnet-8
kurtosis enclave stop eip8141
kurtosis enclave rm eip8141 --force
docker stop $(docker ps -q) 2>/dev/null || true
```

## Implementation Details

- Frame tx P2P works in the 3-node setup via the `Transactions` (0x02) message — full tx bodies are broadcast to all peers
- The `GetPooledTransactions` path does NOT support frame txs, but this is unused when all peers receive full broadcasts
- Frame tx opcodes (APPROVE, TXPARAM, FRAMEDATALOAD, FRAMEDATACOPY) are activated via Osaka fork (`fulu_fork_epoch: 0` in Kurtosis config)
- Kurtosis generates its own genesis with pre-funded accounts (keys visible via `kurtosis enclave inspect eip8141`)
