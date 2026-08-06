//! 0x67 — cometBFTLightBlockValidate
//!
//! Validates a CometBFT light block for the BSC cross-chain bridge.  Used by
//! the BEP-341 light client for CometBFT v0.37.0 and compatible versions.
//!
//! # Input layout
//!
//! ```text
//! | cs_length (32, last 8 bytes = u64 BE) | consensus_state | light_block |
//! ```
//!
//! ## Consensus state binary format (v2)
//!
//! ```text
//! | chainID (32) | height (8) | nextValidatorSetHash (32) |
//! | [{ed25519_pubkey (32), voting_power (8), relayer_address (20), bls_key (48)}…] |
//! ```
//!
//! Each validator entry is 108 bytes.  Maximum 99 validators.
//!
//! ## Light block format
//!
//! The light block is encoded as a protobuf `LightBlock` message from the
//! `cometbft.types.v1` protobuf package.  Decoding it requires the full
//! CometBFT protobuf schema plus the signature-verification logic from
//! `cometbft/light`.
//!
//! # Output on success
//!
//! ```text
//! | validatorSetChanged (1) | padding (23) | consensusStateBytesLength (8) | new_consensus_state |
//! ```
//!
//! # Gas
//!
//! 3 000  (`params.CometBFTLightBlockValidateGas`)
//!
//! # Implementation
//!
//! Full light-client validation against **greenfield-cometbft v1.3.2** (the
//! fork bsc-geth vendors): protobuf `LightBlock` decode ([`proto`]), the
//! RFC-6962 SHA256 Merkle validator-set/header hashes ([`merkle`]), the
//! `CanonicalVote` sign-bytes, and Ed25519 commit verification with the
//! adjacent (≥2/3) / non-adjacent (≥1/3 trusting) branches ([`verify`]).
//! Pasteur (BEP-682) adds the unique-validator-set check and the per-byte gas.
//!
//! Reference: `core/vm/lightclient/v2/lightclient.go`
//! (`DecodeLightBlockValidationInput`, `ConsensusState.ApplyLightBlock`,
//! `EncodeLightBlockValidationResult`) and greenfield-cometbft `types/*`.

use super::PrecompileError;

mod merkle;
mod proto;
mod verify;

use prost::Message;
use verify::ValidatorInfo;

/// Gas cost for cometBFTLightBlockValidate.  Matches `params.CometBFTLightBlockValidateGas`.
pub const COMETBFT_VALIDATE_GAS: u64 = 3_000;

/// Per-input-byte gas added by Pasteur (BEP-682).
/// Matches `params.CometBFTLightBlockValidatePerByteGas`.
pub const COMETBFT_VALIDATE_PER_BYTE_GAS: u64 = 16;

// ── Layout constants ──────────────────────────────────────────────────────────

/// Outer 32-byte word carrying the consensus-state length in its last 8 bytes.
const CS_LEN_WORD: usize = 32;
const CS_LEN_OFFSET: usize = 24;

/// Fixed-size prefix of the v2 consensus state.
const CHAIN_ID_LEN: usize = 32;
const HEIGHT_LEN: usize = 8;
const NEXT_VAL_SET_HASH_LEN: usize = 32;
const CS_FIXED_LEN: usize = CHAIN_ID_LEN + HEIGHT_LEN + NEXT_VAL_SET_HASH_LEN;

/// Per-validator entry: ed25519 pubkey (32) + voting power (8) + relayer
/// address (20) + BLS key (48) = 108 bytes.
const VALIDATOR_ENTRY_LEN: usize = 32 + 8 + 20 + 48;

/// Maximum number of validators (99, matching the Go reference).
const MAX_VALIDATORS: usize = 99;
/// Maximum v2 consensus-state length.
const MAX_CS_LEN: usize = CS_FIXED_LEN + MAX_VALIDATORS * VALIDATOR_ENTRY_LEN;

// ── Public interface ──────────────────────────────────────────────────────────

/// A single validator entry in a v2 consensus state.
#[allow(dead_code)]
pub(crate) struct ValidatorEntryV2<'a> {
    pub pubkey: &'a [u8], // 32 bytes (ed25519)
    pub voting_power: i64,
    pub relayer_address: &'a [u8], // 20 bytes
    pub bls_key: &'a [u8],         // 48 bytes
}

/// Parsed representation of a v2 (CometBFT) consensus state.
#[allow(dead_code)]
pub(crate) struct ConsensusStateV2<'a> {
    pub chain_id: &'a [u8], // 32 bytes, null-padded
    pub height: u64,
    pub next_validator_set_hash: &'a [u8], // 32 bytes
    pub validators: Vec<ValidatorEntryV2<'a>>,
}

/// Run the cometBFTLightBlockValidate precompile.
///
/// Gas is charged before returning any error other than
/// [`PrecompileError::NotEnoughGas`].  Structural parsing of the consensus
/// state is complete; protobuf `LightBlock` decoding and Ed25519
/// commit-signature verification are not yet implemented.
pub fn run(
    input: &[u8],
    gas_limit: u64,
    is_pasteur: bool,
) -> Result<(u64, Vec<u8>), PrecompileError> {
    // Pasteur (BEP-682) changes the gas from a flat 3000 to
    // `3000 + 16 * len(input)`. Pre-Pasteur keeps the flat cost.
    let gas_cost = if is_pasteur {
        COMETBFT_VALIDATE_GAS
            .saturating_add(COMETBFT_VALIDATE_PER_BYTE_GAS.saturating_mul(input.len() as u64))
    } else {
        COMETBFT_VALIDATE_GAS
    };
    if gas_limit < gas_cost {
        return Err(PrecompileError::NotEnoughGas);
    }
    // bsc-geth's `cometBFTLightBlockValidate.Run` returns an error on
    // parse/validation failures, and the BSC CALL implementation burns ALL
    // forwarded gas when a precompile errors. See the matching note in
    // `tm_header_validate::run`.
    //
    let output = run_inner(input, is_pasteur)?;
    Ok((gas_cost, output))
}

fn run_inner(input: &[u8], is_pasteur: bool) -> Result<Vec<u8>, PrecompileError> {
    if input.is_empty() || input.len() <= CS_LEN_WORD {
        return Err(PrecompileError::InvalidInput);
    }

    let cs_length = u64::from_be_bytes(
        input[CS_LEN_OFFSET..CS_LEN_WORD]
            .try_into()
            .map_err(|_| PrecompileError::InvalidInput)?,
    ) as usize;
    let cs_end = CS_LEN_WORD
        .checked_add(cs_length)
        .ok_or(PrecompileError::InvalidInput)?;
    if input.len() <= cs_end {
        return Err(PrecompileError::InvalidInput);
    }

    let cs_bytes = &input[CS_LEN_WORD..cs_end];
    let light_block_bytes = &input[cs_end..];

    let cs = parse_consensus_state_v2(cs_bytes)?;

    if light_block_bytes.is_empty() {
        return Err(PrecompileError::InvalidInput);
    }

    let light_block =
        proto::LightBlock::decode(light_block_bytes).map_err(|_| PrecompileError::InvalidInput)?;

    // Trusted (consensus-state) validators, common representation.
    let trusted: Vec<ValidatorInfo> = cs
        .validators
        .iter()
        .map(|v| ValidatorInfo {
            pubkey: v.pubkey.to_vec(),
            voting_power: v.voting_power,
            bls_key: v.bls_key.to_vec(),
            relayer_address: v.relayer_address.to_vec(),
        })
        .collect();

    // BEP-682 (Pasteur): reject duplicate validators in the trusted set.
    if is_pasteur {
        validate_unique_validator_set(&trusted)?;
    }

    apply_light_block(&cs, &trusted, &light_block, is_pasteur)
}

/// `ConsensusState.ApplyLightBlock` + `EncodeLightBlockValidationResult`.
///
/// `isHertz` is always true here (0x67 is Hertz-era onward on BSC), so the
/// returned `validatorSetChanged` flag is the real pre-update value.
fn apply_light_block(
    cs: &ConsensusStateV2<'_>,
    trusted: &[ValidatorInfo],
    light_block: &proto::LightBlock,
    is_pasteur: bool,
) -> Result<Vec<u8>, PrecompileError> {
    let signed_header = light_block
        .signed_header
        .as_ref()
        .ok_or(PrecompileError::InvalidInput)?;
    let header = signed_header
        .header
        .as_ref()
        .ok_or(PrecompileError::InvalidInput)?;
    let commit = signed_header
        .commit
        .as_ref()
        .ok_or(PrecompileError::InvalidInput)?;
    let block_val_set = light_block
        .validator_set
        .as_ref()
        .ok_or(PrecompileError::InvalidInput)?;

    // Block validators in the common representation.
    let block_vals: Vec<ValidatorInfo> = block_val_set
        .validators
        .iter()
        .map(|v| {
            let pubkey = v
                .pub_key
                .as_ref()
                .and_then(|k| k.ed25519.clone())
                .ok_or(PrecompileError::InvalidInput)?;
            Ok(ValidatorInfo {
                pubkey,
                voting_power: v.voting_power,
                bls_key: v.bls_key.clone(),
                relayer_address: v.relayer_address.clone(),
            })
        })
        .collect::<Result<_, PrecompileError>>()?;

    if is_pasteur {
        validate_unique_validator_set(&block_vals)?;
    }

    // Height must advance.
    let block_height = header.height;
    if block_height <= cs.height as i64 {
        return Err(PrecompileError::InvalidInput);
    }

    // block.ValidateBasic(cs.ChainID) — the consensus-critical checks.
    let cs_chain_id = trim_nul(cs.chain_id);
    if header.chain_id.as_bytes() != cs_chain_id {
        return Err(PrecompileError::InvalidInput);
    }
    if commit.height != header.height {
        return Err(PrecompileError::InvalidInput);
    }
    // Header.Hash() == Commit.BlockID.Hash
    let commit_block_id = commit.block_id.clone().unwrap_or_default();
    if verify::header_hash(header).as_slice() != commit_block_id.hash.as_slice() {
        return Err(PrecompileError::InvalidInput);
    }
    // ValidatorSet.Hash() == Header.ValidatorsHash
    if verify::validator_set_hash(&block_vals).as_slice() != header.validators_hash.as_slice() {
        return Err(PrecompileError::InvalidInput);
    }

    // Commit verification, branching on adjacency.
    let chain_id_str =
        std::str::from_utf8(cs_chain_id).map_err(|_| PrecompileError::InvalidInput)?;
    let commit_bid = commit.block_id.clone().unwrap_or_default();
    if cs.height as i64 == block_height - 1 {
        // Adjacent: next-set hash must match, then ≥2/3 of the new set.
        if cs.next_validator_set_hash != header.validators_hash.as_slice() {
            return Err(PrecompileError::InvalidInput);
        }
        if !verify::verify_commit_light(
            chain_id_str,
            &commit_bid,
            block_height,
            commit,
            &block_vals,
        ) {
            return Err(PrecompileError::InvalidInput);
        }
    } else {
        // Non-adjacent: ≥1/3 of the trusted set, then ≥2/3 of the new set.
        if !verify::verify_commit_light_trusting(chain_id_str, commit, trusted) {
            return Err(PrecompileError::InvalidInput);
        }
        if !verify::verify_commit_light(
            chain_id_str,
            &commit_bid,
            block_height,
            commit,
            &block_vals,
        ) {
            return Err(PrecompileError::InvalidInput);
        }
    }

    // validatorSetChanged: old set hash vs new header's ValidatorsHash (pre-update).
    let validator_set_changed =
        verify::validator_set_hash(trusted).as_slice() != header.validators_hash.as_slice();

    // Encode the UPDATED consensus state (height, nextValidatorSetHash, new set).
    let updated_cs =
        encode_updated_consensus_state(cs_chain_id, block_height, header, &block_vals)?;

    // EncodeLightBlockValidationResult: [flag(1)][pad(23)][len(8 BE)][cs bytes].
    let mut out = vec![0u8; 32];
    out[0] = u8::from(validator_set_changed);
    out[24..32].copy_from_slice(&(updated_cs.len() as u64).to_be_bytes());
    out.extend_from_slice(&updated_cs);
    Ok(out)
}

/// Re-encode the updated consensus state in the v2 wire format:
/// chainID(32, NUL-padded) | height(8 BE) | nextValidatorSetHash(32) |
/// N × (pubkey 32 | votingPower 8 BE | relayerAddress 20 | blsKey 48).
fn encode_updated_consensus_state(
    chain_id: &[u8],
    height: i64,
    header: &proto::Header,
    validators: &[ValidatorInfo],
) -> Result<Vec<u8>, PrecompileError> {
    if validators.len() > MAX_VALIDATORS {
        return Err(PrecompileError::InvalidInput);
    }
    if header.next_validators_hash.len() != NEXT_VAL_SET_HASH_LEN {
        return Err(PrecompileError::InvalidInput);
    }
    let mut out = vec![0u8; CS_FIXED_LEN + validators.len() * VALIDATOR_ENTRY_LEN];
    let mut pos = 0;
    let cid_len = chain_id.len().min(CHAIN_ID_LEN);
    out[pos..pos + cid_len].copy_from_slice(&chain_id[..cid_len]);
    pos += CHAIN_ID_LEN;
    out[pos..pos + HEIGHT_LEN].copy_from_slice(&(height as u64).to_be_bytes());
    pos += HEIGHT_LEN;
    out[pos..pos + NEXT_VAL_SET_HASH_LEN].copy_from_slice(&header.next_validators_hash);
    pos += NEXT_VAL_SET_HASH_LEN;
    for v in validators {
        if v.pubkey.len() != 32 || v.relayer_address.len() != 20 || v.bls_key.len() != 48 {
            return Err(PrecompileError::InvalidInput);
        }
        out[pos..pos + 32].copy_from_slice(&v.pubkey);
        pos += 32;
        out[pos..pos + 8].copy_from_slice(&(v.voting_power as u64).to_be_bytes());
        pos += 8;
        out[pos..pos + 20].copy_from_slice(&v.relayer_address);
        pos += 20;
        out[pos..pos + 48].copy_from_slice(&v.bls_key);
        pos += 48;
    }
    Ok(out)
}

/// Trim NUL bytes from both ends (`bytes.Trim(_, "\x00")`).
fn trim_nul(b: &[u8]) -> &[u8] {
    let start = b.iter().position(|&x| x != 0).unwrap_or(b.len());
    let end = b.iter().rposition(|&x| x != 0).map_or(start, |i| i + 1);
    &b[start..end]
}

/// BEP-682 `validateUniqueValidatorSet`: reject duplicate `address` and
/// `pubkey` always; duplicate `bls_key` / `relayer_address` only when non-zero.
fn validate_unique_validator_set(validators: &[ValidatorInfo]) -> Result<(), PrecompileError> {
    use std::collections::HashSet;
    let mut addrs = HashSet::new();
    let mut pubkeys = HashSet::new();
    let mut bls = HashSet::new();
    let mut relayers = HashSet::new();
    let is_zero = |b: &[u8]| b.iter().all(|&x| x == 0);
    for v in validators {
        if !addrs.insert(v.address()) {
            return Err(PrecompileError::InvalidInput);
        }
        if !pubkeys.insert(v.pubkey.clone()) {
            return Err(PrecompileError::InvalidInput);
        }
        if !v.bls_key.is_empty() && !is_zero(&v.bls_key) && !bls.insert(v.bls_key.clone()) {
            return Err(PrecompileError::InvalidInput);
        }
        if !v.relayer_address.is_empty()
            && !is_zero(&v.relayer_address)
            && !relayers.insert(v.relayer_address.clone())
        {
            return Err(PrecompileError::InvalidInput);
        }
    }
    Ok(())
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Validate and parse the v2 consensus-state binary blob.
///
/// Layout:
/// ```text
/// | chainID (32) | height (8) | nextValidatorSetHash (32) |
/// | [{pubkey (32), votingPower (8), relayerAddress (20), blsKey (48)}…] |
/// ```
pub(crate) fn parse_consensus_state_v2(
    input: &[u8],
) -> Result<ConsensusStateV2<'_>, PrecompileError> {
    let len = input.len();

    if len <= CS_FIXED_LEN {
        return Err(PrecompileError::InvalidInput);
    }

    let variable_len = len - CS_FIXED_LEN;
    if !variable_len.is_multiple_of(VALIDATOR_ENTRY_LEN) {
        return Err(PrecompileError::InvalidInput);
    }

    let num_validators = variable_len / VALIDATOR_ENTRY_LEN;
    if num_validators > MAX_VALIDATORS {
        return Err(PrecompileError::InvalidInput);
    }

    // Enforce the absolute size cap.
    if len > MAX_CS_LEN {
        return Err(PrecompileError::InvalidInput);
    }

    let mut pos = 0;

    let chain_id = &input[pos..pos + CHAIN_ID_LEN];
    pos += CHAIN_ID_LEN;

    let height = u64::from_be_bytes(
        input[pos..pos + HEIGHT_LEN]
            .try_into()
            .expect("slice is exactly 8 bytes"),
    );
    pos += HEIGHT_LEN;

    let next_validator_set_hash = &input[pos..pos + NEXT_VAL_SET_HASH_LEN];
    pos += NEXT_VAL_SET_HASH_LEN;

    let mut validators = Vec::with_capacity(num_validators);
    for _ in 0..num_validators {
        let pubkey = &input[pos..pos + 32];
        pos += 32;
        let voting_power = i64::from_be_bytes(
            input[pos..pos + 8]
                .try_into()
                .expect("slice is exactly 8 bytes"),
        );
        pos += 8;
        let relayer_address = &input[pos..pos + 20];
        pos += 20;
        let bls_key = &input[pos..pos + 48];
        pos += 48;

        validators.push(ValidatorEntryV2 {
            pubkey,
            voting_power,
            relayer_address,
            bls_key,
        });
    }

    Ok(ConsensusStateV2 {
        chain_id,
        height,
        next_validator_set_hash,
        validators,
    })
}

/// Encode a v2 consensus state back to its binary wire format.
#[allow(dead_code)]
pub(crate) fn encode_consensus_state_v2(
    cs: &ConsensusStateV2<'_>,
) -> Result<Vec<u8>, PrecompileError> {
    let num_validators = cs.validators.len();
    if num_validators > MAX_VALIDATORS {
        return Err(PrecompileError::InvalidInput);
    }

    let total = CS_FIXED_LEN + num_validators * VALIDATOR_ENTRY_LEN;
    let mut out = vec![0u8; total];
    let mut pos = 0;

    let chain_id_len = cs.chain_id.len().min(CHAIN_ID_LEN);
    out[pos..pos + chain_id_len].copy_from_slice(&cs.chain_id[..chain_id_len]);
    pos += CHAIN_ID_LEN;

    out[pos..pos + HEIGHT_LEN].copy_from_slice(&cs.height.to_be_bytes());
    pos += HEIGHT_LEN;

    out[pos..pos + NEXT_VAL_SET_HASH_LEN].copy_from_slice(cs.next_validator_set_hash);
    pos += NEXT_VAL_SET_HASH_LEN;

    for v in &cs.validators {
        out[pos..pos + 32].copy_from_slice(v.pubkey);
        pos += 32;
        out[pos..pos + 8].copy_from_slice(&v.voting_power.to_be_bytes());
        pos += 8;
        out[pos..pos + 20].copy_from_slice(v.relayer_address);
        pos += 20;
        out[pos..pos + 48].copy_from_slice(v.bls_key);
        pos += 48;
    }

    Ok(out)
}

/// Encode the light-block validation result.
///
/// ```text
/// | validatorSetChanged (1) | padding (23) | consensusStateBytesLength (8) | new_consensus_state |
/// ```
#[allow(dead_code)]
pub(crate) fn encode_result(validator_set_changed: bool, consensus_state_bytes: &[u8]) -> Vec<u8> {
    let mut header = vec![0u8; 32];
    if validator_set_changed {
        header[0] = 0x01;
    }
    header[24..32].copy_from_slice(&(consensus_state_bytes.len() as u64).to_be_bytes());
    let mut result = header;
    result.extend_from_slice(consensus_state_bytes);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid input: cs_length word + cs_bytes + light_block_bytes.
    fn build_input(cs_bytes: &[u8], light_block: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; CS_LEN_WORD];
        out[CS_LEN_OFFSET..CS_LEN_WORD].copy_from_slice(&(cs_bytes.len() as u64).to_be_bytes());
        out.extend_from_slice(cs_bytes);
        out.extend_from_slice(light_block);
        out
    }

    /// Build a minimal valid v2 consensus-state bytes with `n` validators.
    fn build_cs_bytes(n: usize) -> Vec<u8> {
        let total = CS_FIXED_LEN + n * VALIDATOR_ENTRY_LEN;
        let mut v = vec![0u8; total];
        // height = 1
        v[CHAIN_ID_LEN..CHAIN_ID_LEN + HEIGHT_LEN].copy_from_slice(&1u64.to_be_bytes());
        v
    }

    #[test]
    fn test_not_enough_gas() {
        let input = build_input(&build_cs_bytes(1), &[0x00]);
        assert_eq!(
            run(&input, COMETBFT_VALIDATE_GAS - 1, false),
            Err(PrecompileError::NotEnoughGas)
        );
    }

    #[test]
    fn test_empty_input_rejected() {
        assert_eq!(
            run(&[], COMETBFT_VALIDATE_GAS, false),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_only_cs_len_word_rejected() {
        // Input is exactly 32 bytes — no room for cs_bytes or light block.
        let input = vec![0u8; CS_LEN_WORD];
        assert_eq!(
            run(&input, COMETBFT_VALIDATE_GAS, false),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_cs_length_overflow_rejected() {
        // Set cs_length to u64::MAX to trigger overflow guard.
        let mut input = vec![0u8; CS_LEN_WORD + 1];
        input[CS_LEN_OFFSET..CS_LEN_WORD].copy_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(
            run(&input, COMETBFT_VALIDATE_GAS, false),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_cs_structure_validated() {
        // cs_bytes length not aligned to VALIDATOR_ENTRY_LEN
        let bad_cs = vec![0u8; CS_FIXED_LEN + 1];
        let input = build_input(&bad_cs, &[0x00]);
        assert_eq!(
            run(&input, COMETBFT_VALIDATE_GAS, false),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_undecodable_light_block_rejected() {
        // A single 0x00 byte is not a decodable protobuf `LightBlock`
        // (field number 0 is invalid), so validation fails → the CALL burns all
        // forwarded gas, matching bsc-geth.
        let input = build_input(&build_cs_bytes(1), &[0x00]);
        assert_eq!(
            run(&input, COMETBFT_VALIDATE_GAS, false),
            Err(PrecompileError::InvalidInput)
        );
    }

    #[test]
    fn test_parse_cs_v2_roundtrip() {
        let n = 2;
        let cs_bytes = build_cs_bytes(n);
        let cs = parse_consensus_state_v2(&cs_bytes).unwrap();
        assert_eq!(cs.height, 1);
        assert_eq!(cs.validators.len(), n);

        let encoded = encode_consensus_state_v2(&cs).unwrap();
        assert_eq!(encoded, cs_bytes);
    }

    #[test]
    fn test_encode_result_layout() {
        let cs_data = vec![0x42u8; 10];
        let result = encode_result(true, &cs_data);
        assert_eq!(result[0], 0x01);
        assert_eq!(&result[1..24], &[0u8; 23]);
        assert_eq!(&result[24..32], &10u64.to_be_bytes());
        assert_eq!(&result[32..], &cs_data[..]);
    }

    #[test]
    fn test_encode_result_layout_false() {
        let cs_data = vec![0x42u8; 5];
        let result = encode_result(false, &cs_data);
        assert_eq!(result[0], 0x00);
    }
}
