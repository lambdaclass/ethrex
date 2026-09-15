# EIP-8141 Frame Transactions Devnet — User Guide

A public devnet for testing [EIP-8141 Frame Transactions](https://eips.ethereum.org/EIPS/eip-8141) running on ethrex.

## Network Details

| Field | Value |
|-------|-------|
| **Chain ID** | `3151908` (`0x301824`) |
| **RPC URL** | `https://rpc1.eip-8141.ethrex.xyz` (also `rpc2` and `rpc3` for other nodes) |
| **Block Explorer (Blockscout)** | `https://explorer.eip-8141.ethrex.xyz` — patched with EIP-8141 Frames tab |
| **Block Explorer (Dora)** | `https://dora.eip-8141.ethrex.xyz` — beacon chain / slot explorer |
| **Faucet** | `https://faucet.eip-8141.ethrex.xyz` |
| **Slot Time** | 6 seconds |
| **Fork** | Osaka (frame tx opcodes enabled from genesis) |
| **Consensus** | 3x ethrex EL + 3x Lighthouse CL |

## Deployed Contracts

| Contract | Address | Description |
|----------|---------|-------------|
| **GasSponsor** | `0x17435cce3d1b4fa2e5f8a08ed921d57c6762a180` | Open paymaster — sponsors anyone whose ERC-20 balance > 0. No signature required. |
| **CanonicalPaymaster** | `0x422a3492e218383753d8006c7bfa97815b44373f` | Signature-gated paymaster — the owner must sign each sponsored tx. 12-hour withdrawal timelock (EIP-8141 canonical design). |
| **MockToken** | `0xb4b46bdaa835f8e4b4d8e208b6559cd267851051` | Returns 1 for any `balanceOf()`. Used by GasSponsor so any sender passes the balance check. |

> **Note:** Contract addresses change on devnet restart since they're deployed post-genesis. Check `INTERNAL-OPS.md` for the deployment procedure.

### GasSponsor (Open Paymaster)

Sponsors gas for anyone who holds tokens. No per-transaction signature from the paymaster is needed — the sender just needs a non-zero ERC-20 balance.

**How it works in a frame tx:**
1. VERIFY frame targets the GasSponsor with `verify()` calldata (`0xfc735e99`)
2. GasSponsor reads the frame tx sender via `TXPARAM(0x02, 0)`
3. Checks the sender's balance on the configured ERC-20 token
4. Calls `APPROVE(0, 0, 2)` to approve as payer

**Functions:**
- `verify()` (`0xfc735e99`) — Check sender token balance, approve as payer
- `setConfig(address _token)` (`0x20e3dbd4`) — Set the ERC-20 token address
- `token()` (`0xfc0c546a`) — Read the configured token address
- `receive()` — Accept ETH (for funding)

**Source:** [`contracts/GasSponsor.yul`](contracts/GasSponsor.yul) — compile with `solc --strict-assembly`

### CanonicalPaymaster (Signature-Gated)

The EIP-8141 canonical paymaster design: the paymaster owner must sign each sponsored transaction. Includes a 12-hour timelocked withdrawal to prevent instant draining (which would mass-invalidate pending sponsored txs in the mempool).

**How it works in a frame tx:**
1. VERIFY frame targets the CanonicalPaymaster with 65-byte calldata: `r(32) || s(32) || v(1)` — the owner's secp256k1 signature over the frame tx sig_hash
2. CanonicalPaymaster reads the sig_hash via `TXPARAM(0x08, 0)`
3. Calls `ecrecover` and verifies the recovered address matches the stored owner
4. Calls `APPROVE(0, 0, 2)` to approve as payer

**Functions:**
- `fallback` (65 bytes calldata) — Verify owner signature, approve as payer
- `owner()` (`0x8da5cb5b`) — Read the owner address
- `requestWithdrawal(address to, uint256 amount)` (`0xdbaf2145`) — Start timelocked withdrawal (owner only)
- `executeWithdrawal()` (`0x9e6371ba`) — Complete withdrawal after 12h delay (owner only)
- `receive()` — Accept ETH

**Source:** [`contracts/CanonicalPaymaster.yul`](contracts/CanonicalPaymaster.yul) — Yul port of the [EIP-8141 reference Solidity contract](https://github.com/ethereum/EIPs/blob/master/assets/eip-8141/CanonicalPaymaster.sol), compiled with `solc --strict-assembly`

### Compiling and Deploying Paymaster Contracts

Both contracts use EIP-8141 opcodes (`TXPARAM`, `APPROVE`) via Yul `verbatim`, so they **must** be compiled with `solc --strict-assembly` (not standard Solidity compilation):

```bash
# Compile
solc --strict-assembly contracts/GasSponsor.yul 2>/dev/null | grep -A1 "Binary" | tail -1
solc --strict-assembly contracts/CanonicalPaymaster.yul 2>/dev/null | grep -A1 "Binary" | tail -1

# Deploy (the deploy-contracts.sh script handles everything)
bash scripts/eip8141-devnet/deploy-contracts.sh https://rpc1.eip-8141.ethrex.xyz <DEPLOYER_KEY>
```

> **solc verbatim bug:** When using `verbatim_3i_0o(hex"AA", 0, 0, 2)` for APPROVE(scope=2), solc (v0.8.28-0.8.30) may optimize the literal `2` to `1`. The fix: assign to a variable first (`let payerScope := 2`). The `deploy-contracts.sh` script verifies the compiled bytecode has the correct scope before deploying.

## Connect MetaMask

1. Open MetaMask → Settings → Networks → Add Network
2. Fill in:
   - **Network Name:** `EIP-8141 Frame Devnet`
   - **RPC URL:** `https://rpc1.eip-8141.ethrex.xyz`
   - **Chain ID:** `3151908`
   - **Currency Symbol:** `ETH`
3. Save and switch

> MetaMask can send regular EIP-1559 transactions on this network. Frame transactions
> (type 0x06) must be submitted programmatically — MetaMask does not support them natively.

## Get Test ETH

### Via Faucet Web UI

Open the faucet URL in your browser, paste your address, and click claim. You'll receive 1 ETH per request.

### Via curl

```bash
curl https://faucet.eip-8141.ethrex.xyz/api/claim \
  -H "Content-Type: application/json" \
  -d '{"address": "0xYourAddress"}'
```

### Via rex CLI

```bash
# Install rex
cargo install --git https://github.com/lambdaclass/rex --locked

# Check balance
rex balance 0xYourAddress --rpc-url https://rpc1.eip-8141.ethrex.xyz
```

## Send Frame Transactions

### Understanding Frame Transaction Structure

A frame transaction (type `0x06`) consists of:

```
[chain_id, nonce, sender, frames, max_priority_fee, max_fee, max_blob_fee, blob_hashes]
```

Each frame has: `[mode, target, gas_limit, data]`

| Mode | Name | What it does |
|------|------|-------------|
| 0 | DEFAULT | General-purpose call, caller = ENTRY_POINT (`0xaa`) |
| 1 | VERIFY | Static validation, must call APPROVE opcode |
| 2 | SENDER | Executes as `tx.sender`, requires sender approved |

### Example 1: Self-Verified Transfer (Simplest)

A frame tx where the sender verifies themselves and sends ETH:

```
Frame 0: VERIFY mode, target=sender
  → EOA default code runs ecrecover, calls APPROVE(scope=3) [sender+payer]

Frame 1: SENDER mode, target=recipient
  → Default code executes the transfer
```

### Example 2: Sponsored Transaction (GasSponsor — open, no signature needed)

```
Frame 0: VERIFY mode, target=sender, scope_restriction=1
  → EOA verifies signature, calls APPROVE(scope=1) [sender only]

Frame 1: VERIFY mode, target=GasSponsor, scope_restriction=2
  → GasSponsor checks sender's token balance, calls APPROVE(scope=2) [payer only]
  → Data: verify() selector (0xfc735e99)

Frame 2: SENDER mode, target=sender
  → Default EOA code executes subcalls from RLP-encoded data
```

### Example 3: Sponsored Transaction (CanonicalPaymaster — owner signs)

```
Frame 0: VERIFY mode, target=sender, scope_restriction=1
  → EOA verifies sender's signature, calls APPROVE(scope=1) [sender only]

Frame 1: VERIFY mode, target=CanonicalPaymaster, scope_restriction=2
  → Paymaster verifies OWNER's signature over sig_hash via ecrecover
  → Calls APPROVE(scope=2) [payer only]
  → Data: r(32) || s(32) || v(1) = 65 bytes (owner's secp256k1 signature)

Frame 2: SENDER mode, target=sender
  → Default EOA code executes subcalls
```

The key difference: GasSponsor lets anyone with tokens get sponsorship automatically. CanonicalPaymaster requires the owner to co-sign each transaction, giving the operator full control over what gets sponsored.

### Using the Test Scripts (Verified Working)

The repo includes Python test scripts that construct frame transactions with the exact RLP encoding ethrex expects:

```bash
cd ethrex/scripts/eip8141-devnet

# Install dependencies (one-time)
python3 -m venv .venv && .venv/bin/pip install web3 eth-keys

# Self-verified frame tx (VERIFY + SENDER, sender pays gas)
.venv/bin/python3 test-frame-tx.py \
  --rpc-url https://rpc1.eip-8141.ethrex.xyz \
  --private-key <YOUR_FUNDED_KEY>

# Sponsored frame tx with GasSponsor (open — sender pays NO gas)
.venv/bin/python3 test-sponsored-tx.py \
  --rpc-url https://rpc1.eip-8141.ethrex.xyz \
  --private-key <YOUR_FUNDED_KEY> \
  --sponsor 0x17435cce3d1b4fa2e5f8a08ed921d57c6762a180

# Sponsored frame tx with CanonicalPaymaster (owner must co-sign)
# --sender-key: the user sending the tx
# --owner-key: the paymaster owner who authorizes the sponsorship
.venv/bin/python3 test-canonical-paymaster.py \
  --rpc-url https://rpc1.eip-8141.ethrex.xyz \
  --sender-key <YOUR_FUNDED_KEY> \
  --owner-key <PAYMASTER_OWNER_KEY> \
  --paymaster 0x422a3492e218383753d8006c7bfa97815b44373f
```

### Using the Viem Fork

The [viem fork](https://github.com/ch4r10t33r/viem/tree/frames) adds TypeScript types and serialization for frame transactions. Note: the viem fork's frame encoding uses separate `mode` + `flags` fields — verify compatibility with ethrex's packed `mode` field (see compatibility note below) before sending transactions.

```bash
git clone -b frames https://github.com/ch4r10t33r/viem
cd viem
npm install
```

Edit the example files in `examples/frame-transactions/` to set:
- RPC URL to the devnet endpoint
- Chain ID to `3151908`
- A funded private key (get ETH from the faucet first)

```bash
npx tsx examples/frame-transactions/simple-self-verified.ts
npx tsx examples/frame-transactions/sponsored-transaction.ts
npx tsx examples/frame-transactions/atomic-batch.ts
```

### Reading Frame Transaction Receipts

Frame tx receipts have extra fields compared to standard receipts:

```bash
curl -s -X POST https://rpc1.eip-8141.ethrex.xyz \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getTransactionReceipt",
    "params": ["<TX_HASH>"],
    "id": 1
  }' | python3 -m json.tool
```

**Extra receipt fields:**
```json
{
  "payer": "0x...",
  "frameReceipts": [
    {"status": true, "gasUsed": "0x...", "logs": [...]},
    {"status": true, "gasUsed": "0x...", "logs": [...]}
  ]
}
```

- `payer` — Address that paid for gas (may differ from sender in sponsored txs)
- `frameReceipts` — Per-frame execution results (status, gas, logs)
- Top-level `status` is `false` if any SENDER frame reverted

## Deploy Your Own Contracts

```bash
# Deploy a standard Solidity contract
rex deploy --contract-path MyContract.sol \
  --private-key $YOUR_KEY \
  --rpc-url https://rpc1.eip-8141.ethrex.xyz \
  --print-address

# Verify deployment
rex code <DEPLOYED_ADDR> --rpc-url https://rpc1.eip-8141.ethrex.xyz
```

> **Note:** Contracts using EIP-8141 opcodes (APPROVE, TXPARAM, FRAMEDATALOAD, FRAMEDATACOPY)
> must be written in Yul and compiled with `solc --strict-assembly`, since `verbatim` is not
> available in Solidity inline assembly.

## Health Checks

```bash
RPC="https://rpc1.eip-8141.ethrex.xyz"

# Block number (increases every ~6s)
curl -s -X POST $RPC -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

# Chain ID
curl -s -X POST $RPC -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'

# Peer count (should be 2)
curl -s -X POST $RPC -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}'

# Paymaster balances
rex balance 0x17435cce3d1b4fa2e5f8a08ed921d57c6762a180 --rpc-url $RPC  # GasSponsor
rex balance 0x422a3492e218383753d8006c7bfa97815b44373f --rpc-url $RPC  # CanonicalPaymaster
```

## Block Explorers

### Blockscout (Frame-Aware)

Open `https://explorer.eip-8141.ethrex.xyz` in your browser. This is a patched Blockscout with EIP-8141 support — frame transactions show a **Frames** tab on the transaction detail page with:
- Per-frame execution mode (DEFAULT / VERIFY / SENDER)
- Target address
- Gas used per frame
- Frame status (success/revert)
- Decoded calldata

To view a frame transaction: search by tx hash or click through from the blocks list. Frame txs display as type `6` in the transactions table.

### Dora (Beacon Chain)

Open `https://dora.eip-8141.ethrex.xyz` for the Dora slot explorer. This shows beacon chain slots, validators, and attestations. Frame txs may appear as "unknown type" in Dora since it doesn't have EIP-8141 awareness — use Blockscout for frame tx details.

## Using Rex for Frame Transactions

[Rex](https://github.com/lambdaclass/rex) (branch `feat/frame-tx`) has native EIP-8141 support via the `rex frame` subcommands. Install from source:

```bash
git clone https://github.com/lambdaclass/rex && cd rex
git checkout feat/frame-tx
cargo install --path cli --locked
```

### Self-verified frame tx

```bash
rex frame send \
  --to 0xRecipient \
  --value 0.01ether \
  --private-key $YOUR_KEY \
  --rpc-url https://rpc1.eip-8141.ethrex.xyz
```

### Sponsored frame tx (GasSponsor)

```bash
rex frame send \
  --to 0xRecipient \
  --value 0.01ether \
  --sponsor 0x17435cce3d1b4fa2e5f8a08ed921d57c6762a180 \
  --sponsor-calldata 0xfc735e99 \
  --private-key $YOUR_KEY \
  --rpc-url https://rpc1.eip-8141.ethrex.xyz
```

### Inspect a frame tx receipt

```bash
rex frame receipt <TX_HASH> --rpc-url https://rpc1.eip-8141.ethrex.xyz
# Output:
# Status:    SUCCESS
# Payer:     0x17435cce...  (sponsor address, not sender)
# Frames:    3
#   Frame 0: OK, gas=0xbb8, logs=0
#   Frame 1: OK, gas=0x132e, logs=0
#   Frame 2: OK, gas=0xa28, logs=0
```

### Dry-run (inspect raw tx without sending)

```bash
rex frame send \
  --to 0xRecipient \
  --value 0.01ether \
  --private-key $YOUR_KEY \
  --rpc-url https://rpc1.eip-8141.ethrex.xyz \
  --dry-run
# Prints: sig_hash, raw_tx hex, size
```

> **Known issue:** `rex frame send` prints the tx hash but then errors trying to parse the receipt
> (standard receipt parser doesn't handle type 6). The tx itself succeeds — use `rex frame receipt`
> separately to see the full frame receipt.

## EIP-8141 Resources

- [EIP-8141 Specification](https://eips.ethereum.org/EIPS/eip-8141)
- [Ethereum Magicians Discussion](https://ethereum-magicians.org/t/frame-transaction/27617)
- [Viem Frame Transactions Fork](https://github.com/ch4r10t33r/viem/tree/frames/examples/frame-transactions)
- [ethrex Implementation Docs](https://github.com/lambdaclass/ethrex/blob/eip-8141-1/docs/eip-8141.md)

## Known Limitations

- **MetaMask:** Cannot construct or sign frame transactions. Use viem fork or raw RPC.
- **Dora:** May display frame txs as "unknown type" — this is cosmetic.
- **Contract compilation:** Contracts using EIP-8141 opcodes must be pure Yul (`solc --strict-assembly`), not Solidity inline assembly.
- **P2P:** Frame txs propagate via full broadcast (`Transactions` message) but NOT via `GetPooledTransactions`. Works fine in 3-node setup.
