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

ZisK requires **two separate binaries** for proof generation:
- **GPU binary**: Used for STARK proof generation (fast)
- **CPU binary**: Used for SNARK proof generation (cannot use GPU binary due to segfault)

```bash
git clone git@github.com:0xPolygonHermez/zisk.git
cd zisk && git checkout pre-develop-0.16.0
```

**Build both binaries:**
```bash
# Build GPU binary for STARK proof generation
cargo build --release --features gpu
cp target/release/cargo-zisk ~/.zisk/bin/cargo-zisk-gpu

# Build CPU binary for SNARK proof generation
cargo build --release
cp target/release/cargo-zisk ~/.zisk/bin/cargo-zisk-cpu
```

**CPU only (if no GPU available):**
```bash
cargo build --release
# Only one binary needed when running CPU-only
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
    -o crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk
```

This outputs the VK root hash and saves it to the path where the deployer expects it.

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
ETHREX_DEPLOYER_ZISK_VERIFIER_ADDRESS=0xd5cf1b40771142c801c9f522d27721ded4d8ef0d \
ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true \
make -C crates/l2 deploy-l1
```

> [!NOTE]
> - `ETHREX_DEPLOYER_ZISK_VERIFIER_ADDRESS` is the verifier contract address from step 10
> - Save the deployed contract addresses for the next step

### 12. Start the L2 Node

```bash
ZISK=true ETHREX_NO_MONITOR=true ETHREX_LOG_LEVEL=debug make -C crates/l2 init-l2 | grep -E "INFO|WARN|ERROR"
```

> [!IMPORTANT]
> Both committer and proof coordinator accounts must be funded on L1.

### 13. Start the Prover

In a separate terminal, start the ZisK prover:

**With GPU acceleration (recommended):**
```bash
ZISK_STARK_BINARY=cargo-zisk-gpu ZISK_SNARK_BINARY=cargo-zisk-cpu \
    make -C crates/l2 init-prover-zisk GPU=true
```

**CPU only:**
```bash
make -C crates/l2 init-prover-zisk
```

> [!IMPORTANT]
> When using GPU, you must set `ZISK_STARK_BINARY` and `ZISK_SNARK_BINARY` to point to the GPU and CPU binaries respectively. The SNARK proof generation crashes with a segfault when using the GPU binary.

The prover automatically:
1. Receives batch input from the proof coordinator
2. Runs STARK proof generation (GPU binary if configured)
3. Runs SNARK proof generation (must use CPU binary)
4. Submits proof back to coordinator
5. Sequencer verifies on-chain

## ZisK Environment Variables (optional)

The prover uses sensible defaults, but you can override paths and binaries:

| Variable | Default | Description |
|----------|---------|-------------|
| `ZISK_HOME` | `~/.zisk` | ZisK home directory |
| `ZISK_ELF_PATH` | embedded | Path to guest ELF |
| `ZISK_PROVING_KEY_PATH` | `~/.zisk/provingKey` | STARK proving key |
| `ZISK_PROVING_KEY_SNARK_PATH` | `~/.zisk/provingKeySnark` | SNARK proving key |
| `ZISK_STARK_BINARY` | `cargo-zisk` | Binary for STARK proof (use GPU version) |
| `ZISK_SNARK_BINARY` | `cargo-zisk` | Binary for SNARK proof (must be CPU version) |

## Troubleshooting

### Executable stack error
```
Dynamic library error: ~/.zisk/provingKeySnark/final/final.so: cannot enable executable stack
```
Fix:
```bash
patchelf --clear-execstack ~/.zisk/provingKeySnark/final/final.so
```

### SNARK proof segfault
```
Segmentation fault (core dumped) cargo-zisk prove-snark ...
```
The GPU-built binary crashes during SNARK generation. Use a CPU-built binary for SNARK proofs:
```bash
export ZISK_SNARK_BINARY=cargo-zisk-cpu
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
Use the GPU binary for faster STARK proof generation:
```bash
cargo-zisk-gpu prove \
    -e <PATH_TO_ELF> \
    -i <PATH_TO_INPUT> \
    -k ~/.zisk/provingKey \
    -o <OUTPUT_PATH> \
    -a -u -f
```

### Generate SNARK proof
Use the CPU binary (GPU binary will segfault):
```bash
mkdir -p <OUTPUT_PATH>/proofs
cargo-zisk-cpu prove-snark \
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
