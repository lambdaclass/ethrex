# ZisK Proving Guide

This guide covers the **one-time setup** required to run ethrex L2 with the ZisK prover backend.

## Proof Flow Overview

ZisK uses a two-stage proving system:

```
Program Execution → STARK Proof → SNARK Proof → On-chain Verification
                    (GPU/CPU)      (CPU)           (sequencer)
```

| Stage | Proving Key | Hardware | Purpose |
|-------|-------------|----------|---------|
| STARK | `~/.zisk/provingKey` | GPU (fast) or CPU | Generate large proof |
| SNARK | `~/.zisk/provingKeySnark` | CPU | Compress for on-chain verification |

## One-Time Setup

### 1. Compile ethrex

**With GPU acceleration (recommended):**
```bash
COMPILE_CONTRACTS=true make -C crates/l2 build-prover-zisk GPU=true
```

**CPU only:**
```bash
COMPILE_CONTRACTS=true make -C crates/l2 build-prover-zisk
```

This generates the ELF at `crates/guest-program/bin/zisk/out/riscv64ima-zisk-elf`

### 2. Clone and build ZisK

```bash
git clone git@github.com:0xPolygonHermez/zisk.git
cd zisk && git checkout feature/pre-develop-0.16.0-stable
```

**With GPU:**
```bash
cargo build --release --features gpu
```

**CPU only:**
```bash
cargo build --release
```

### 3. Install ZisK binaries

Follow steps 3 to 7 in the [ZisK installation guide](https://github.com/0xPolygonHermez/zisk/blob/feature/bn128/book/getting_started/installation.md#option-2-building-from-source).

This installs `cargo-zisk`, `ziskemu`, and `libzisk_witness.so` to `~/.zisk/bin`.

### 4. Download proving keys

```bash
# STARK proving key
wget https://storage.googleapis.com/zisk-setup/zisk-provingkey-pre-0.16.0.tar.gz

# SNARK proving key
wget https://storage.googleapis.com/zisk-setup/zisk-provingkey-pre-0.16.0-plonk.tar.gz
```

### 5. Extract proving keys

```bash
tar -xzf zisk-provingkey-pre-0.16.0.tar.gz -C ~/.zisk/
tar -xzf zisk-provingkey-pre-0.16.0-plonk.tar.gz -C ~/.zisk/
```

After extraction you should have:
- `~/.zisk/provingKey/` - STARK proving key
- `~/.zisk/provingKeySnark/` - SNARK proving key

### 6. ROM setup (once per ELF)

This step must be run whenever the guest program ELF changes:

```bash
cargo-zisk rom-setup -e <PATH_TO_ELF> -k ~/.zisk/provingKey
```

For ethrex:
```bash
cargo-zisk rom-setup \
    -e crates/guest-program/bin/zisk/out/riscv64ima-zisk-elf \
    -k ~/.zisk/provingKey
```

### 7. Verify setup (optional)

```bash
cargo-zisk check-setup -k ~/.zisk/provingKey -a
```

### 8. Generate verification key

Generate the VK for your ELF (needed for on-chain verification):

```bash
cargo-zisk rom-vkey \
    -e crates/guest-program/bin/zisk/out/riscv64ima-zisk-elf \
    -k ~/.zisk/provingKey \
    -o ~/.zisk/vk
```

This outputs the VK root hash:
```
INFO: Root hash: [3121973382251281428, 1947533496960916486, 15830689218699704550, 16339664693968653792]
```

Save this root hash - it's needed when verifying proofs on-chain.

### 9. Compile the verifier contract

```bash
cd <PATH_TO_ZISK_REPO>

solc --optimize --abi --bin \
    --base-path . --include-path zisk-contracts --allow-paths . \
    -o build/zisk-contracts --overwrite \
    zisk-contracts/ZiskVerifier.sol
```

### 10. Deploy the verifier contract

Deploy to L1 (or your target chain):

```bash
rex deploy --private-key <PRIVATE_KEY> \
    --bytecode `cat build/zisk-contracts/ZiskVerifier.bin` \
    --rpc-url <L1_RPC_URL>
```

Output:
```
Contract deployed in tx: 0x6e0f3ad8b0e837e835a9b1af83623c8865bef43a5cb111bb01889c8e2cc80d7a
Contract address: 0xa0c79e7f98c9914c337d5b010af208b98f23f117
```

Verify deployment:
```bash
rex call <CONTRACT_ADDRESS> "VERSION()" --rpc-url <L1_RPC_URL>
```

**Note:** Configure the deployed verifier address in your L2 sequencer config.

## Running the L2

After the ZisK setup is complete, follow these steps to run the L2 with ZisK proving.

### 11. Deploy L1 Contracts

Deploy the L1 contracts with ZisK verification enabled:

```bash
COMPILE_CONTRACTS=true \
ETHREX_L2_ZISK=true \
ETHREX_DEPLOYER_ZISK_VERIFIER_ADDRESS=<ZISK_VERIFIER_ADDRESS> \
ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true \
ethrex l2 deploy \
  --eth-rpc-url <ETH_RPC_URL> \
  --private-key <YOUR_PRIVATE_KEY> \
  --on-chain-proposer-owner <ON_CHAIN_PROPOSER_OWNER>  \
  --bridge-owner <BRIDGE_OWNER_ADDRESS>  \
  --genesis-l2-path fixtures/genesis/l2.json \
  --proof-sender.l1-address <PROOF_SENDER_L1_ADDRESS>
```

> [!NOTE]
> - `ETHREX_DEPLOYER_ZISK_VERIFIER_ADDRESS` is the verifier contract address from step 10
> - Save the deployed contract addresses for the next step

### 12. Start the L2 Node

```bash
ethrex l2 \
    --l1.bridge-address <BRIDGE_ADDRESS> \
    --l1.on-chain-proposer-address <ON_CHAIN_PROPOSER_ADDRESS> \
    --block-producer.coinbase-address <COINBASE_ADDRESS> \
    --committer.l1-private-key <COMMITTER_PRIVATE_KEY> \
    --proof-coordinator.l1-private-key <PROOF_SENDER_PRIVATE_KEY> \
    --eth.rpc-url <L1_RPC_URL> \
    --network fixtures/genesis/l2.json \
    --datadir ethrex_l2 \
    --no-monitor
```

> [!IMPORTANT]
> Both committer and proof coordinator accounts must be funded on L1.

### 13. Start the Prover

In a separate terminal, start the ZisK prover:

```bash
make -C crates/l2 init-prover-zisk GPU=true
```

> [!NOTE]
> The `GPU=true` flag is optional but recommended for faster STARK proof generation.

The prover automatically:
1. Receives batch input from the proof coordinator
2. Runs `cargo-zisk prove` (STARK proof on GPU)
3. Runs `cargo-zisk prove-snark` (SNARK proof on CPU)
4. Submits proof back to coordinator
5. Sequencer verifies on-chain

## ZisK Environment Variables (optional)

The prover uses sensible defaults, but you can override paths:

| Variable | Default | Description |
|----------|---------|-------------|
| `ZISK_HOME` | `~/.zisk` | ZisK home directory |
| `ZISK_ELF_PATH` | embedded | Path to guest ELF |
| `ZISK_PROVING_KEY_PATH` | `~/.zisk/provingKey` | STARK proving key |
| `ZISK_PROVING_KEY_SNARK_PATH` | `~/.zisk/provingKeySnark` | SNARK proving key |
| `ZISK_WITNESS_LIB_PATH` | auto-detected | Path to `libzisk_witness.so` |
| `ZISK_REPO_PATH` | - | ZisK repo (for witness lib) |

## Troubleshooting

### Executable stack error
```
Dynamic library error: ~/.zisk/provingKeySnark/final/final.so: cannot enable executable stack
```
Fix:
```bash
patchelf --clear-execstack ~/.zisk/provingKeySnark/final/final.so
```

### MerkleHash assertion error
```
Failed assert in template/function VerifyMerkleHash line 51
```
This is intermittent - the prover will retry automatically.

### Library path issues
```bash
export LD_LIBRARY_PATH=$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/lib
```

---

## Manual Proving (for debugging)

If you need to manually generate proofs (e.g., for debugging):

### Generate STARK proof
```bash
cargo-zisk prove \
    -e <PATH_TO_ELF> \
    -i <PATH_TO_INPUT> \
    -k ~/.zisk/provingKey \
    -w ~/.zisk/bin/libzisk_witness.so \
    -o <OUTPUT_PATH> \
    -a -u -f
```

### Generate SNARK proof
```bash
mkdir -p <OUTPUT_PATH>/proofs
cargo-zisk prove-snark \
    -k ~/.zisk/provingKeySnark \
    -p <OUTPUT_PATH>/vadcop_final_proof.bin \
    -o <OUTPUT_PATH>
```

This generates:
- `<OUTPUT_PATH>/proofs/final_snark_proof.hex`
- `<OUTPUT_PATH>/proofs/final_snark_publics.hex`

### Verify proof on-chain (manually)

```bash
rex call <VERIFIER_CONTRACT_ADDRESS> \
    "verifySnarkProof(uint64[4],bytes,bytes)" \
    "[<VK_ROOT_HASH>]" \
    `cat <OUTPUT_PATH>/proofs/final_snark_publics.hex` \
    `cat <OUTPUT_PATH>/proofs/final_snark_proof.hex` \
    --rpc-url <L1_RPC_URL>
```

Example:
```bash
rex call 0xa0c79e7f98c9914c337d5b010af208b98f23f117 \
    "verifySnarkProof(uint64[4],bytes,bytes)" \
    "[3121973382251281428,1947533496960916486,15830689218699704550,16339664693968653792]" \
    `cat zisk-output/proofs/final_snark_publics.hex` \
    `cat zisk-output/proofs/final_snark_proof.hex` \
    --rpc-url http://localhost:8545
```

Returns `0x` (empty bytes) on success.
