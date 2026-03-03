//! Withdrawal (L2->L1) handler.
//!
//! Fixes the three root causes of SP1 proof verification failure for withdrawals:
//!   1. BURN_ADDRESS must receive the withdrawn value (EVM: bridge -> BURN_ADDRESS.call{value})
//!   2. L1Messenger.lastMessageId must be incremented (storage slot 0)
//!   3. Correct withdrawal logs must be generated (WithdrawalInitiated + L1Message)
//!
//! The `get_block_l1_messages()` function in `crates/l2/common/src/messages.rs`
//! extracts L1Message events (topic = L1MESSAGE_TOPIC, address = MESSENGER_ADDRESS)
//! from receipts to compute `l1_out_messages_merkle_root`. The Log 2 generated here
//! must exactly match that filter.

use bytes::Bytes;

use ethrex_common::types::{Log, Transaction};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::keccak::keccak_hash;

use super::constants::{
    BURN_ADDRESS, COMMON_BRIDGE_L2_ADDRESS, ETH_TOKEN_ADDRESS, L1MESSAGE_TOPIC,
    L2_TO_L1_MESSENGER_ADDRESS, MESSENGER_LAST_MESSAGE_ID_SLOT, WITHDRAWAL_GAS,
    WITHDRAWAL_INITIATED_TOPIC,
};
use crate::common::app_execution::AppCircuitError;
use crate::common::app_state::AppState;

/// Handle a withdrawal (L2->L1) transaction via CommonBridgeL2.
///
/// Performs the three state changes that the EVM executes:
///   1. sender -= value (gas is handled separately by `gas_fee` module)
///   2. BURN_ADDRESS += value (EVM: bridge -> BURN_ADDRESS.call{value}(""))
///   3. Messenger.lastMessageId += 1 (storage slot 0)
///
/// Returns `(gas_used, new_message_id)` for receipt and log generation.
pub fn handle_withdrawal(
    state: &mut AppState,
    tx: &Transaction,
    sender: Address,
) -> Result<(u64, U256), AppCircuitError> {
    let value = tx.value();

    // 1. sender -= value
    if !value.is_zero() {
        state.debit_balance(sender, value)?;
    }

    // 2. BURN_ADDRESS += value
    if !value.is_zero() {
        state.credit_balance(BURN_ADDRESS, value)?;
    }

    // 3. Messenger.lastMessageId += 1
    let current_id =
        state.get_storage(L2_TO_L1_MESSENGER_ADDRESS, MESSENGER_LAST_MESSAGE_ID_SLOT)?;
    let new_id = current_id + U256::one();
    state.set_storage(
        L2_TO_L1_MESSENGER_ADDRESS,
        MESSENGER_LAST_MESSAGE_ID_SLOT,
        new_id,
    )?;

    Ok((WITHDRAWAL_GAS, new_id))
}

/// Generate the two EVM-matching event logs for a withdrawal transaction.
///
/// Log 1: `WithdrawalInitiated(address indexed sender, address indexed receiver, uint256 indexed amount)`
///         emitted by CommonBridgeL2 (0xffff)
///
/// Log 2: `L1Message(address indexed senderOnL2, bytes32 indexed data, uint256 indexed messageId)`
///         emitted by L2ToL1Messenger (0xfffe)
///
/// The `data_hash` in Log 2 is `keccak256(abi.encodePacked(ETH_TOKEN, ETH_TOKEN, receiverOnL1, value))`.
pub fn generate_withdrawal_logs(sender: Address, tx: &Transaction, message_id: U256) -> Vec<Log> {
    let value = tx.value();
    let data = tx.data();

    // Parse receiver from calldata: data[16..36] = ABI-encoded address
    let receiver_on_l1 = if data.len() >= 36 {
        Address::from_slice(&data[16..36])
    } else {
        sender // fallback: send back to sender
    };

    // Log 1: WithdrawalInitiated from bridge (0xffff) -- all params indexed
    let log1 = Log {
        address: COMMON_BRIDGE_L2_ADDRESS,
        topics: vec![
            *WITHDRAWAL_INITIATED_TOPIC,
            addr_to_h256(sender),
            addr_to_h256(receiver_on_l1),
            u256_to_h256(value),
        ],
        data: Bytes::new(),
    };

    // Log 2: L1Message from Messenger (0xfffe) -- all params indexed
    // data_hash = keccak256(abi.encodePacked(ETH_TOKEN, ETH_TOKEN, receiverOnL1, value))
    let data_hash = compute_withdrawal_data_hash(receiver_on_l1, value);
    let log2 = Log {
        address: L2_TO_L1_MESSENGER_ADDRESS,
        topics: vec![
            *L1MESSAGE_TOPIC,
            addr_to_h256(COMMON_BRIDGE_L2_ADDRESS),
            data_hash,
            u256_to_h256(message_id),
        ],
        data: Bytes::new(),
    };

    vec![log1, log2]
}

/// Convert an address to H256 (left-padded with zeros).
fn addr_to_h256(addr: Address) -> H256 {
    let mut bytes = [0u8; 32];
    bytes[12..32].copy_from_slice(addr.as_bytes());
    H256(bytes)
}

/// Convert U256 to H256 (big-endian).
fn u256_to_h256(value: U256) -> H256 {
    H256(value.to_big_endian())
}

/// Compute the withdrawal data hash used in the L1Message event.
///
/// `keccak256(abi.encodePacked(ETH_TOKEN_ADDRESS, ETH_TOKEN_ADDRESS, receiverOnL1, value))`
///
/// This matches the Solidity: `keccak256(abi.encodePacked(tokenOnL2, tokenOnL1, receiver, amount))`
/// where for ETH withdrawals both tokens are ETH_TOKEN_ADDRESS.
fn compute_withdrawal_data_hash(receiver: Address, value: U256) -> H256 {
    // abi.encodePacked produces tightly packed bytes:
    //   20 bytes (ETH_TOKEN) + 20 bytes (ETH_TOKEN) + 20 bytes (receiver) + 32 bytes (value)
    let mut packed = Vec::with_capacity(92);
    packed.extend_from_slice(ETH_TOKEN_ADDRESS.as_bytes()); // 20 bytes
    packed.extend_from_slice(ETH_TOKEN_ADDRESS.as_bytes()); // 20 bytes
    packed.extend_from_slice(receiver.as_bytes()); // 20 bytes
    packed.extend_from_slice(&value.to_big_endian()); // 32 bytes

    H256::from(keccak_hash(&packed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::H160;
    use ethrex_common::types::TxKind;
    use ethrex_common::types::transaction::EIP1559Transaction;

    use crate::common::app_state::AppState;
    use crate::common::app_types::{AccountProof, StorageProof};

    // ── Helpers ──────────────────────────────────────────────

    /// The real `withdraw(address)` selector: keccak256("withdraw(address)")[0..4]
    fn withdraw_selector() -> [u8; 4] {
        let hash = keccak_hash(b"withdraw(address)");
        [hash[0], hash[1], hash[2], hash[3]]
    }

    /// Build `withdraw(address _receiverOnL1)` calldata matching the EVM ABI encoding.
    fn build_withdraw_calldata(receiver: Address) -> Bytes {
        let mut data = Vec::with_capacity(36);
        data.extend_from_slice(&withdraw_selector());
        data.extend_from_slice(&[0u8; 12]); // left-pad to 32 bytes
        data.extend_from_slice(receiver.as_bytes());
        Bytes::from(data)
    }

    /// Build `mintETH(address to)` calldata (selector 0xb0f4d395).
    fn build_mint_eth_calldata(recipient: Address) -> Bytes {
        let mut data = Vec::with_capacity(36);
        data.extend_from_slice(&[0xb0, 0xf4, 0xd3, 0x95]); // mintETH selector
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(recipient.as_bytes());
        Bytes::from(data)
    }

    /// Create an EIP-1559 withdrawal TX: to = bridge(0xffff), value = amount, data = withdraw(receiver)
    fn make_withdrawal_tx(receiver: Address, value: U256, sender_addr: Address) -> Transaction {
        let tx = EIP1559Transaction {
            chain_id: 1,
            nonce: 0,
            max_priority_fee_per_gas: 2_000_000_000,
            max_fee_per_gas: 10_000_000_000,
            gas_limit: 200_000,
            to: TxKind::Call(COMMON_BRIDGE_L2_ADDRESS),
            value,
            data: build_withdraw_calldata(receiver),
            ..Default::default()
        };
        let _ = tx.sender_cache.set(sender_addr);
        Transaction::EIP1559Transaction(tx)
    }

    /// Build an AppState with the given accounts and storage slots.
    fn make_state(
        accounts: Vec<(Address, u64, U256)>,
        storage: Vec<(Address, H256, U256)>,
    ) -> AppState {
        let account_proofs: Vec<AccountProof> = accounts
            .iter()
            .map(|(addr, nonce, balance)| AccountProof {
                address: *addr,
                nonce: *nonce,
                balance: *balance,
                storage_root: H256::zero(),
                code_hash: H256::zero(),
                proof: vec![],
            })
            .collect();
        let storage_proofs: Vec<StorageProof> = storage
            .iter()
            .map(|(addr, slot, value)| StorageProof {
                address: *addr,
                slot: *slot,
                value: *value,
                account_proof: vec![],
                storage_proof: vec![],
            })
            .collect();
        AppState::from_proofs(H256::zero(), account_proofs, storage_proofs)
    }

    // ── Withdrawal state transition tests ────────────────────

    #[test]
    fn withdrawal_debits_sender_and_credits_burn_address() {
        let sender = H160([0xAA; 20]);
        let receiver = H160([0xBB; 20]);
        let five_eth = U256::from(5_000_000_000_000_000_000u64);

        let mut state = make_state(
            vec![
                (sender, 0, five_eth * 2),
                (BURN_ADDRESS, 0, U256::zero()),
                (L2_TO_L1_MESSENGER_ADDRESS, 0, U256::zero()),
            ],
            vec![(
                L2_TO_L1_MESSENGER_ADDRESS,
                MESSENGER_LAST_MESSAGE_ID_SLOT,
                U256::zero(),
            )],
        );

        let tx = make_withdrawal_tx(receiver, five_eth, sender);
        let (gas, msg_id) = handle_withdrawal(&mut state, &tx, sender).unwrap();

        assert_eq!(state.get_balance(sender).unwrap(), five_eth);
        assert_eq!(state.get_balance(BURN_ADDRESS).unwrap(), five_eth);
        assert_eq!(msg_id, U256::one());
        assert_eq!(
            state
                .get_storage(L2_TO_L1_MESSENGER_ADDRESS, MESSENGER_LAST_MESSAGE_ID_SLOT)
                .unwrap(),
            U256::one()
        );
        assert_eq!(gas, WITHDRAWAL_GAS);
    }

    #[test]
    fn withdrawal_increments_message_id_sequentially() {
        let sender = H160([0xAA; 20]);
        let receiver = H160([0xBB; 20]);
        let one_eth = U256::from(1_000_000_000_000_000_000u64);

        let mut state = make_state(
            vec![
                (sender, 0, one_eth * 10),
                (BURN_ADDRESS, 0, U256::zero()),
                (L2_TO_L1_MESSENGER_ADDRESS, 0, U256::zero()),
            ],
            vec![(
                L2_TO_L1_MESSENGER_ADDRESS,
                MESSENGER_LAST_MESSAGE_ID_SLOT,
                U256::from(5),
            )],
        );

        let tx = make_withdrawal_tx(receiver, one_eth, sender);
        let (_, msg_id_1) = handle_withdrawal(&mut state, &tx, sender).unwrap();
        assert_eq!(msg_id_1, U256::from(6));

        let tx2 = make_withdrawal_tx(receiver, one_eth, sender);
        let (_, msg_id_2) = handle_withdrawal(&mut state, &tx2, sender).unwrap();
        assert_eq!(msg_id_2, U256::from(7));
    }

    #[test]
    fn withdrawal_insufficient_balance_fails() {
        let sender = H160([0xAA; 20]);
        let receiver = H160([0xBB; 20]);
        let one_eth = U256::from(1_000_000_000_000_000_000u64);

        let mut state = make_state(
            vec![
                (sender, 0, U256::from(100)),
                (BURN_ADDRESS, 0, U256::zero()),
                (L2_TO_L1_MESSENGER_ADDRESS, 0, U256::zero()),
            ],
            vec![(
                L2_TO_L1_MESSENGER_ADDRESS,
                MESSENGER_LAST_MESSAGE_ID_SLOT,
                U256::zero(),
            )],
        );

        let tx = make_withdrawal_tx(receiver, one_eth, sender);
        assert!(handle_withdrawal(&mut state, &tx, sender).is_err());
    }

    // ── Withdrawal log tests ─────────────────────────────────

    #[test]
    fn generate_withdrawal_logs_produces_two_logs() {
        let sender = H160([0xAA; 20]);
        let receiver = H160([0xBB; 20]);
        let five_eth = U256::from(5_000_000_000_000_000_000u64);
        let message_id = U256::from(1);

        let tx = make_withdrawal_tx(receiver, five_eth, sender);
        let logs = generate_withdrawal_logs(sender, &tx, message_id);

        assert_eq!(logs.len(), 2, "Must produce exactly 2 logs");
    }

    #[test]
    fn log1_withdrawal_initiated_matches_evm() {
        let sender = H160([0xAA; 20]);
        let receiver = H160([0xBB; 20]);
        let five_eth = U256::from(5_000_000_000_000_000_000u64);
        let message_id = U256::from(1);

        let tx = make_withdrawal_tx(receiver, five_eth, sender);
        let logs = generate_withdrawal_logs(sender, &tx, message_id);
        let log1 = &logs[0];

        assert_eq!(log1.address, COMMON_BRIDGE_L2_ADDRESS);

        let expected_selector =
            H256::from(keccak_hash(b"WithdrawalInitiated(address,address,uint256)"));
        assert_eq!(log1.topics[0], expected_selector);
        assert_eq!(log1.topics[0], *WITHDRAWAL_INITIATED_TOPIC);

        let mut expected_sender = [0u8; 32];
        expected_sender[12..32].copy_from_slice(&[0xAA; 20]);
        assert_eq!(log1.topics[1], H256(expected_sender));

        let mut expected_receiver = [0u8; 32];
        expected_receiver[12..32].copy_from_slice(&[0xBB; 20]);
        assert_eq!(log1.topics[2], H256(expected_receiver));

        assert_eq!(log1.topics[3], H256(five_eth.to_big_endian()));
        assert!(log1.data.is_empty());
        assert_eq!(log1.topics.len(), 4);
    }

    #[test]
    fn log2_l1message_matches_evm_and_get_block_l1_messages() {
        let sender = H160([0xAA; 20]);
        let receiver = H160([
            0xf9, 0x3e, 0xe4, 0xcf, 0x8c, 0x6c, 0x40, 0xb3, 0x29, 0xb0, 0xc0, 0x62, 0x6f, 0x28,
            0x33, 0x3c, 0x13, 0x2c, 0xf2, 0x41,
        ]);
        let five_eth = U256::from(5_000_000_000_000_000_000u64);
        let message_id = U256::from(1);

        let tx = make_withdrawal_tx(receiver, five_eth, sender);
        let logs = generate_withdrawal_logs(sender, &tx, message_id);
        let log2 = &logs[1];

        assert_eq!(log2.address, L2_TO_L1_MESSENGER_ADDRESS);

        let expected_selector = H256::from(keccak_hash(b"L1Message(address,bytes32,uint256)"));
        assert_eq!(log2.topics[0], expected_selector);
        assert_eq!(log2.topics[0], *L1MESSAGE_TOPIC);

        let mut expected_from = [0u8; 32];
        expected_from[12..32].copy_from_slice(COMMON_BRIDGE_L2_ADDRESS.as_bytes());
        assert_eq!(log2.topics[1], H256(expected_from));

        let mut packed = Vec::with_capacity(92);
        packed.extend_from_slice(ETH_TOKEN_ADDRESS.as_bytes());
        packed.extend_from_slice(ETH_TOKEN_ADDRESS.as_bytes());
        packed.extend_from_slice(receiver.as_bytes());
        packed.extend_from_slice(&five_eth.to_big_endian());
        let expected_data_hash = H256::from(keccak_hash(&packed));
        assert_eq!(log2.topics[2], expected_data_hash);

        assert_eq!(log2.topics[3], H256(message_id.to_big_endian()));
        assert!(log2.data.is_empty());
        assert_eq!(log2.topics.len(), 4);
    }

    #[test]
    fn generated_logs_are_parseable_by_get_block_l1_messages() {
        use ethrex_common::types::Receipt;
        use ethrex_l2_common::messages::get_block_l1_messages;

        let sender = H160([0xAA; 20]);
        let receiver = H160([
            0xf9, 0x3e, 0xe4, 0xcf, 0x8c, 0x6c, 0x40, 0xb3, 0x29, 0xb0, 0xc0, 0x62, 0x6f, 0x28,
            0x33, 0x3c, 0x13, 0x2c, 0xf2, 0x41,
        ]);
        let five_eth = U256::from(5_000_000_000_000_000_000u64);
        let message_id = U256::from(1);

        let tx = make_withdrawal_tx(receiver, five_eth, sender);
        let logs = generate_withdrawal_logs(sender, &tx, message_id);

        let receipt = Receipt {
            tx_type: ethrex_common::types::TxType::EIP1559,
            succeeded: true,
            cumulative_gas_used: 100_000,
            logs,
        };

        let l1_messages = get_block_l1_messages(&[receipt]);

        assert_eq!(l1_messages.len(), 1, "Must extract exactly 1 L1Message");

        let msg = &l1_messages[0];
        assert_eq!(msg.from, COMMON_BRIDGE_L2_ADDRESS);

        let mut packed = Vec::with_capacity(92);
        packed.extend_from_slice(ETH_TOKEN_ADDRESS.as_bytes());
        packed.extend_from_slice(ETH_TOKEN_ADDRESS.as_bytes());
        packed.extend_from_slice(receiver.as_bytes());
        packed.extend_from_slice(&five_eth.to_big_endian());
        let expected_data_hash = H256::from(keccak_hash(&packed));
        assert_eq!(msg.data_hash, expected_data_hash);
        assert_eq!(msg.message_id, message_id);
    }

    // ── Deposit (privileged tx) calldata test ────────────────

    #[test]
    fn deposit_mint_eth_calldata_correctly_parsed() {
        use crate::common::handlers::deposit::handle_privileged_tx;

        let real_recipient = H160([
            0xf9, 0x3e, 0xe4, 0xcf, 0x8c, 0x6c, 0x40, 0xb3, 0x29, 0xb0, 0xc0, 0x62, 0x6f, 0x28,
            0x33, 0x3c, 0x13, 0x2c, 0xf2, 0x41,
        ]);
        let ten_eth = U256::from(10) * U256::from(1_000_000_000_000_000_000u64);

        let tx = Transaction::PrivilegedL2Transaction(
            ethrex_common::types::transaction::PrivilegedL2Transaction {
                chain_id: 1,
                nonce: 0,
                max_priority_fee_per_gas: 0,
                max_fee_per_gas: 0,
                gas_limit: 21000,
                to: TxKind::Call(COMMON_BRIDGE_L2_ADDRESS),
                value: ten_eth,
                data: build_mint_eth_calldata(real_recipient),
                from: COMMON_BRIDGE_L2_ADDRESS,
                ..Default::default()
            },
        );

        let mut state = make_state(
            vec![
                (COMMON_BRIDGE_L2_ADDRESS, 0, U256::zero()),
                (real_recipient, 0, U256::zero()),
            ],
            vec![],
        );

        handle_privileged_tx(&mut state, &tx).unwrap();

        assert_eq!(state.get_balance(real_recipient).unwrap(), ten_eth);
        assert_eq!(
            state.get_balance(COMMON_BRIDGE_L2_ADDRESS).unwrap(),
            U256::zero()
        );
    }
}
