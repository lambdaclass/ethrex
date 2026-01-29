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

### 1. Build the ZisK Guest ELF

The guest program ELF must be built using the ZisK toolchain (`cargo-zisk`), not standard cargo:

```bash
cd crates/guest-program/bin/zisk
cargo-zisk build --release --features l2
```

This compiles the guest program to `target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk`.

Copy the ELF to the expected output location (for the deployer to find the VK):

```bash
mkdir -p out
cp target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk out/riscv64ima-zisk-elf
```

> [!NOTE]
> **Two ELF paths are used:**
> - `target/.../ethrex-guest-zisk` - Use this for ROM setup (step 7) and VK generation (step 9). The filename matters for cache file naming.
> - `out/riscv64ima-zisk-elf` - Copy destination for the deployer to find the VK file.

> [!IMPORTANT]
> **When to rebuild the ELF:**
> - After modifying any code in `crates/guest-program/`
> - After pulling changes that affect the guest program
> - The Makefile target `build-prover-zisk` does NOT rebuild the ELF - it only builds the prover binary

### 2. Build the Prover Binary

**With GPU acceleration (recommended):**
```bash
COMPILE_CONTRACTS=true make -C crates/l2 build-prover-zisk GPU=true
```

**CPU only:**
```bash
COMPILE_CONTRACTS=true make -C crates/l2 build-prover-zisk
```

The prover binary embeds the ELF from step 1. If you rebuild the ELF, you must also rebuild the prover binary to pick up the changes.

### 3. Clone and build ZisK

```bash
git clone git@github.com:0xPolygonHermez/zisk.git
cd zisk && git checkout pre-develop-0.16.0
```

```bash
cargo build --release
```

### 4. Install ZisK binaries

Follow steps 3 to 7 in the [ZisK installation guide](https://github.com/0xPolygonHermez/zisk/blob/feature/bn128/book/getting_started/installation.md#option-2-building-from-source).

This installs `cargo-zisk`, `ziskemu`, and `libzisk_witness.so` to `~/.zisk/bin`.

### 4b. Build and install GPU binary (if using GPU)

The GPU-built binary crashes during SNARK proof generation, so you need **two separate binaries**:
- **CPU binary** (`cargo-zisk`): Used for SNARK proofs - already installed in step 4
- **GPU binary** (`cargo-zisk-gpu`): Used for STARK proofs - build now

```bash
cd <PATH_TO_ZISK_REPO>
cargo build --release --features gpu
cp target/release/cargo-zisk ~/.zisk/bin/cargo-zisk-gpu
```

> [!NOTE]
> The default `cargo-zisk` (CPU) will be used for SNARK proofs. Set `ZISK_STARK_BINARY=cargo-zisk-gpu` to use GPU acceleration for STARK proofs.

### 5. Download proving keys

```bash
# STARK proving key
wget https://storage.googleapis.com/zisk-setup/zisk-provingkey-pre-0.16.0.tar.gz

# SNARK proving key
wget https://storage.googleapis.com/zisk-setup/zisk-provingkey-pre-0.16.0-plonk.tar.gz
```

### 6. Extract proving keys

```bash
tar -xzf zisk-provingkey-pre-0.16.0.tar.gz -C ~/.zisk/
tar -xzf zisk-provingkey-pre-0.16.0-plonk.tar.gz -C ~/.zisk/
```

After extraction you should have:
- `~/.zisk/provingKey/` - STARK proving key
- `~/.zisk/provingKeySnark/` - SNARK proving key

### 7. ROM setup (once per ELF)

This step must be run whenever the guest program ELF changes:

```bash
cargo-zisk rom-setup -e <PATH_TO_ELF> -k ~/.zisk/provingKey
```

For ethrex, use the **cargo output path** (not the copied `out/` path):
```bash
cargo-zisk rom-setup \
    -e crates/guest-program/bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk \
    -k ~/.zisk/provingKey
```

> [!IMPORTANT]
> **ELF path matters for cache file naming!** ROM setup creates cache files named `<elf-filename>-<hash>-*.bin`. The prover looks for files with prefix `ethrex-guest-zisk-<hash>`. If you run ROM setup with a different path (e.g., `out/riscv64ima-zisk-elf`), the cache files will have the wrong prefix and the prover will fail with "Path does not exist" errors.

### 8. Verify setup (optional)

```bash
cargo-zisk check-setup -k ~/.zisk/provingKey -a
```

### 9. Generate verification key

Generate the VK for your ELF (needed for on-chain verification):

```bash
cargo-zisk rom-vkey \
    -e crates/guest-program/bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk \
    -k ~/.zisk/provingKey \
    -o crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk
```

This outputs the VK root hash and saves it to the path where the deployer expects it.

### 10. Compile the verifier contract

> [!CAUTION]
> **CRITICAL**: The verifier contract MUST come from the SNARK proving key folder (`~/.zisk/provingKeySnark/final/`), NOT from the ZisK repo. The PlonkVerifier.sol contains verification key constants that must match the proving key used to generate proofs. Using the wrong verifier will cause all proof verifications to fail.

The SNARK proving key tarball includes the matching verifier contracts:
- `~/.zisk/provingKeySnark/final/ZiskVerifier.sol`
- `~/.zisk/provingKeySnark/final/PlonkVerifier.sol`
- `~/.zisk/provingKeySnark/final/IZiskVerifier.sol`

Compile using solc:

```bash
cd ~/.zisk/provingKeySnark/final
solc --optimize --abi --bin \
    --base-path . --allow-paths . \
    -o build --overwrite \
    ZiskVerifier.sol
```

### 11. Deploy the verifier contract

Deploy to L1 (or your target chain):

```bash
rex deploy --private-key <PRIVATE_KEY> \
    --bytecode $(cat ~/.zisk/provingKeySnark/final/build/ZiskVerifier.bin) \
    --rpc-url <L1_RPC_URL>
```

Output:
```
Contract deployed at: 0x937bC1A524f22dE858203b2cbBAD073A98FfF0B5
```

Verify deployment:
```bash
rex call <CONTRACT_ADDRESS> "VERSION()" --rpc-url <L1_RPC_URL>
```

> [!NOTE]
> The contract returns `"v0.15.0"` even when using the `pre-develop-0.16.0` proving keys. This is expected - the version string in the contract template hasn't been updated by the ZisK team. What matters is that the PlonkVerifier verification key constants match the proving key, not the version string.

**Note:** Configure the deployed verifier address in your L2 sequencer config.

## Running the L2

After the ZisK setup is complete, follow these steps to run the L2 with ZisK proving.

### 12. Verify VK matches ELF (CRITICAL - DO NOT SKIP)

> [!CAUTION]
> **MANDATORY CHECK BEFORE DEPLOYING.** A VK mismatch causes `InvalidProof` errors (0x09bde339) that require full redeployment. You will waste 15+ minutes per proof attempt if this is wrong.

**The VK must be regenerated from the PROVER'S EMBEDDED ELF, not the source ELF.**

The prover binary writes its embedded ELF to `crates/l2/prover/ethrex-guest-zisk`. This is the ELF that actually gets used for proving. Always generate the VK from THIS file:

```bash
# Step 1: Ensure the prover has written its embedded ELF
# (Run the prover once briefly, or check if the file exists)
ls -la crates/l2/prover/ethrex-guest-zisk

# Step 2: Generate VK from the PROVER'S ELF (not the source ELF!)
cargo-zisk rom-vkey \
    -e crates/l2/prover/ethrex-guest-zisk \
    -k ~/.zisk/provingKey \
    -o /tmp/correct-vk

# Step 3: Compare with existing VK file
if ! diff -q /tmp/correct-vk crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk > /dev/null 2>&1; then
    echo "VK MISMATCH! Updating VK file..."
    cp /tmp/correct-vk crates/guest-program/bin/zisk/out/riscv64ima-zisk-vk
    echo "VK updated. You MUST redeploy contracts."
else
    echo "VK matches. Safe to proceed."
fi
```

> [!WARNING]
> **Common mistake:** Running `rom-vkey` on `crates/guest-program/bin/zisk/target/.../ethrex-guest-zisk` instead of `crates/l2/prover/ethrex-guest-zisk`. Even if the ELF hashes are identical, the cache file naming can cause different VK outputs. Always use the prover's ELF path.

> [!IMPORTANT]
> **Guest program byte ordering:** The guest program outputs SHA256 hash via `ziskos::set_output()`. ZisK internally writes u32 values as little-endian bytes. To preserve the original SHA256 bytes in public values, the guest MUST use `from_le_bytes` (same as aligned_layer):
> ```rust
> // CORRECT: sha256[A,B,C,D] → from_le_bytes → u32(0xDCBA) → ZisK writes LE → [A,B,C,D]
> output.chunks_exact(4).for_each(|(idx, bytes)| {
>     ziskos::set_output(idx, u32::from_le_bytes(bytes.try_into().unwrap()))
> });
> ```
> Using `from_be_bytes` causes bytes to be reversed within each 4-byte chunk, which breaks verification.

### 13. Deploy L1 Contracts

Deploy the L1 contracts with ZisK verification enabled:

```bash
COMPILE_CONTRACTS=true \
ETHREX_L2_ZISK=true \
ETHREX_DEPLOYER_ZISK_VERIFIER_ADDRESS=0x8b12ca58bb4a5cf859bf0d6a17384729e978587d \
ETHREX_DEPLOYER_RANDOMIZE_CONTRACT_DEPLOYMENT=true \
make -C crates/l2 deploy-l1
```

> [!NOTE]
> - `ETHREX_DEPLOYER_ZISK_VERIFIER_ADDRESS` is the verifier contract address from step 11
> - Save the deployed contract addresses for the next step

### 14. Start the L2 Node

```bash
ZISK=true ETHREX_NO_MONITOR=true ETHREX_LOG_LEVEL=debug make -C crates/l2 init-l2 | grep -E "INFO|WARN|ERROR"
```

> [!IMPORTANT]
> Both committer and proof coordinator accounts must be funded on L1.

### 15. Start the Prover

In a separate terminal, start the ZisK prover:

**With GPU acceleration (recommended):**
```bash
ZISK_STARK_BINARY=cargo-zisk-gpu make -C crates/l2 init-prover-zisk GPU=true
```

**CPU only:**
```bash
make -C crates/l2 init-prover-zisk
```

> [!NOTE]
> `ZISK_SNARK_BINARY` defaults to `cargo-zisk` (CPU version), which is correct. Only `ZISK_STARK_BINARY` needs to be set for GPU acceleration.

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

### Missing SNARK proving key
```
Failed to read file /home/admin/.zisk/provingKeySnark/recursivef/recursivef.starkinfo.json
```
The SNARK proving key is missing. Download and extract it:
```bash
wget https://storage.googleapis.com/zisk-setup/zisk-provingkey-pre-0.16.0-plonk.tar.gz
tar -xzf zisk-provingkey-pre-0.16.0-plonk.tar.gz -C ~/.zisk/
```

### Library path issues
```bash
export LD_LIBRARY_PATH=$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/lib
```

### Cache file not found (Path does not exist)
```
Path does not exist: ~/.zisk/provingKey/rom/ethrex-guest-zisk-68cb2244...-mt.bin
```
This happens when ROM setup was run with a different ELF path than expected. The prover looks for cache files with prefix `ethrex-guest-zisk-<hash>`, but ROM setup creates files named after the ELF filename.

**Fix:** Re-run ROM setup with the correct ELF path:
```bash
cargo-zisk rom-setup \
    -e crates/guest-program/bin/zisk/target/riscv64ima-zisk-zkvm-elf/release/ethrex-guest-zisk \
    -k ~/.zisk/provingKey
```

### ELF hash mismatch after code changes
If you modified guest program code but the prover still uses old cache files:
1. Rebuild the ELF: `cd crates/guest-program/bin/zisk && cargo-zisk build --release --features l2`
2. Re-run ROM setup (step 7)
3. Regenerate VK (step 9)
4. Rebuild prover binary (step 2) - it embeds the ELF

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
