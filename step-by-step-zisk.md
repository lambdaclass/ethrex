# ZisK Proving Guide

This guide covers the **one-time setup** required to run ethrex L2 with the ZisK prover backend.

## Understanding ZisK Components

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

### Understanding the VK (Verification Key)

**There are TWO different "VK" concepts in ZisK:**

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        VK Confusion Clarified                                │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  1. PROGRAM VK (what we call "VK" in ethrex)                                │
│     ├── What: Merkle root hash of your compiled guest program's ROM         │
│     ├── Size: 32 bytes (4 x uint64 packed as big-endian)                    │
│     ├── File: crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk          │
│     ├── Created by: `cargo-zisk rom-setup` (as side effect) or              │
│     │               `cargo-zisk rom-vkey` (explicitly)                       │
│     ├── Used by: OnChainProposer contract for proof verification            │
│     └── Changes when: Guest program code changes                            │
│                                                                              │
│  2. SNARK VERIFICATION KEY (embedded in PlonkVerifier.sol)                  │
│     ├── What: Cryptographic constants for the PLONK verifier circuit        │
│     ├── Size: Many field elements (embedded in contract bytecode)           │
│     ├── File: Part of PlonkVerifier.sol in provingKeySnark/final/           │
│     ├── Created by: ZisK team (part of the trusted setup)                   │
│     ├── Used by: ZiskVerifier.sol for SNARK proof verification              │
│     └── Changes when: NEVER (it's part of the proving key)                  │
│                                                                              │
│  CRITICAL: These must be used together correctly:                           │
│  - The SNARK proof is verified against the SNARK VK (in PlonkVerifier)      │
│  - The PROGRAM VK is hashed with publicValues to create the SNARK input     │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### How On-Chain Verification Works

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    On-Chain Verification Flow                                │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  OnChainProposer.verify() receives:                                         │
│  ├── batchNumber                                                            │
│  ├── proof (768 bytes - the SNARK proof)                                    │
│  └── publicInputs (batch data)                                              │
│                                                                              │
│  Step 1: Get stored Program VK                                              │
│  ┌──────────────────────────────────────────────────────────────┐           │
│  │  bytes32 ziskVk = verificationKeys[commitHash][ZISK_ID];     │           │
│  │  uint64[4] programVk = _toZiskProgramVk(ziskVk);             │           │
│  │                                                              │           │
│  │  Conversion: bytes32 → uint64[4]                             │           │
│  │  vk[0] = uint64(word >> 192)  // first 8 bytes               │           │
│  │  vk[1] = uint64(word >> 128)  // next 8 bytes                │           │
│  │  vk[2] = uint64(word >> 64)   // next 8 bytes                │           │
│  │  vk[3] = uint64(word)         // last 8 bytes                │           │
│  └──────────────────────────────────────────────────────────────┘           │
│                                                                              │
│  Step 2: Build publicValues (256 bytes)                                     │
│  ┌──────────────────────────────────────────────────────────────┐           │
│  │  bytes32 outputHash = sha256(publicInputs);                  │           │
│  │  publicValues[0..3] = 0x00000008  // count = 8 u32s          │           │
│  │  publicValues[4..35] = outputHash // the sha256 hash         │           │
│  │  publicValues[36..255] = 0x00...  // padding                 │           │
│  └──────────────────────────────────────────────────────────────┘           │
│                                                                              │
│  Step 3: Call ZiskVerifier.verifySnarkProof()                               │
│  ┌──────────────────────────────────────────────────────────────┐           │
│  │  // Inside ZiskVerifier.sol:                                 │           │
│  │  digest = sha256(programVk || publicValues) % FIELD_MOD      │           │
│  │  PlonkVerifier.verifyProof(proof, [digest])                  │           │
│  └──────────────────────────────────────────────────────────────┘           │
│                                                                              │
│  The proof verifies that:                                                   │
│  - A program with merkle root = programVk                                   │
│  - Was executed and produced output = publicValues                          │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

### VK Byte Ordering

**This is critical and a common source of bugs:**

```
ROM Setup outputs root hash as 4 uint64 values:
  [17951655398561329467, 12219912878120779724, 11007752846151204199, 8887639580373120299]

These must be stored as 32 bytes in BIG-ENDIAN order:
  [0] = 17951655398561329467 = 0xf92117791978b13b → bytes: f9 21 17 79 19 78 b1 3b
  [1] = 12219912878120779724 = 0xa995da04ce9c3fcc → bytes: a9 95 da 04 ce 9c 3f cc
  [2] = 11007752846151204199 = 0x98c364e45a233d67 → bytes: 98 c3 64 e4 5a 23 3d 67
  [3] = 8887639580373120299  = 0x7b573d1c0fd7752b → bytes: 7b 57 3d 1c 0f d7 75 2b

Final VK file (32 bytes):
  f92117791978b13ba995da04ce9c3fcc98c364e45a233d677b573d1c0fd7752b

When contract reads this as bytes32 and does:
  uint64(word >> 192) = 0xf92117791978b13b = 17951655398561329467 ✓
```

---

## One-Time Setup

### 1. Build the Prover (includes ELF, ROM setup, and VK)

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

**Verify it worked:**
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

### 2. Install ZisK Tools (prerequisite for step 1)

**What this does:** Installs the ZisK command-line tools (`cargo-zisk`, `ziskemu`, etc.) that are used by the build process.

```bash
git clone git@github.com:0xPolygonHermez/zisk.git
cd zisk && git checkout pre-develop-0.16.0
cargo build --release
```

Follow the [ZisK installation guide](https://github.com/0xPolygonHermez/zisk/blob/feature/bn128/book/getting_started/installation.md#option-2-building-from-source) steps 3-7 to install binaries to `~/.zisk/bin`.

**Verify it worked:**
```bash
cargo-zisk --version
# Should show version info
```

### 3. Build GPU Binary (if using GPU)

**What this does:** Builds a separate GPU-accelerated binary for STARK proofs. You need TWO binaries because the GPU binary crashes during SNARK generation.

```bash
cd <PATH_TO_ZISK_REPO>
cargo build --release --features gpu
cp target/release/cargo-zisk ~/.zisk/bin/cargo-zisk-gpu
```

**Why two binaries?**
- `cargo-zisk` (CPU): Used for SNARK proofs (GPU version crashes)
- `cargo-zisk-gpu`: Used for STARK proofs (much faster)

### 4. Download Proving Keys

**What this does:** Downloads the trusted setup proving keys. These are large cryptographic parameters needed for proof generation.

```bash
# STARK proving key (~25GB)
wget https://storage.googleapis.com/zisk-setup/zisk-provingkey-pre-0.16.0.tar.gz

# SNARK proving key (~25GB)
wget https://storage.googleapis.com/zisk-setup/zisk-provingkey-pre-0.16.0-plonk.tar.gz
```

### 5. Extract Proving Keys

```bash
tar -xzf zisk-provingkey-pre-0.16.0.tar.gz -C ~/.zisk/
tar -xzf zisk-provingkey-pre-0.16.0-plonk.tar.gz -C ~/.zisk/
```

**Verify extraction:**
```bash
ls ~/.zisk/provingKey/
# Should contain many .bin and .json files

ls ~/.zisk/provingKeySnark/final/
# Should contain ZiskVerifier.sol, PlonkVerifier.sol, final.zkey, etc.
```

### 6. Compile Verifier Contract

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

**Verify compilation:**
```bash
ls ~/.zisk/provingKeySnark/final/build/
# Should contain ZiskVerifier.bin and ZiskVerifier.abi
```

### 7. Deploy Verifier Contract

```bash
rex deploy --private-key <PRIVATE_KEY> \
    --bytecode $(cat ~/.zisk/provingKeySnark/final/build/ZiskVerifier.bin) \
    --rpc-url <L1_RPC_URL>
```

**Save the deployed address!** You'll need it for L2 deployment.

**Verify deployment:**
```bash
rex call <CONTRACT_ADDRESS> "VERSION()" --rpc-url <L1_RPC_URL>
# Returns "v0.15.0" (version string is outdated but contract works)
```

---

## Manual ROM Setup and VK Generation (optional)

The prover build (step 1) runs these automatically, but you may need to run them manually for debugging or to regenerate with different options.

### Manual ROM Setup

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

### Manual VK Generation

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
# The bytes should be big-endian encoding of the root hash
```

**Convert root hash to expected VK bytes (for verification):**
```python
# Python script to verify VK matches root hash
root_hash = [17951655398561329467, 12219912878120779724, 11007752846151204199, 8887639580373120299]
import struct
vk = struct.pack(">QQQQ", *root_hash)  # Big-endian!
print(vk.hex())
# Compare with: xxd -p out/riscv64ima-zisk-vk | tr -d '\n'
```

---

## Verifying Your Setup

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

```bash
# Method 2: Verify byte order manually
xxd crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk

# First 8 bytes should be big-endian encoding of root_hash[0]
# Example: if root_hash[0] = 17951655398561329467 = 0xf92117791978b13b
# Then first 8 bytes should be: f9 21 17 79 19 78 b1 3b
```

### Check Contract Has Correct VK

After deploying contracts, verify the VK was stored correctly:

```bash
# Get the stored VK from contract (raw bytes32)
cast call <ON_CHAIN_PROPOSER_ADDRESS> \
    "debugGetRawVk(uint256)(bytes32)" 0 \
    --rpc-url <L1_RPC_URL>

# Compare with your VK file
xxd -p crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk | tr -d '\n'
# These should match!
```

**Important:** Also verify the converted VK matches (this is what the verifier actually uses):

```bash
# Get the converted VK from contract (uint64[4] array)
# Use batchNumber=0 to check the VK for the first batch
cast call <ON_CHAIN_PROPOSER_ADDRESS> \
    "debugGetConvertedVk(uint256)(uint64[4])" 0 \
    --rpc-url <L1_RPC_URL>

# Convert your local VK file to uint64 array and compare
VK_FILE=crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk
python3 -c "
import struct
data = open('$VK_FILE', 'rb').read()
values = struct.unpack('<4Q', data)  # 4 little-endian uint64s
print(f'[{values[0]},{values[1]},{values[2]},{values[3]}]')
"

# These MUST match! If they don't, the VK byte ordering is wrong.
```

### Check Public Values Match

After a proof is generated, verify the publicValues from the proof file match what the contract computes:

```bash
# Get the publicValues from the proof file (256 bytes)
PUBLICS_FILE=crates/l2/prover/zisk_output/snark_proof/final_snark_publics.bin
PROOF_PUBLICS=$(xxd -p $PUBLICS_FILE | tr -d '\n')
echo "Proof publicValues: 0x$PROOF_PUBLICS"

# Get the publicValues from contract (for batch 0)
# This computes: [4-byte count=8][32-byte sha256(publicInputs)][220-byte padding]
cast call <ON_CHAIN_PROPOSER_ADDRESS> \
    "debugGetFinalPublicValues(uint256)(bytes)" 0 \
    --rpc-url <L1_RPC_URL>

# These MUST match! If they don't, either:
# - The batch number is wrong (proof was for a different batch)
# - The publicInputs hash differs (different batch data)
```

**Step-by-step debugging if they don't match:**

```bash
# 1. Check the first 36 bytes (count + hash) - easier to compare
cast call <ON_CHAIN_PROPOSER_ADDRESS> \
    "debugGetPublicValuesPrefix(uint256)(bytes)" 0 \
    --rpc-url <L1_RPC_URL>

# Compare with proof file's first 36 bytes
xxd -p $PUBLICS_FILE | head -c 72  # 36 bytes = 72 hex chars

# 2. Check the sha256 hash separately
cast call <ON_CHAIN_PROPOSER_ADDRESS> \
    "debugGetPublicInputsHash(uint256)(bytes32)" 0 \
    --rpc-url <L1_RPC_URL>

# 3. Get the raw publicInputs that are hashed
cast call <ON_CHAIN_PROPOSER_ADDRESS> \
    "debugGetPublicInputs(uint256)(bytes)" 0 \
    --rpc-url <L1_RPC_URL>
```

**Understanding publicValues format:**
```
┌─────────────────────────────────────────────────────────────────────────────┐
│  ZisK publicValues (256 bytes)                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│  Bytes 0-3:   0x00000008 (count = 8 u32s = 32 bytes of actual output)      │
│  Bytes 4-35:  sha256(publicInputs) - the batch data hash                    │
│  Bytes 36-255: 0x00...00 (padding to 256 bytes)                            │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Test Manual Verification

After generating a proof, test it manually:

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

cast call <ZISK_VERIFIER_ADDRESS> \
    "verifySnarkProof(uint64[4],bytes,bytes)" \
    "$VK_ARRAY" \
    "0x$PUBLICS" \
    "0x$PROOF" \
    --rpc-url <L1_RPC_URL>

# Empty return (0x) = success
# 0x09bde339 = InvalidProof (VK or publicValues mismatch)
```

---

## Running the L2

### 8. Deploy L1 Contracts

```bash
COMPILE_CONTRACTS=true \
ETHREX_L2_ZISK=true \
ETHREX_DEPLOYER_ZISK_VERIFIER_ADDRESS=<VERIFIER_ADDRESS_FROM_STEP_7> \
ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true \
make -C crates/l2 deploy-l1
```

**What happens:** The deployer reads `crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk` and stores it in the OnChainProposer contract.

### 9. Start L2 Node

```bash
ZISK=true ETHREX_NO_MONITOR=true ETHREX_LOG_LEVEL=debug make -C crates/l2 init-l2 | grep -E "INFO|WARN|ERROR"
```

### 10. Start Prover

**With GPU:**
```bash
ZISK_STARK_BINARY=cargo-zisk-gpu make -C crates/l2 init-prover-zisk GPU=true
```

**CPU only:**
```bash
make -C crates/l2 init-prover-zisk
```

---

## Troubleshooting

### InvalidProof (0x09bde339)

**Cause:** VK mismatch between contract and proof.

**Debug steps:**
1. Check VK in contract matches VK file
2. Verify VK file was generated from the same ELF used for proving
3. Check byte ordering (must be big-endian)

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

### Executable stack error

```bash
patchelf --clear-execstack ~/.zisk/provingKeySnark/final/final.so
```

---

## Quick Reference

### File Locations Summary

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
# Build ELF
cargo-zisk build --release --features l2

# ROM setup (run after ELF changes)
cargo-zisk rom-setup -e <ELF_PATH> -k ~/.zisk/provingKey

# Generate VK file
cargo-zisk rom-vkey -e <ELF_PATH> -k ~/.zisk/provingKey -o <VK_OUTPUT_PATH>

# Check setup
cargo-zisk check-setup -k ~/.zisk/provingKey -a

# Manual STARK proof
cargo-zisk-gpu prove -e <ELF> -i <INPUT> -k ~/.zisk/provingKey -o <OUTPUT> -a -u -f

# Manual SNARK proof
cargo-zisk prove-snark -k ~/.zisk/provingKeySnark -p <STARK_PROOF> -o <OUTPUT>
```
