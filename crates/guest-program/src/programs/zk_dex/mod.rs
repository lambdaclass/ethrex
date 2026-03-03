pub mod circuit;
pub mod events;
pub mod notes;
pub mod orders;
pub mod storage;

use crate::traits::{GuestProgram, GuestProgramError, ResourceLimits, backends};

/// DEX contract address on the L2 (must match the guest binary constant).
const DEX_CONTRACT_ADDRESS: ethrex_common::Address = ethrex_common::H160([0xDE; 20]);

/// ZK-DEX Guest Program — privacy-preserving decentralized exchange.
///
/// This program proves batch state transitions for the ZkDex contract using
/// the [`DexCircuit`](circuit::DexCircuit) implementation of the [`AppCircuit`]
/// trait. Supports 8 operation types: token transfer, mint, spend, liquidate,
/// convertNote, makeOrder, takeOrder, and settleOrder.
///
/// Reference: <https://github.com/tokamak-network/zk-dex/tree/circom>
///
/// [`AppCircuit`]: crate::common::app_execution::AppCircuit
pub struct ZkDexGuestProgram;

impl ZkDexGuestProgram {
    fn non_empty(elf: &[u8]) -> Option<&[u8]> {
        if elf.is_empty() || elf == [0] {
            None
        } else {
            Some(elf)
        }
    }
}

impl GuestProgram for ZkDexGuestProgram {
    fn program_id(&self) -> &str {
        "zk-dex"
    }

    fn elf(&self, backend: &str) -> Option<&[u8]> {
        match backend {
            backends::SP1 => Self::non_empty(crate::ZKVM_SP1_ZK_DEX_ELF),
            _ => None,
        }
    }

    fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
        None
    }

    fn program_type_id(&self) -> u8 {
        2 // ZK-DEX
    }

    fn serialize_input(&self, raw_input: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        #[cfg(feature = "l2")]
        {
            use crate::common::input_converter::convert_to_app_input;
            use crate::l2::ProgramInput;
            use rkyv::rancor::Error as RkyvError;

            let program_input: ProgramInput =
                rkyv::from_bytes::<ProgramInput, RkyvError>(raw_input)
                    .map_err(|e| GuestProgramError::Serialization(e.to_string()))?;

            let (accounts, storage_slots) = analyze_zk_dex_transactions(
                &program_input.blocks,
                DEX_CONTRACT_ADDRESS,
                &program_input.fee_configs,
            )
            .map_err(|e| GuestProgramError::Internal(e.to_string()))?;

            let app_input = convert_to_app_input(program_input, &accounts, &storage_slots)
                .map_err(|e| GuestProgramError::Internal(e.to_string()))?;

            let bytes = rkyv::to_bytes::<RkyvError>(&app_input)
                .map_err(|e| GuestProgramError::Serialization(e.to_string()))?;
            Ok(bytes.to_vec())
        }
    }

    fn encode_output(&self, raw_output: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
        Ok(raw_output.to_vec())
    }

    fn resource_limits(&self) -> ResourceLimits {
        ResourceLimits {
            max_input_bytes: Some(64 * 1024 * 1024), // 64 MB
            max_proving_duration: Some(std::time::Duration::from_secs(1800)), // 30 minutes
        }
    }

    fn version(&self) -> &str {
        "0.1.0"
    }
}

/// Analyze zk-dex batch transactions to determine which accounts and storage
/// slots are needed for proof generation.
///
/// For each transaction in the blocks:
/// - Sender address → needed for nonce/balance verification
/// - Recipient address → needed for balance updates
/// - DEX contract operations: parse calldata to find needed storage slots
/// - Privileged txs (deposits): recipient account
/// - Withdrawals: bridge contract account
/// - System calls: system contract account
#[cfg(feature = "l2")]
fn analyze_zk_dex_transactions(
    blocks: &[ethrex_common::types::Block],
    dex_contract: ethrex_common::Address,
    fee_configs: &[ethrex_common::types::l2::fee_config::FeeConfig],
) -> Result<
    (
        Vec<ethrex_common::Address>,
        Vec<(ethrex_common::Address, ethrex_common::H256)>,
    ),
    String,
> {
    use std::collections::BTreeSet;

    use ethrex_common::types::TxKind;
    use ethrex_common::{H256, U256};

    use crate::common::handlers::constants::{
        BURN_ADDRESS, COMMON_BRIDGE_L2_ADDRESS, FEE_TOKEN_RATIO_ADDRESS,
        FEE_TOKEN_REGISTRY_ADDRESS, L2_TO_L1_MESSENGER_ADDRESS, MESSENGER_LAST_MESSAGE_ID_SLOT,
    };

    let mut accounts: BTreeSet<ethrex_common::Address> = BTreeSet::new();
    let mut storage_slots: BTreeSet<(ethrex_common::Address, ethrex_common::H256)> =
        BTreeSet::new();

    // Selectors for all supported operations.
    let transfer_sel = circuit::transfer_selector_bytes();
    let mint_sel = circuit::mint_selector_bytes();
    let spend_sel = circuit::spend_selector_bytes();
    let liquidate_sel = circuit::liquidate_selector_bytes();
    let convert_note_sel = circuit::convert_note_selector_bytes();
    let make_order_sel = circuit::make_order_selector_bytes();
    let take_order_sel = circuit::take_order_selector_bytes();
    let settle_order_sel = circuit::settle_order_selector_bytes();

    let mut has_withdrawal = false;
    let mut has_non_privileged = false;

    for block in blocks {
        for tx in &block.body.transactions {
            // Privileged (deposit) transactions.
            if tx.is_privileged() {
                if let TxKind::Call(to) = tx.to() {
                    accounts.insert(to);
                    let data = tx.data();
                    if to == COMMON_BRIDGE_L2_ADDRESS && data.len() >= 36 {
                        let recipient = ethrex_common::Address::from_slice(&data[16..36]);
                        accounts.insert(recipient);
                    }
                }
                continue;
            }

            // Sender always needed (nonce, balance for gas).
            if let Ok(sender) = tx.sender() {
                accounts.insert(sender);

                let to_addr = match tx.to() {
                    TxKind::Call(addr) => addr,
                    TxKind::Create => continue,
                };

                accounts.insert(to_addr);

                has_non_privileged = true;

                // Withdrawal via CommonBridgeL2.
                if to_addr == COMMON_BRIDGE_L2_ADDRESS {
                    has_withdrawal = true;
                    accounts.insert(COMMON_BRIDGE_L2_ADDRESS);
                    continue;
                }

                // System contract calls.
                if to_addr == L2_TO_L1_MESSENGER_ADDRESS
                    || to_addr == FEE_TOKEN_REGISTRY_ADDRESS
                    || to_addr == FEE_TOKEN_RATIO_ADDRESS
                {
                    accounts.insert(to_addr);
                    continue;
                }

                // DEX contract operations — extract needed storage slots.
                if to_addr == dex_contract {
                    let data = tx.data();
                    if data.len() < 4 {
                        continue;
                    }
                    let sel = &data[..4];
                    accounts.insert(dex_contract);

                    if sel == transfer_sel && data.len() >= 4 + 96 {
                        // transfer(address to, address token, uint256 amount)
                        let transfer_to = ethrex_common::Address::from_slice(&data[4 + 12..4 + 32]);
                        let token = ethrex_common::Address::from_slice(&data[4 + 32 + 12..4 + 64]);
                        storage_slots
                            .insert((dex_contract, circuit::balance_storage_slot(token, sender)));
                        storage_slots.insert((
                            dex_contract,
                            circuit::balance_storage_slot(token, transfer_to),
                        ));
                    } else if sel == mint_sel && data.len() >= 420 {
                        // mint: notes[noteHash] + encryptedNotes[noteHash]
                        let note_hash = H256::from_slice(&data[292..324]);
                        add_note_slots(
                            &mut storage_slots,
                            dex_contract,
                            note_hash,
                            data,
                            true,
                            388,
                        );
                    } else if sel == spend_sel && data.len() >= 484 {
                        // spend: up to 4 note slots + 2 encrypted notes
                        for i in 0..4 {
                            let offset = 292 + i * 32;
                            let hash = H256::from_slice(&data[offset..offset + 32]);
                            if hash != notes::EMPTY_NOTE_HASH {
                                storage_slots
                                    .insert((dex_contract, storage::note_state_slot(hash)));
                                // New notes (indices 2,3) get encrypted notes
                                if i >= 2 {
                                    let enc_offset_pos = 420 + (i - 2) * 32;
                                    add_encrypted_note_slots(
                                        &mut storage_slots,
                                        dex_contract,
                                        hash,
                                        data,
                                        enc_offset_pos,
                                    );
                                }
                            }
                        }
                    } else if sel == liquidate_sel && data.len() >= 420 {
                        // liquidate: note + recipient account
                        let to = ethrex_common::Address::from_slice(&data[4 + 12..4 + 32]);
                        let note_hash = H256::from_slice(&data[324..356]);
                        accounts.insert(to);
                        storage_slots.insert((dex_contract, storage::note_state_slot(note_hash)));
                    } else if sel == convert_note_sel && data.len() >= 420 {
                        // convertNote: smartNote + newNote + encryptedNotes[newNote]
                        let smart_note = H256::from_slice(&data[292..324]);
                        let new_note = H256::from_slice(&data[356..388]);
                        storage_slots.insert((dex_contract, storage::note_state_slot(smart_note)));
                        add_note_slots(&mut storage_slots, dex_contract, new_note, data, true, 388);
                    } else if sel == make_order_sel && data.len() >= 420 {
                        // makeOrder: orders.length + order fields + maker note
                        let maker_note = H256::from_slice(&data[356..388]);
                        storage_slots.insert((dex_contract, storage::note_state_slot(maker_note)));
                        // orders.length
                        storage_slots.insert((dex_contract, storage::orders_length_slot()));
                        // We need to read orders.length to know the index, but for the
                        // witness we pre-allocate order field slots. Read the current
                        // length from the execution witness to compute the exact index.
                        // For safety, we add a slot range (the actual index will be
                        // determined at execution time and slots auto-created by set_storage).
                        add_order_field_slots_for_next(&mut storage_slots, dex_contract);
                    } else if sel == &take_order_sel && data.len() >= 516 {
                        // takeOrder: 2 notes + order fields + encrypted staking note
                        let order_id = U256::from_big_endian(&data[4..36]);
                        let parent_note = H256::from_slice(&data[324..356]);
                        let stake_note = H256::from_slice(&data[388..420]);
                        storage_slots.insert((dex_contract, storage::note_state_slot(parent_note)));
                        add_note_slots(
                            &mut storage_slots,
                            dex_contract,
                            stake_note,
                            data,
                            true,
                            484,
                        );
                        add_order_field_slots(&mut storage_slots, dex_contract, order_id);
                    } else if sel == settle_order_sel && data.len() >= 772 {
                        // settleOrder: 3 new notes + 3 old notes (from order) + order state
                        let order_id = U256::from_big_endian(&data[4..36]);
                        let reward_note = H256::from_slice(&data[452..484]);
                        let payment_note = H256::from_slice(&data[548..580]);
                        let change_note = H256::from_slice(&data[644..676]);

                        // New notes need state + encrypted data slots
                        for note_hash in [reward_note, payment_note, change_note] {
                            storage_slots
                                .insert((dex_contract, storage::note_state_slot(note_hash)));
                        }
                        // Encrypted notes for the 3 new notes.
                        // We estimate size from encDatas or use a conservative estimate.
                        add_settle_encrypted_note_slots(
                            &mut storage_slots,
                            dex_contract,
                            reward_note,
                            payment_note,
                            change_note,
                            data,
                        );

                        // Order fields (to read makerNote, parentNote, takerNoteToMaker)
                        add_order_field_slots(&mut storage_slots, dex_contract, order_id);

                        // Old notes from order (we don't know the hashes yet,
                        // but we need the order fields to read them).
                        // The execute function will read them from order storage.
                        // We need their note_state_slots too, but we can only know
                        // them after reading the order. For now, we rely on the
                        // execution witness containing all touched slots.
                        // In practice, the L2 execution already touched these slots,
                        // so they'll be in the ExecutionWitness.
                    }
                }
            }
        }
    }

    // ── Withdrawal-required accounts/storage ──
    if has_withdrawal {
        accounts.insert(BURN_ADDRESS);
        accounts.insert(L2_TO_L1_MESSENGER_ADDRESS);
        storage_slots.insert((L2_TO_L1_MESSENGER_ADDRESS, MESSENGER_LAST_MESSAGE_ID_SLOT));
    }

    // ── Gas fee distribution accounts ──
    if has_non_privileged {
        for block in blocks {
            accounts.insert(block.header.coinbase);
        }
        for fc in fee_configs {
            if let Some(op) = fc.operator_fee_config {
                accounts.insert(op.operator_fee_vault);
            }
        }
    }

    Ok((
        accounts.into_iter().collect(),
        storage_slots.into_iter().collect(),
    ))
}

// ── Witness analyzer helpers ────────────────────────────────────

/// Add note state slot and encrypted note storage slots for a note.
/// If `has_encrypted` is true, extracts the encrypted note size from
/// the ABI-encoded bytes at `enc_offset_pos` in calldata.
#[cfg(feature = "l2")]
fn add_note_slots(
    storage_slots: &mut std::collections::BTreeSet<(ethrex_common::Address, ethrex_common::H256)>,
    contract: ethrex_common::Address,
    note_hash: ethrex_common::H256,
    data: &[u8],
    has_encrypted: bool,
    enc_offset_pos: usize,
) {
    storage_slots.insert((contract, storage::note_state_slot(note_hash)));

    if has_encrypted {
        add_encrypted_note_slots(storage_slots, contract, note_hash, data, enc_offset_pos);
    }
}

/// Add encrypted note storage slots based on ABI-encoded bytes size.
#[cfg(feature = "l2")]
fn add_encrypted_note_slots(
    storage_slots: &mut std::collections::BTreeSet<(ethrex_common::Address, ethrex_common::H256)>,
    contract: ethrex_common::Address,
    note_hash: ethrex_common::H256,
    data: &[u8],
    enc_offset_pos: usize,
) {
    use ethrex_common::U256;

    let enc_len = if data.len() >= enc_offset_pos + 32 {
        let offset =
            U256::from_big_endian(&data[enc_offset_pos..enc_offset_pos + 32]).low_u64() as usize;
        let abs_pos = 4 + offset;
        if data.len() >= abs_pos + 32 {
            U256::from_big_endian(&data[abs_pos..abs_pos + 32]).low_u64() as usize
        } else {
            256 // Conservative estimate
        }
    } else {
        256 // Conservative estimate
    };

    for slot in storage::encrypted_note_slots(note_hash, enc_len) {
        storage_slots.insert((contract, slot));
    }
}

/// Add all 7 field slots for an existing order.
#[cfg(feature = "l2")]
fn add_order_field_slots(
    storage_slots: &mut std::collections::BTreeSet<(ethrex_common::Address, ethrex_common::H256)>,
    contract: ethrex_common::Address,
    order_id: ethrex_common::U256,
) {
    for field in 0..7u64 {
        storage_slots.insert((contract, storage::order_field_slot(order_id, field)));
    }
}

/// Add order field slots for the next order (to be created by makeOrder).
///
/// Since we don't know the order index yet (it comes from orders.length),
/// the `set_storage` calls in `execute_make_order` will auto-create the
/// slots in AppState. We just need orders.length to be available.
#[cfg(feature = "l2")]
fn add_order_field_slots_for_next(
    storage_slots: &mut std::collections::BTreeSet<(ethrex_common::Address, ethrex_common::H256)>,
    contract: ethrex_common::Address,
) {
    // orders.length is already added by the caller.
    // For new order slots, set_storage in AppState creates them on write.
    // We only need the length slot to read the current count.
    let _ = (storage_slots, contract);
}

/// Add encrypted note slots for settleOrder's 3 new notes.
///
/// Uses a conservative size estimate since the individual encrypted notes
/// are RLP-decoded from encDatas at execution time.
#[cfg(feature = "l2")]
fn add_settle_encrypted_note_slots(
    storage_slots: &mut std::collections::BTreeSet<(ethrex_common::Address, ethrex_common::H256)>,
    contract: ethrex_common::Address,
    reward: ethrex_common::H256,
    payment: ethrex_common::H256,
    change: ethrex_common::H256,
    data: &[u8],
) {
    use ethrex_common::U256;

    // Estimate total encDatas size and divide by 3 for per-note estimate.
    let estimated_per_note = if data.len() >= 772 {
        let offset = U256::from_big_endian(&data[740..772]).low_u64() as usize;
        let abs_pos = 4 + offset;
        if data.len() >= abs_pos + 32 {
            let total_len = U256::from_big_endian(&data[abs_pos..abs_pos + 32]).low_u64() as usize;
            total_len / 3
        } else {
            256
        }
    } else {
        256
    };

    for note_hash in [reward, payment, change] {
        for slot in storage::encrypted_note_slots(note_hash, estimated_per_note) {
            storage_slots.insert((contract, slot));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_id_is_zk_dex() {
        let gp = ZkDexGuestProgram;
        assert_eq!(gp.program_id(), "zk-dex");
    }

    #[test]
    fn program_type_id_is_two() {
        let gp = ZkDexGuestProgram;
        assert_eq!(gp.program_type_id(), 2);
    }

    #[test]
    fn sp1_elf_lookup() {
        let gp = ZkDexGuestProgram;
        let result = gp.elf(crate::traits::backends::SP1);
        if crate::ZKVM_SP1_ZK_DEX_ELF.is_empty() {
            assert!(result.is_none());
        } else {
            assert!(result.is_some());
        }
    }

    #[test]
    fn unsupported_backend_returns_none() {
        let gp = ZkDexGuestProgram;
        assert!(gp.elf("risc0").is_none());
        assert!(gp.elf("nonexistent").is_none());
    }

    #[test]
    fn serialize_input_rejects_invalid_bytes() {
        let gp = ZkDexGuestProgram;
        let data = b"test data";
        assert!(
            gp.serialize_input(data).is_err(),
            "serialize_input should reject arbitrary bytes"
        );
    }
}
