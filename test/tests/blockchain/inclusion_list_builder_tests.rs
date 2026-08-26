use std::cell::RefCell;
use std::collections::HashMap;

use ethrex_blockchain::focil_eligibility::SenderCode;
use ethrex_blockchain::inclusion_list_builder::{
    AccountStateView, DEFAULT_PER_SENDER_CAP, IlPolicy, IlStateProvider, IlStateProviderError,
    InclusionListBuilder, MAX_BYTES_PER_INCLUSION_LIST,
};
use ethrex_blockchain::mempool::{KeyedConcurrency, Mempool};
use ethrex_common::types::{
    APPROVE_EXECUTION_AND_PAYMENT, EIP1559Transaction, EIP4844Transaction,
    FRAME_SIG_SCHEME_SECP256K1, Frame, FrameEncoding, FrameLimits, FrameMode, FrameSignature,
    FrameTransaction, LegacyTransaction, MempoolTransaction, PrivilegedL2Transaction, Transaction,
    TxKind, utxo_vault,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;

/// In-memory state provider for unit tests. `codes` is keyed by `code_hash`;
/// none of these builder tests exercise sender-code classification, so
/// `classify_code` defaults to `Eoa` for any hash not explicitly registered.
#[derive(Default)]
struct FakeState {
    accounts: RefCell<HashMap<Address, AccountStateView>>,
    codes: RefCell<HashMap<H256, SenderCode>>,
}

impl FakeState {
    fn set(&self, addr: Address, nonce: u64, balance: U256) {
        self.accounts.borrow_mut().insert(
            addr,
            AccountStateView {
                nonce,
                balance,
                ..Default::default()
            },
        );
    }
}

impl IlStateProvider for FakeState {
    fn get_account(
        &self,
        address: Address,
    ) -> Result<Option<AccountStateView>, IlStateProviderError> {
        Ok(self.accounts.borrow().get(&address).copied())
    }

    fn classify_code(&self, code_hash: H256) -> Result<SenderCode, IlStateProviderError> {
        Ok(self
            .codes
            .borrow()
            .get(&code_hash)
            .copied()
            .unwrap_or(SenderCode::Eoa))
    }
}

fn addr(byte: u8) -> Address {
    let mut a = [0u8; 20];
    a[19] = byte;
    Address::from(a)
}

fn legacy_tx(nonce: u64, gas_price: u64, gas_limit: u64, value: u64) -> Transaction {
    Transaction::LegacyTransaction(LegacyTransaction {
        nonce,
        gas_price: U256::from(gas_price),
        gas: gas_limit,
        to: TxKind::Call(addr(0xff)),
        value: U256::from(value),
        v: U256::from(27),
        r: U256::from(1),
        s: U256::from(1),
        ..Default::default()
    })
}

fn eip1559_tx(
    nonce: u64,
    max_fee: u64,
    max_priority: u64,
    gas_limit: u64,
    value: u64,
) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce,
        max_priority_fee_per_gas: max_priority,
        max_fee_per_gas: max_fee,
        gas_limit,
        to: TxKind::Call(addr(0xff)),
        value: U256::from(value),
        signature_r: U256::from(1),
        signature_s: U256::from(1),
        ..Default::default()
    })
}

fn blob_tx(nonce: u64, max_fee: u64, gas_limit: u64) -> Transaction {
    Transaction::EIP4844Transaction(EIP4844Transaction {
        chain_id: 1,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: max_fee,
        gas: gas_limit,
        to: addr(0xff),
        max_fee_per_blob_gas: U256::from(1),
        signature_r: U256::from(1),
        signature_s: U256::from(1),
        ..Default::default()
    })
}

fn privileged_tx(nonce: u64) -> Transaction {
    Transaction::PrivilegedL2Transaction(PrivilegedL2Transaction {
        chain_id: 1,
        nonce,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1,
        gas_limit: 21_000,
        to: TxKind::Call(addr(0xff)),
        from: addr(0x01),
        ..Default::default()
    })
}

/// A frame transaction carrying `nonce_keys`/`nonce_seq`. `[0]` is the linear
/// domain (`nonce_seq` is the account nonce); anything else is an EIP-8250 keyed
/// sequence tracked by the NONCE_MANAGER predeploy.
fn keyed_frame_tx(nonce_keys: Vec<U256>, nonce_seq: u64) -> Transaction {
    Transaction::FrameTransaction(FrameTransaction {
        chain_id: 1,
        nonce_keys,
        nonce_seq,
        sender: addr(0x01),
        frames: vec![Frame {
            mode: FrameMode::Verify as u8,
            flags: APPROVE_EXECUTION_AND_PAYMENT,
            target: Some(addr(0x01)),
            limits: FrameLimits {
                execution: 21_000,
                state: 21_000,
            },
            value: U256::zero(),
            data: Default::default(),
            encoding: FrameEncoding::Limits,
        }],
        signatures: vec![FrameSignature {
            scheme: FRAME_SIG_SCHEME_SECP256K1,
            signer: Some(addr(0x01)),
            msg: Default::default(),
            signature: Default::default(),
        }],
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 1_000,
        ..Default::default()
    })
}

/// An EIP-8312 UTXO spend: sender is the shared vault address, so it carries no
/// per-sender identity at all.
fn vault_frame_tx(nonce_keys: Vec<U256>) -> Transaction {
    let Transaction::FrameTransaction(mut tx) = keyed_frame_tx(nonce_keys, 0) else {
        unreachable!()
    };
    tx.sender = utxo_vault();
    Transaction::FrameTransaction(tx)
}

fn insert_tx(mempool: &Mempool, sender: Address, tx: Transaction) -> H256 {
    insert_with_concurrency(mempool, sender, tx, KeyedConcurrency::Denied)
}

/// Insert an EIP-8250 keyed frame tx that the mempool has cleared for
/// per-`(sender, nonce_key)` gating, so a sender can hold several at once.
fn insert_keyed_tx(mempool: &Mempool, sender: Address, tx: Transaction) -> H256 {
    insert_with_concurrency(mempool, sender, tx, KeyedConcurrency::Allowed)
}

fn insert_with_concurrency(
    mempool: &Mempool,
    sender: Address,
    tx: Transaction,
    concurrency: KeyedConcurrency,
) -> H256 {
    let mtx = MempoolTransaction::new(tx, sender);
    let hash = mtx.transaction().hash(&NativeCrypto);
    mempool
        .add_transaction(hash, sender, mtx, None, None, concurrency)
        .expect("add_transaction");
    hash
}

/// Most callers want a wallet-balanced sender that can pay a few txs.
fn fund(state: &FakeState, sender: Address, nonce: u64) {
    state.set(sender, nonce, U256::from(u128::MAX));
}

#[test]
fn empty_mempool_returns_empty() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    let builder = InclusionListBuilder::default();
    let il = builder.build(&mempool, 0, &state);
    assert!(il.is_empty());
}

#[test]
fn production_policy_excludes_blob_txs() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    let sender = addr(0x01);
    fund(&state, sender, 0);

    let blob_hash = insert_tx(&mempool, sender, blob_tx(0, 1_000, 21_000));
    let plain_hash = insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, 0));

    let builder = InclusionListBuilder::default();
    let il = builder.build(&mempool, 0, &state);

    let hashes: Vec<H256> = il.iter().map(|tx| tx.hash(&NativeCrypto)).collect();
    assert!(
        !hashes.contains(&blob_hash),
        "blob tx must not appear in IL"
    );
    assert!(hashes.contains(&plain_hash), "plain tx should appear");
}

#[test]
fn privileged_l2_tx_excluded() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    let sender = addr(0x01);
    fund(&state, sender, 0);

    let priv_hash = insert_tx(&mempool, sender, privileged_tx(0));
    let plain_hash = insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, 0));

    let builder = InclusionListBuilder::default();
    let il = builder.build(&mempool, 0, &state);

    let hashes: Vec<H256> = il.iter().map(|tx| tx.hash(&NativeCrypto)).collect();
    assert!(!hashes.contains(&priv_hash));
    assert!(hashes.contains(&plain_hash));
}

#[test]
fn per_sender_cap_respected() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    let sender = addr(0x01);
    fund(&state, sender, 0);

    // 5 consecutive nonce txs, all valid.
    for nonce in 0..5u64 {
        insert_tx(&mempool, sender, legacy_tx(nonce, 1, 21_000, 0));
    }

    let builder = InclusionListBuilder::new(IlPolicy::Production, 2, MAX_BYTES_PER_INCLUSION_LIST);
    let il = builder.build(&mempool, 0, &state);

    assert_eq!(
        il.len(),
        2,
        "per-sender cap of 2 must produce exactly 2 txs from one sender"
    );
    let mut nonces: Vec<u64> = il.iter().map(|tx| tx.nonce()).collect();
    nonces.sort();
    assert_eq!(nonces, vec![0, 1], "cap must take ascending nonces");
}

#[test]
fn total_rlp_under_8192_bytes() {
    let mempool = Mempool::new(2048);
    let state = FakeState::default();

    // Many distinct senders, each contributing one legacy tx with a
    // unique `value` so hashes differ and the mempool actually stores
    // all of them. 200 ~110-byte txs is comfortably past the 8 KiB cap,
    // so the packer must clip the output.
    for i in 0..200u16 {
        // Use a distinct address per sender (16-bit space).
        let mut bytes = [0u8; 20];
        bytes[18] = (i >> 8) as u8;
        bytes[19] = (i & 0xff) as u8;
        // Skip the zero-address.
        if bytes == [0u8; 20] {
            continue;
        }
        let sender = Address::from(bytes);
        fund(&state, sender, 0);
        insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, u64::from(i) + 1));
    }

    let builder = InclusionListBuilder::default();
    let il = builder.build(&mempool, 0, &state);

    let total_bytes: usize = il.iter().map(|tx| tx.encode_canonical_to_vec().len()).sum();
    assert!(
        total_bytes <= MAX_BYTES_PER_INCLUSION_LIST,
        "total RLP {} exceeded {}",
        total_bytes,
        MAX_BYTES_PER_INCLUSION_LIST
    );
    // Sanity: at ~110 bytes per tx, 8 KiB / 110 ≈ 74 txs fit. The
    // builder should have packed many txs, not just a handful.
    assert!(
        il.len() >= 50,
        "expected packer to take many txs near the byte limit, got {}",
        il.len()
    );
}

#[test]
fn invalid_nonce_excluded() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    let sender = addr(0x01);
    // Account is at nonce 5 in parent state but mempool tx claims nonce 0.
    state.set(sender, 5, U256::from(u128::MAX));

    let stale_hash = insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, 0));

    let builder = InclusionListBuilder::default();
    let il = builder.build(&mempool, 0, &state);

    let hashes: Vec<H256> = il.iter().map(|tx| tx.hash(&NativeCrypto)).collect();
    assert!(
        !hashes.contains(&stale_hash),
        "tx with stale nonce must be excluded"
    );
}

#[test]
fn insufficient_balance_excluded() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    let sender = addr(0x01);
    // Sender has 0 balance — tx with non-zero gas cost can't pay.
    state.set(sender, 0, U256::zero());

    let broke_hash = insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, 0));

    let builder = InclusionListBuilder::default();
    let il = builder.build(&mempool, 0, &state);

    let hashes: Vec<H256> = il.iter().map(|tx| tx.hash(&NativeCrypto)).collect();
    assert!(!hashes.contains(&broke_hash));
}

#[test]
fn keyed_frame_tx_survives_a_mismatched_account_nonce() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    let sender = addr(0x01);
    // EIP-8250: `nonce_seq` counts within the `(sender, nonce_key)` domain, so a
    // sender whose account nonce has moved on says nothing about a keyed tx.
    state.set(sender, 7, U256::from(u128::MAX));

    let keyed_hash = insert_tx(&mempool, sender, keyed_frame_tx(vec![U256::from(9u64)], 0));

    let builder = InclusionListBuilder::default();
    let il = builder.build(&mempool, 0, &state);

    let hashes: Vec<H256> = il.iter().map(|tx| tx.hash(&NativeCrypto)).collect();
    assert!(
        hashes.contains(&keyed_hash),
        "a keyed frame tx must not be judged against the sender's linear nonce"
    );
}

#[test]
fn keyed_frame_txs_on_disjoint_keys_do_not_collapse() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    let sender = addr(0x01);
    fund(&state, sender, 0);

    // Two independent keyed sequences from one sender, both at seq 0. A linear
    // nonce walk keeps at most one of them. `KeyedConcurrency::Allowed` is what
    // the mempool grants a prefix that no sibling transaction can invalidate.
    let first = insert_keyed_tx(&mempool, sender, keyed_frame_tx(vec![U256::from(1u64)], 0));
    let second = insert_keyed_tx(&mempool, sender, keyed_frame_tx(vec![U256::from(2u64)], 0));

    let builder = InclusionListBuilder::default();
    let il = builder.build(&mempool, 0, &state);

    let hashes: Vec<H256> = il.iter().map(|tx| tx.hash(&NativeCrypto)).collect();
    assert!(hashes.contains(&first));
    assert!(hashes.contains(&second));
}

#[test]
fn vault_sender_spends_are_not_per_sender_capped() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    // EIP-8312: every UTXO spend shares the vault sender, so a per-sender cap
    // there would cap the whole network.
    let vault = utxo_vault();
    state.set(vault, 0, U256::zero());

    let mut hashes = Vec::new();
    for key in 1..=4u64 {
        hashes.push(insert_tx(
            &mempool,
            vault,
            vault_frame_tx(vec![U256::from(key)]),
        ));
    }

    let builder = InclusionListBuilder::new(IlPolicy::Production, 2, MAX_BYTES_PER_INCLUSION_LIST);
    let il = builder.build(&mempool, 0, &state);

    let built: Vec<H256> = il.iter().map(|tx| tx.hash(&NativeCrypto)).collect();
    for hash in &hashes {
        assert!(
            built.contains(hash),
            "a vault-sender spend must not be dropped by the per-sender cap"
        );
    }
}

#[test]
fn key_zero_frame_tx_still_walks_the_linear_nonce() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    let sender = addr(0x01);
    state.set(sender, 5, U256::from(u128::MAX));

    // `nonce_keys == [0]` is the linear domain: `nonce_seq` IS the account nonce.
    let stale = insert_tx(&mempool, sender, keyed_frame_tx(vec![U256::zero()], 0));

    let builder = InclusionListBuilder::default();
    let il = builder.build(&mempool, 0, &state);

    let hashes: Vec<H256> = il.iter().map(|tx| tx.hash(&NativeCrypto)).collect();
    assert!(
        !hashes.contains(&stale),
        "a key-0 frame tx below the account nonce can never be included"
    );
}

#[test]
fn frame_tx_is_not_balance_gated() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    let sender = addr(0x01);
    // A sponsored frame tx is paid by a paymaster resolved during the validation
    // prefix, so a broke sender is not evidence it cannot be included.
    state.set(sender, 0, U256::zero());

    let sponsored = insert_tx(&mempool, sender, keyed_frame_tx(vec![U256::zero()], 0));

    let builder = InclusionListBuilder::default();
    let il = builder.build(&mempool, 0, &state);

    let hashes: Vec<H256> = il.iter().map(|tx| tx.hash(&NativeCrypto)).collect();
    assert!(hashes.contains(&sponsored));
}

#[test]
fn priority_fee_policy_orders_by_fee() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    let sender_a = addr(0x01);
    let sender_b = addr(0x02);
    fund(&state, sender_a, 0);
    fund(&state, sender_b, 0);

    // sender_a: low tip; sender_b: high tip. With the priority-fee
    // policy, sender_b's tx should appear first.
    let low = insert_tx(&mempool, sender_a, eip1559_tx(0, 100, 1, 21_000, 0));
    let high = insert_tx(&mempool, sender_b, eip1559_tx(0, 100, 50, 21_000, 0));

    let builder = InclusionListBuilder::new(
        IlPolicy::PriorityFee,
        DEFAULT_PER_SENDER_CAP,
        MAX_BYTES_PER_INCLUSION_LIST,
    );
    let il = builder.build(&mempool, 0, &state);

    assert_eq!(il.len(), 2);
    assert_eq!(il[0].hash(&NativeCrypto), high, "highest tip first");
    assert_eq!(il[1].hash(&NativeCrypto), low);
}

#[test]
fn random_policy_terminates() {
    let mempool = Mempool::new(64);
    let state = FakeState::default();
    // Vary tx `value` per sender so each tx hashes differently and the
    // mempool stores distinct entries; otherwise hash-collision would
    // collapse all inserts onto one slot.
    for i in 0..10u8 {
        let sender = addr(i.saturating_add(1));
        fund(&state, sender, 0);
        insert_tx(&mempool, sender, legacy_tx(0, 1, 21_000, u64::from(i + 1)));
    }

    let builder = InclusionListBuilder::new(
        IlPolicy::Random,
        DEFAULT_PER_SENDER_CAP,
        MAX_BYTES_PER_INCLUSION_LIST,
    );
    let il = builder.build(&mempool, 0, &state);

    assert_eq!(
        il.len(),
        10,
        "random policy must include all eligible txs that fit"
    );
    let total_bytes: usize = il.iter().map(|tx| tx.encode_canonical_to_vec().len()).sum();
    assert!(total_bytes <= MAX_BYTES_PER_INCLUSION_LIST);
}
