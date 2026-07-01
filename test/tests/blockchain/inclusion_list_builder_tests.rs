use std::cell::RefCell;
use std::collections::HashMap;

use ethrex_blockchain::inclusion_list_builder::{
    AccountStateView, DEFAULT_PER_SENDER_CAP, IlPolicy, IlStateProvider, IlStateProviderError,
    InclusionListBuilder, MAX_BYTES_PER_INCLUSION_LIST,
};
use ethrex_blockchain::mempool::Mempool;
use ethrex_common::types::{
    EIP1559Transaction, EIP4844Transaction, LegacyTransaction, MempoolTransaction,
    PrivilegedL2Transaction, Transaction, TxKind,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;

/// In-memory state provider for unit tests.
#[derive(Default)]
struct FakeState {
    accounts: RefCell<HashMap<Address, AccountStateView>>,
}

impl FakeState {
    fn set(&self, addr: Address, nonce: u64, balance: U256) {
        self.accounts
            .borrow_mut()
            .insert(addr, AccountStateView { nonce, balance });
    }
}

impl IlStateProvider for FakeState {
    fn get_account(
        &self,
        address: Address,
    ) -> Result<Option<AccountStateView>, IlStateProviderError> {
        Ok(self.accounts.borrow().get(&address).copied())
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

fn insert_tx(mempool: &Mempool, sender: Address, tx: Transaction) -> H256 {
    let mtx = MempoolTransaction::new(tx, sender);
    let hash = mtx.transaction().hash(&NativeCrypto);
    mempool
        .add_transaction(hash, sender, mtx)
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
