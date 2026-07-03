//! eth_call / eth_estimateGas environment construction.
//!
//! Simulation must never be stricter than execution: a header without a
//! `slot_number` imports and executes fine (the execution env defaults the
//! slot to zero), so simulating on top of it must work the same way. Headers
//! legitimately lack the slot on Amsterdam+ chains whose consensus client
//! drives a pre-V4 engine API (the slot only arrives in
//! `PayloadAttributesV4`) — including ethrex's own `--dev` mode, which
//! produces blocks via `engine_forkchoiceUpdatedV3`.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::types::{Account, BlockHeader, ChainConfig, Code, GenericTransaction};
use ethrex_common::{Address, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::vm::VMType;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::DynVmDatabase;
use ethrex_vm::backends::levm::LEVM;
use rustc_hash::FxHashMap;
use std::sync::Arc;

const SENDER: Address = Address::repeat_byte(0xAA);

fn amsterdam_db_and_header(slot_number: Option<u64>) -> (GeneralizedDatabase, BlockHeader) {
    let mut store = Store::new("", EngineType::InMemory).unwrap();
    let mut chain_config = ChainConfig::default();
    // Activate everything up to Amsterdam from genesis.
    chain_config.shanghai_time = Some(0);
    chain_config.cancun_time = Some(0);
    chain_config.prague_time = Some(0);
    chain_config.osaka_time = Some(0);
    chain_config.amsterdam_time = Some(0);
    // Set the config on the SAME store the VM reads (the field is a per-Store
    // in-memory copy, so setting it on a clone would not propagate).
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async {
            store.set_chain_config(&chain_config).await.unwrap();
        });

    let header = BlockHeader {
        number: 1,
        timestamp: 1,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(7),
        state_root: *EMPTY_TRIE_HASH,
        slot_number,
        ..Default::default()
    };

    let vm_db: DynVmDatabase = Box::new(StoreVmDatabase::new(store, header.clone()).unwrap());
    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    cache.insert(
        SENDER,
        Account::new(
            U256::from(10u64).pow(U256::from(18u64)),
            Code::from_bytecode(Bytes::new(), &NativeCrypto),
            0,
            FxHashMap::default(),
        ),
    );
    (
        GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache),
        header,
    )
}

fn plain_transfer() -> GenericTransaction {
    GenericTransaction {
        from: SENDER.into(),
        to: ethrex_common::types::TxKind::Call(Address::repeat_byte(0xBB)),
        value: U256::one(),
        ..Default::default()
    }
}

#[test]
fn simulation_tolerates_missing_slot_number_on_amsterdam() {
    // Regression: this used to fail with "slot_number must be present in
    // Amsterdam+ blocks", breaking every eth_call / eth_estimateGas on chains
    // whose headers carry no slot — while execution of the same blocks
    // succeeded (its env defaults the slot to zero).
    let (mut db, header) = amsterdam_db_and_header(None);
    let result =
        LEVM::simulate_tx_from_generic(&plain_transfer(), &header, &mut db, VMType::L1, &NativeCrypto);
    assert!(
        result.is_ok(),
        "simulation must tolerate a missing slot_number like execution does; got {result:?}"
    );
}

#[test]
fn simulation_uses_the_slot_number_when_present() {
    let (mut db, header) = amsterdam_db_and_header(Some(1234));
    let result =
        LEVM::simulate_tx_from_generic(&plain_transfer(), &header, &mut db, VMType::L1, &NativeCrypto);
    assert!(result.is_ok(), "got {result:?}");
}
