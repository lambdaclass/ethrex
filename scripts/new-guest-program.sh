#!/usr/bin/env bash
set -euo pipefail

# Guest Program Scaffold Generator
# Usage: ./scripts/new-guest-program.sh <program-name>
# Example: ./scripts/new-guest-program.sh my-program
#
# Creates:
#   crates/guest-program/src/programs/<name>/
#     ├── mod.rs       — GuestProgram trait impl + tests
#     ├── types.rs     — Input/output types with rkyv + serde
#     └── execution.rs — Execution logic skeleton
#   crates/guest-program/bin/sp1-<name>/
#     ├── Cargo.toml   — SP1 guest binary config
#     └── src/main.rs  — SP1 zkVM entry point

if [ $# -lt 1 ]; then
    echo "Usage: $0 <program-name>"
    echo "Example: $0 my-custom-program"
    echo ""
    echo "This creates a full guest program module with:"
    echo "  - Custom input/output types (rkyv + serde)"
    echo "  - Execution logic skeleton with keccak state transitions"
    echo "  - GuestProgram trait implementation"
    echo "  - SP1 zkVM entry point binary"
    echo "  - Unit tests"
    exit 1
fi

PROGRAM_NAME="$1"
# Validate: lowercase, numbers, hyphens only
if ! echo "$PROGRAM_NAME" | grep -qE '^[a-z][a-z0-9-]{1,62}[a-z0-9]$'; then
    echo "Error: program name must be 3-64 chars, lowercase letters, numbers, and hyphens"
    exit 1
fi

# Convert to Rust identifiers (macOS-compatible)
SNAKE_NAME=$(echo "$PROGRAM_NAME" | tr '-' '_')
# PascalCase: split on hyphens, capitalize first letter of each part
PASCAL_NAME=$(echo "$PROGRAM_NAME" | awk -F'-' '{for(i=1;i<=NF;i++){$i=toupper(substr($i,1,1)) substr($i,2)}}1' OFS='')
# UPPER_SNAKE for ELF constant names
UPPER_SNAKE=$(echo "$SNAKE_NAME" | tr '[:lower:]' '[:upper:]')

PROGRAMS_DIR="crates/guest-program/src/programs"
BIN_DIR="crates/guest-program/bin"
MOD_FILE="$PROGRAMS_DIR/mod.rs"

# Check we're in repo root
if [ ! -f "$MOD_FILE" ]; then
    echo "Error: must be run from the ethrex repository root"
    echo "Expected: $MOD_FILE"
    exit 1
fi

# Check for duplicates
if [ -d "$PROGRAMS_DIR/${SNAKE_NAME}" ] || [ -f "$PROGRAMS_DIR/${SNAKE_NAME}.rs" ]; then
    echo "Error: $PROGRAMS_DIR/${SNAKE_NAME} already exists"
    exit 1
fi

# Determine next available type ID
NEXT_TYPE_ID=$(grep -rh 'fn program_type_id' "$PROGRAMS_DIR"/ 2>/dev/null \
    | grep -oE '[0-9]+' | sort -n | tail -1)
NEXT_TYPE_ID=$((NEXT_TYPE_ID + 1))

echo "Creating guest program: $PROGRAM_NAME"
echo "  Struct:  ${PASCAL_NAME}GuestProgram"
echo "  Module:  $SNAKE_NAME"
echo "  Type ID: $NEXT_TYPE_ID"
echo ""

# ── Create module directory ──────────────────────────────────────────

mkdir -p "$PROGRAMS_DIR/${SNAKE_NAME}"

# ── types.rs ─────────────────────────────────────────────────────────

cat > "$PROGRAMS_DIR/${SNAKE_NAME}/types.rs" << RUST_EOF
use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};

/// Input for the ${PASCAL_NAME} guest program.
#[derive(Serialize, Deserialize, RSerialize, RDeserialize, Archive, Clone, Debug)]
pub struct ${PASCAL_NAME}ProgramInput {
    /// State root before execution.
    pub initial_state_root: [u8; 32],
    /// TODO: Add program-specific input fields.
    pub data: Vec<u8>,
}

/// Output of the ${PASCAL_NAME} guest program.
///
/// Committed as public values by the zkVM so the L1 verifier can
/// check the state transition.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ${PASCAL_NAME}ProgramOutput {
    /// State root before execution.
    pub initial_state_root: [u8; 32],
    /// State root after execution.
    pub final_state_root: [u8; 32],
    /// Number of items processed.
    pub item_count: u64,
}

impl ${PASCAL_NAME}ProgramOutput {
    /// Encode the output to bytes for L1 commitment verification.
    ///
    /// Layout: \`initial_state_root (32) || final_state_root (32) || item_count (8 BE)\`
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(72);
        buf.extend_from_slice(&self.initial_state_root);
        buf.extend_from_slice(&self.final_state_root);
        buf.extend_from_slice(&self.item_count.to_be_bytes());
        buf
    }
}
RUST_EOF

echo "  Created: $PROGRAMS_DIR/${SNAKE_NAME}/types.rs"

# ── execution.rs ─────────────────────────────────────────────────────

cat > "$PROGRAMS_DIR/${SNAKE_NAME}/execution.rs" << RUST_EOF
use ethrex_crypto::keccak::keccak_hash;

use super::types::{${PASCAL_NAME}ProgramInput, ${PASCAL_NAME}ProgramOutput};

/// Errors that can occur during ${PASCAL_NAME} execution.
#[derive(Debug, thiserror::Error)]
pub enum ${PASCAL_NAME}ExecutionError {
    #[error("Empty input")]
    EmptyInput,
    // TODO: Add program-specific error variants.
}

/// Execute the ${PASCAL_NAME} program logic.
///
/// # State transition model
///
/// \`\`\`text
/// state = initial_state_root
/// state = keccak256(state || data)
/// final_state_root = state
/// \`\`\`
pub fn execution_program(
    input: ${PASCAL_NAME}ProgramInput,
) -> Result<${PASCAL_NAME}ProgramOutput, ${PASCAL_NAME}ExecutionError> {
    if input.data.is_empty() {
        return Err(${PASCAL_NAME}ExecutionError::EmptyInput);
    }

    // Hash the initial state with the input data.
    let mut preimage = Vec::with_capacity(32 + input.data.len());
    preimage.extend_from_slice(&input.initial_state_root);
    preimage.extend_from_slice(&input.data);

    let final_state_root = keccak_hash(&preimage);

    Ok(${PASCAL_NAME}ProgramOutput {
        initial_state_root: input.initial_state_root,
        final_state_root,
        item_count: 1,
    })
}
RUST_EOF

echo "  Created: $PROGRAMS_DIR/${SNAKE_NAME}/execution.rs"

# ── mod.rs ───────────────────────────────────────────────────────────

cat > "$PROGRAMS_DIR/${SNAKE_NAME}/mod.rs" << RUST_EOF
pub mod execution;
pub mod types;

use crate::traits::{GuestProgram, GuestProgramError, backends};

/// ${PASCAL_NAME} Guest Program.
///
/// TODO: Add program description.
pub struct ${PASCAL_NAME}GuestProgram;

impl ${PASCAL_NAME}GuestProgram {
    fn non_empty(elf: &[u8]) -> Option<&[u8]> {
        if elf.is_empty() || elf == [0] {
            None
        } else {
            Some(elf)
        }
    }
}

impl GuestProgram for ${PASCAL_NAME}GuestProgram {
    fn program_id(&self) -> &str {
        "${PROGRAM_NAME}"
    }

    fn elf(&self, _backend: &str) -> Option<&[u8]> {
        // TODO: Add ELF lookup per backend once compiled.
        // Example:
        //   backends::SP1 => Self::non_empty(crate::ZKVM_SP1_${UPPER_SNAKE}_ELF),
        None
    }

    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
        None
    }

    fn program_type_id(&self) -> u8 {
        ${NEXT_TYPE_ID}
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_input.to_vec())
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_output.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::execution::execution_program;
    use super::types::${PASCAL_NAME}ProgramInput;

    #[test]
    fn program_id_is_correct() {
        let gp = ${PASCAL_NAME}GuestProgram;
        assert_eq!(gp.program_id(), "${PROGRAM_NAME}");
    }

    #[test]
    fn program_type_id_is_correct() {
        let gp = ${PASCAL_NAME}GuestProgram;
        assert_eq!(gp.program_type_id(), ${NEXT_TYPE_ID});
    }

    #[test]
    fn unsupported_backend_returns_none() {
        let gp = ${PASCAL_NAME}GuestProgram;
        assert!(gp.elf("nonexistent").is_none());
        assert!(gp.vk_bytes("nonexistent").is_none());
    }

    #[test]
    fn serialize_input_is_identity() {
        let gp = ${PASCAL_NAME}GuestProgram;
        let data = b"test data";
        assert_eq!(gp.serialize_input(data).unwrap(), data);
    }

    #[test]
    fn execution_produces_deterministic_output() {
        let input = ${PASCAL_NAME}ProgramInput {
            initial_state_root: [0xAA; 32],
            data: vec![1, 2, 3],
        };
        let output = execution_program(input.clone()).expect("should succeed");

        assert_eq!(output.initial_state_root, [0xAA; 32]);
        assert_eq!(output.item_count, 1);
        assert_ne!(output.final_state_root, output.initial_state_root);

        let output2 = execution_program(input).expect("should succeed");
        assert_eq!(output.final_state_root, output2.final_state_root);
    }

    #[test]
    fn execution_rejects_empty_input() {
        let input = ${PASCAL_NAME}ProgramInput {
            initial_state_root: [0; 32],
            data: vec![],
        };
        assert!(execution_program(input).is_err());
    }

    #[test]
    fn output_encode_length() {
        let input = ${PASCAL_NAME}ProgramInput {
            initial_state_root: [0xBB; 32],
            data: vec![1],
        };
        let output = execution_program(input).expect("should succeed");
        // 32 + 32 + 8 = 72 bytes
        assert_eq!(output.encode().len(), 72);
    }

    #[test]
    fn rkyv_roundtrip() {
        let input = ${PASCAL_NAME}ProgramInput {
            initial_state_root: [0xCC; 32],
            data: vec![1, 2, 3],
        };
        let bytes =
            rkyv::to_bytes::<rkyv::rancor::Error>(&input).expect("rkyv serialize");
        let restored: ${PASCAL_NAME}ProgramInput =
            rkyv::from_bytes::<${PASCAL_NAME}ProgramInput, rkyv::rancor::Error>(&bytes)
                .expect("rkyv deserialize");
        assert_eq!(restored.initial_state_root, input.initial_state_root);
    }
}
RUST_EOF

echo "  Created: $PROGRAMS_DIR/${SNAKE_NAME}/mod.rs"

# ── SP1 binary ───────────────────────────────────────────────────────

SP1_BIN_DIR="$BIN_DIR/sp1-${PROGRAM_NAME}"
mkdir -p "$SP1_BIN_DIR/src"

cat > "$SP1_BIN_DIR/Cargo.toml" << TOML_EOF
[package]
name = "ethrex-guest-sp1-${PROGRAM_NAME}"
version = "9.0.0"
edition = "2024"
license = "MIT OR Apache-2.0"

[workspace]

[profile.release]
lto = "thin"
codegen-units = 1

[dependencies]
sp1-zkvm = { version = "=5.0.8" }
rkyv = { version = "0.8.10", features = ["std", "unaligned"] }

ethrex-guest-program = { path = "../../", default-features = false }

[patch.crates-io]
tiny-keccak = { git = "https://github.com/sp1-patches/tiny-keccak", tag = "patch-2.0.2-sp1-4.0.0" }
TOML_EOF

cat > "$SP1_BIN_DIR/src/main.rs" << RUST_EOF
#![no_main]

use ethrex_guest_program::programs::${SNAKE_NAME}::execution::execution_program;
use ethrex_guest_program::programs::${SNAKE_NAME}::types::${PASCAL_NAME}ProgramInput;
use rkyv::rancor::Error;

sp1_zkvm::entrypoint!(main);

pub fn main() {
    let input = sp1_zkvm::io::read_vec();
    let input = rkyv::from_bytes::<${PASCAL_NAME}ProgramInput, Error>(&input).unwrap();

    let output = execution_program(input).unwrap();

    sp1_zkvm::io::commit_slice(&output.encode());
}
RUST_EOF

echo "  Created: $SP1_BIN_DIR/Cargo.toml"
echo "  Created: $SP1_BIN_DIR/src/main.rs"

# ── Register in mod.rs ───────────────────────────────────────────────

if ! grep -q "mod ${SNAKE_NAME};" "$MOD_FILE"; then
    # Add mod declaration after the last existing mod line
    LAST_MOD_LINE=$(grep -n "^pub mod " "$MOD_FILE" | tail -1 | cut -d: -f1)
    if [ -z "$LAST_MOD_LINE" ]; then
        LAST_MOD_LINE=$(grep -n "^mod " "$MOD_FILE" | tail -1 | cut -d: -f1)
    fi
    sed -i '' "${LAST_MOD_LINE}a\\
pub mod ${SNAKE_NAME};" "$MOD_FILE"
    echo "  Added: pub mod ${SNAKE_NAME}; to $MOD_FILE"
fi

if ! grep -q "pub use ${SNAKE_NAME}::${PASCAL_NAME}GuestProgram;" "$MOD_FILE"; then
    LAST_USE_LINE=$(grep -n "^pub use " "$MOD_FILE" | tail -1 | cut -d: -f1)
    sed -i '' "${LAST_USE_LINE}a\\
pub use ${SNAKE_NAME}::${PASCAL_NAME}GuestProgram;" "$MOD_FILE"
    echo "  Added: pub use ${SNAKE_NAME}::${PASCAL_NAME}GuestProgram; to $MOD_FILE"
fi

echo ""
echo "Done! Next steps:"
echo ""
echo "  1. Edit types:     $PROGRAMS_DIR/${SNAKE_NAME}/types.rs"
echo "  2. Edit execution: $PROGRAMS_DIR/${SNAKE_NAME}/execution.rs"
echo "  3. Register in prover:"
echo "       crates/l2/prover/src/prover.rs -> create_default_registry()"
echo "  4. Add ELF constant to lib.rs (optional, for compile-time embedding)"
echo "  5. Add build function to build.rs (optional, for GUEST_PROGRAMS)"
echo "  6. Run tests: cargo test -p ethrex-guest-program"
echo "  7. Build SP1 ELF: GUEST_PROGRAMS=${PROGRAM_NAME} make sp1-${PROGRAM_NAME}"
