# ZisK Proving Guide

This guide covers setup and operation of ethrex L2 with the ZisK prover backend.

---

## Prerequisites

These must be completed before building the prover.

### 1. Install ZisK Tools

**What this does:** Installs the ZisK command-line tools (`cargo-zisk`, `ziskemu`, etc.) used by the build process.

```bash
git clone git@github.com:0xPolygonHermez/zisk.git
cd zisk && git checkout pre-develop-0.16.0
cargo build --release
```

Follow the [ZisK installation guide](https://github.com/0xPolygonHermez/zisk/blob/pre-develop-0.16.0/book/getting_started/installation.md#option-2-building-from-source) steps 3-7 to install binaries to `~/.zisk/bin`.

**Verify:**
```bash
cargo-zisk --version
```

### 2. Build GPU Binary (optional, for GPU acceleration)

**What this does:** Builds a separate GPU-accelerated binary for STARK proofs. You need TWO binaries because the GPU binary crashes during SNARK generation.

```bash
cd <PATH_TO_ZISK_REPO>
cargo build --release --features gpu
cp target/release/cargo-zisk ~/.zisk/bin/cargo-zisk-gpu
```

**Why two binaries?**
- `cargo-zisk` (CPU): Used for SNARK proofs (GPU version crashes)
- `cargo-zisk-gpu`: Used for STARK proofs (much faster)

### 3. Download Proving Keys

**What this does:** Downloads the trusted setup proving keys. These are large cryptographic parameters needed for proof generation.

```bash
# STARK proving key (~25GB)
wget https://storage.googleapis.com/zisk-setup/zisk-provingkey-pre-0.16.0.tar.gz

# SNARK proving key (~25GB)
wget https://storage.googleapis.com/zisk-setup/zisk-provingkey-pre-0.16.0-plonk.tar.gz
```

### 4. Extract Proving Keys

```bash
tar -xzf zisk-provingkey-pre-0.16.0.tar.gz -C ~/.zisk/
tar -xzf zisk-provingkey-pre-0.16.0-plonk.tar.gz -C ~/.zisk/
```

**Verify:**
```bash
ls ~/.zisk/provingKey/
# Should contain many .bin and .json files

ls ~/.zisk/provingKeySnark/final/
# Should contain ZiskVerifier.sol, PlonkVerifier.sol, final.zkey, etc.
```

---

## Understanding ZisK Components

This section explains how ZisK proving works. Read it for reference when debugging.

### Proof Flow Overview

ZisK uses a two-stage proving system:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           ZisK Proof Generation                              │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│   ┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐  │
│   │   Guest     │    │   STARK     │    │   SNARK     │    │  On-chain   │  │
│   │  Program    │───>│   Proof     │───>│   Proof     │───>│ Verification│  │
│   │ Execution   │    │  (GPU/CPU)  │    │   (CPU)     │    │             │  │
│   └─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘  │
│         │                  │                  │                   │          │
│         ▼                  ▼                  ▼                   ▼          │
│   Uses: ELF         Uses: provingKey   Uses: provingKeySnark  Uses: VK      │
│                     Creates: STARK      Creates: SNARK         (Program VK) │
│                     proof               proof + publics                      │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Key Files and Their Purpose

| File | What It Is | Where It Lives | When It's Created |
|------|------------|----------------|-------------------|
| **Guest ELF** | Compiled guest program (RISC-V binary) | `crates/guest-program/bin/zisk/out/` | `cargo-zisk build` |
| **Program VK** | Merkle root hash of the ROM (32 bytes) | `crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk` | `cargo-zisk rom-setup` or `cargo-zisk rom-vkey` |
| **STARK Proving Key** | Large key for STARK proof generation | `~/.zisk/provingKey/` | Downloaded (25GB+) |
| **SNARK Proving Key** | Key for SNARK wrapping | `~/.zisk/provingKeySnark/` | Downloaded |
| **ZiskVerifier.sol** | On-chain verifier contract | `~/.zisk/provingKeySnark/final/` | Part of SNARK proving key tarball |
| **ROM Cache Files** | Pre-computed ROM data for proving | `~/.zisk/cache/` | `cargo-zisk rom-setup` |


---

## Build and Deploy

### 1. Build the Prover

**What this does:** The prover build command does EVERYTHING automatically:
1. Builds the guest program ELF using the ZisK toolchain
2. Runs ROM setup (creates cache files in `~/.zisk/cache/`)
3. Generates the VK file
4. Builds the prover binary

**With GPU acceleration (recommended):**
```bash
COMPILE_CONTRACTS=true make -C crates/l2 build-prover-zisk GPU=true
```

**CPU only:**
```bash
COMPILE_CONTRACTS=true make -C crates/l2 build-prover-zisk
```

**What gets created:**
- `crates/guest-program/bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk` - Guest ELF
- `crates/guest-program/bin/zisk/out/riscv64ima-zisk-elf` - Copy of ELF
- `crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk` - VK file (32 bytes)
- `~/.zisk/cache/ethrex-guest-zisk-<hash>-*.bin` - ROM cache files
- `target/release/ethrex` - Prover binary

**Verify:**
```bash
# Check ELF was built
file crates/guest-program/bin/zisk/out/riscv64ima-zisk-elf
# Should show: ELF 64-bit LSB executable, UCB RISC-V

# Check VK was generated (should be 32 bytes)
ls -la crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk
xxd crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk

# Check cache files were created
ls ~/.zisk/cache/ | grep ethrex-guest-zisk
```

> [!NOTE]
> The build.rs in `crates/guest-program` automatically runs `cargo-zisk rom-setup` and `cargo-zisk rom-vkey`.
> You only need to run these manually if you want to regenerate with different options.

> [!IMPORTANT]
> **When to rebuild:**
> - After modifying any code in `crates/guest-program/`
> - After pulling changes that affect the guest program
> - The build will automatically regenerate the ELF, run ROM setup, and update the VK

### 2. Compile Verifier Contract

**What this does:** Compiles the ZiskVerifier.sol contract that will verify proofs on-chain.

> [!CAUTION]
> **Use the contract from provingKeySnark, NOT from the ZisK repo!** The PlonkVerifier.sol contains verification key constants that must match your proving key.

```bash
cd ~/.zisk/provingKeySnark/final
solc --optimize --abi --bin \
    --base-path . --allow-paths . \
    -o build --overwrite \
    ZiskVerifier.sol
```

**Verify:**
```bash
ls ~/.zisk/provingKeySnark/final/build/
# Should contain ZiskVerifier.bin and ZiskVerifier.abi
```

### 3. Deploy Verifier Contract

```bash
rex deploy --private-key <PRIVATE_KEY> \
    --bytecode $(cat ~/.zisk/provingKeySnark/final/build/ZiskVerifier.bin) \
    --rpc-url <L1_RPC_URL>
```

**Save the deployed address!** You'll need it for L2 deployment.

**Verify:**
```bash
rex call <CONTRACT_ADDRESS> "VERSION()" --rpc-url <L1_RPC_URL>
# Returns "v0.15.0" (version string is outdated but contract works)
```

### 4. Deploy L1 Contracts

```bash
COMPILE_CONTRACTS=true \
ETHREX_L2_ZISK=true \
ETHREX_DEPLOYER_ZISK_VERIFIER_ADDRESS=<VERIFIER_ADDRESS_FROM_STEP_3> \
ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true \
make -C crates/l2 deploy-l1
```

**What happens:** The deployer reads `crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk` and stores it in the OnChainProposer contract.

---

## Running the L2

### Start L2 Node

```bash
ZISK=true ETHREX_NO_MONITOR=true ETHREX_LOG_LEVEL=debug make -C crates/l2 init-l2 | grep -E "INFO|WARN|ERROR"
```

### Start Prover

**With GPU:**
```bash
ZISK_STARK_BINARY=cargo-zisk-gpu make -C crates/l2 init-prover-zisk GPU=true
```

**CPU only:**
```bash
make -C crates/l2 init-prover-zisk
```

---

## Verification and Debugging

Use these procedures to verify your setup is correct and debug issues.

### Check VK Matches ELF

Before deploying contracts, verify the VK file matches your ELF:

```bash
# Method 1: Re-run rom-vkey and compare
cargo-zisk rom-vkey \
    -e crates/guest-program/bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk \
    -k ~/.zisk/provingKey \
    -o /tmp/check-vk

diff crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk /tmp/check-vk
# No output = files match
```

### Check Contract Has Correct VK

After deploying contracts, verify the VK was stored correctly:

```bash
# Get the converted VK from contract (uint64[4] array)
rex call <ON_CHAIN_PROPOSER_ADDRESS> \
    "getZiskVk(uint256)(uint64[4])" 1 \
    --rpc-url <L1_RPC_URL>

# Convert your local VK file to uint64 array and compare
VK_FILE=crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk
python3 -c "
import struct
data = open('$VK_FILE', 'rb').read()
values = struct.unpack('<4Q', data)  # 4 little-endian uint64s
print(f'[{values[0]},{values[1]},{values[2]},{values[3]}]')
"

# These MUST match!
```

### Check Public Values Match

After a proof is generated, inspect the publicValues from the proof file:

```bash
# Get the publicValues from the proof file (256 bytes)
PUBLICS_FILE=crates/l2/prover/zisk_output/snark_proof/final_snark_publics.bin
xxd $PUBLICS_FILE | head -3

# Expected format:
# - Bytes 0-3:   0x00000008 (count = 8 u32s)
# - Bytes 4-35:  sha256(publicInputs) with bytes swapped per 4-byte word
# - Bytes 36-255: zero padding
```

The publicValues in the proof must match what `OnChainProposer.buildZiskPublicValues()` computes from the batch commitment data during `verifyBatch()`.

### Test Manual Verification

After generating a proof, test it directly against the verifier contract:

```bash
# Get proof files
ls crates/l2/prover/zisk_output/snark_proof/
# final_snark_proof.bin (768 bytes)
# final_snark_publics.bin (256 bytes)

# Convert VK file to uint64 array (VK is stored in little-endian)
VK_FILE=crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk
VK_ARRAY=$(python3 -c "
import struct
data = open('$VK_FILE', 'rb').read()
values = struct.unpack('<4Q', data)  # 4 little-endian uint64s
print(f'[{values[0]},{values[1]},{values[2]},{values[3]}]')
")
echo "VK array: $VK_ARRAY"

# Call verifier directly
PROOF=$(xxd -p crates/l2/prover/zisk_output/snark_proof/final_snark_proof.bin | tr -d '\n')
PUBLICS=$(xxd -p crates/l2/prover/zisk_output/snark_proof/final_snark_publics.bin | tr -d '\n')

rex call <ZISK_VERIFIER_ADDRESS> \
    "verifySnarkProof(uint64[4],bytes,bytes)" \
    "$VK_ARRAY" \
    "0x$PUBLICS" \
    "0x$PROOF" \
    --rpc-url <L1_RPC_URL>

# Empty return (0x) = success
# 0x09bde339 = InvalidProof (VK or publicValues mismatch)
```

### Public Helper Functions

The OnChainProposer contract exposes these helper functions used by `verify()`:

| Function | Description |
|----------|-------------|
| `getZiskVk(uint256)` | Get converted uint64[4] VK for a batch |
| `buildZiskPublicValues(bytes)` | Build 256-byte publicValues from publicInputs |
| `toZiskProgramVk(bytes32)` | Convert bytes32 to uint64[4] |
| `swapBytes64(uint64)` | Reverse byte order in a uint64 |
| `swapHashBytes(bytes32)` | Swap bytes within each 4-byte word |

**Understanding publicValues format:**
```
┌─────────────────────────────────────────────────────────────────────────────┐
│  ZisK publicValues (256 bytes)                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│  Bytes 0-3:   0x00000008 (count = 8 u32s = 32 bytes of actual output)      │
│  Bytes 4-35:  sha256(publicInputs) with bytes swapped per 4-byte word      │
│  Bytes 36-255: 0x00...00 (padding to 256 bytes)                            │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Troubleshooting

### ZisK Proof Verification Failed

**Error codes:**
- `017` - From OnChainProposer when `verifyBatch()` fails (seen in proof sender logs)
- `0x09bde339` - InvalidProof from ZiskVerifier directly (seen in manual verification)

**Cause:** VK mismatch or publicValues mismatch between contract and proof.

**Debug steps:**
1. Check VK in contract matches VK file (see "Check Contract Has Correct VK")
2. Verify VK file was generated from the same ELF used for proving
3. Check publicValues match (see "Check Public Values Match")

### "Path does not exist" for cache files

**Cause:** ROM setup was run with wrong ELF filename.

**Fix:** Re-run ROM setup with correct path:
```bash
cargo-zisk rom-setup \
    -e crates/guest-program/bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk \
    -k ~/.zisk/provingKey
```

### SNARK proof segfault

**Cause:** Using GPU binary for SNARK proof.

**Fix:** Ensure `ZISK_SNARK_BINARY=cargo-zisk` (CPU version).

---

## Quick Reference

### File Locations

| Purpose | Location |
|---------|----------|
| Guest ELF (source) | `crates/guest-program/bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk` |
| Guest ELF (for deployer) | `crates/guest-program/bin/zisk/out/riscv64ima-zisk-elf` |
| Program VK file | `crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk` |
| ROM cache files | `~/.zisk/cache/ethrex-guest-zisk-<hash>-*.bin` |
| STARK proving key | `~/.zisk/provingKey/` |
| SNARK proving key | `~/.zisk/provingKeySnark/` |
| Verifier contracts | `~/.zisk/provingKeySnark/final/ZiskVerifier.sol` |
| Generated proofs | `crates/l2/prover/zisk_output/snark_proof/` |

### Command Cheatsheet

```bash
# Build everything (ELF, ROM setup, VK, prover binary)
COMPILE_CONTRACTS=true make -C crates/l2 build-prover-zisk GPU=true

# Manual: Build ELF only
cargo-zisk build --release --features l2

# Manual: ROM setup (run after ELF changes)
cargo-zisk rom-setup -e <ELF_PATH> -k ~/.zisk/provingKey

# Manual: Generate VK file
cargo-zisk rom-vkey -e <ELF_PATH> -k ~/.zisk/provingKey -o <VK_OUTPUT_PATH>

# Check setup
cargo-zisk check-setup -k ~/.zisk/provingKey -a

# Manual STARK proof
cargo-zisk-gpu prove -e <ELF> -i <INPUT> -k ~/.zisk/provingKey -o <OUTPUT> -a -u -f

# Manual SNARK proof
cargo-zisk prove-snark -k ~/.zisk/provingKeySnark -p <STARK_PROOF> -o <OUTPUT>

# Deploy verifier contract
rex deploy --private-key <KEY> --bytecode $(cat ~/.zisk/provingKeySnark/final/build/ZiskVerifier.bin) --rpc-url <URL>

# Deploy L1 contracts with ZisK
COMPILE_CONTRACTS=true ETHREX_L2_ZISK=true ETHREX_DEPLOYER_ZISK_VERIFIER_ADDRESS=<ADDR> make -C crates/l2 deploy-l1
```

---

## Advanced: Manual ROM Setup

The prover build runs these automatically, but you may need them for debugging.

### ROM Setup

**What this does:** Pre-computes ROM data for your guest program:
- Converts the ELF to ZisK's internal format
- Computes the merkle tree of the ROM
- Generates cache files for faster proving
- **Outputs the root hash** (this becomes your Program VK!)

```bash
cargo-zisk rom-setup \
    -e crates/guest-program/bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk \
    -k ~/.zisk/provingKey
```

**Expected output:**
```
Computing setup for ROM ...
Computing ELF hash
Computing merkle root
Computing custom trace ROM
Root hash: [17951655398561329467, 12219912878120779724, 11007752846151204199, 8887639580373120299]
ROM setup successfully completed at /home/user/.zisk/cache
```

**The Root Hash IS your Program VK!** These 4 uint64 values identify your specific guest program.

> [!IMPORTANT]
> **ELF filename matters!** Cache files are named after the ELF filename. Use `ethrex-guest-zisk` (the cargo output name), not `riscv64ima-zisk-elf`.

### VK File Generation

**What this does:** Creates the 32-byte VK file that the deployer reads when initializing contracts.

```bash
cargo-zisk rom-vkey \
    -e crates/guest-program/bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk \
    -k ~/.zisk/provingKey \
    -o crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk
```

**Verify the VK file:**
```bash
xxd crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk
# Should show 32 bytes (2 lines of 16 bytes each)
```

**Convert root hash to VK bytes (for verification):**
```python
# Python script to verify VK matches root hash
root_hash = [17951655398561329467, 12219912878120779724, 11007752846151204199, 8887639580373120299]
import struct
vk = struct.pack("<QQQQ", *root_hash)  # Little-endian!
print(vk.hex())
# Compare with: xxd -p out/riscv64ima-zisk-vk | tr -d '\n'
```
