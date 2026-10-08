//! The keyless deployment transactions of the frame-transaction contracts.
//!
//! EIP-8141 (expiry verifier), EIP-8250 (nonce manager) and EIP-8272 (recent root
//! contract) each deploy an ordinary contract with a pre-EIP-155 transaction whose
//! signature was chosen first, so its sender has no known key and the transaction
//! is the only one that sender can ever send. Each EIP publishes that transaction,
//! its sender and the resulting address. These tests check the published data
//! against itself and against the constants ethrex uses: the transaction hashes to
//! the published hash, recovers the published sender, creates the contract at the
//! address ethrex treats as canonical, and leaves exactly the runtime code ethrex
//! matches against.

use bytes::Bytes;
use ethrex_blockchain::vm::StoreVmDatabase;
use ethrex_common::evm::calculate_create_address;
use ethrex_common::types::{
    Account, BlockHeader, Code, Fork, Frame, FrameMode, FrameTransaction, LegacyTransaction,
    Transaction, TxKind, frame_tx_expiry_verifier, frame_tx_nonce_manager, frame_tx_recent_root,
};
use ethrex_common::{Address, H256, U256, constants::EMPTY_TRIE_HASH};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::db::gen_db::GeneralizedDatabase;
use ethrex_levm::environment::{EVMConfig, Environment};
use ethrex_levm::tracing::LevmCallTracer;
use ethrex_levm::vm::{VM, VMType};
use ethrex_storage::Store;
use ethrex_vm::DynVmDatabase;
use ethrex_vm::system_contracts::{
    EXPIRY_VERIFIER_RUNTIME_BYTECODE, NONCE_MANAGER_RUNTIME_BYTECODE, RECENT_ROOT_RUNTIME_BYTECODE,
};
use rustc_hash::FxHashMap;
use std::str::FromStr;
use std::sync::Arc;

/// A deployment transaction as its EIP publishes it. Every one is a legacy contract
/// creation at nonce 0 with value 0, `gasPrice` 1000 gwei, `v` 27 and `r` 0x539.
struct Deployment {
    gas: u64,
    input: &'static str,
    s: &'static str,
    hash: &'static str,
    sender: &'static str,
    address: Address,
    runtime: &'static [u8],
}

const GAS_PRICE: u64 = 1_000_000_000_000;

fn expiry_verifier() -> Deployment {
    Deployment {
        gas: 0x3d090,
        input: "601a8060095f395ff360083614600a575f5ffd5b5f3560c01c4211601657005b5f5ffd",
        s: "2332f8a31e207466",
        hash: "f68c75d16846350145dec76cd1885236dabbf8e06044073aaf11a0a125a9f14a",
        sender: "E514C70E18d981A9A72E83C30db9c13e05a50b97",
        address: frame_tx_expiry_verifier(),
        runtime: &EXPIRY_VERIFIER_RUNTIME_BYTECODE,
    }
}

fn nonce_manager() -> Deployment {
    Deployment {
        gas: 0x3d090,
        input: "60058060095f395ff360006000fd",
        s: "825000000000000141de",
        hash: "5fb73d939ec0b319de03454f0e787a57543d7edcf889a1a8e3fd0473f8fb6c1f",
        sender: "7A57307E9985b0e26B66A7BB920BcaF0396b8198",
        address: frame_tx_nonce_manager(),
        runtime: &NONCE_MANAGER_RUNTIME_BYTECODE,
    }
}

/// The EIP-8272 deployment input: a 10-byte copier and the 320-byte runtime.
const RECENT_ROOT_INPUT: &str = concat!(
    "61014080600a5f395ff3346100ba57366040146100c05736604836066100ba5780156100ba57610480811161",
    "00ba574b60005b602081013560c01c828110156100ba5780830361200011156100ba577f8f42481679c8e6fe",
    "fa040974b3c905e0ce3f2e464ba93acdb074a41181617efc60005260488260203760686000207fbdc897da21",
    "77d260ff5f4be5d4b2aad43f89c3347a305b584fa5a2546d053daa60005290611fff1660c01b604052604860",
    "00205414156100ba5760480182811061002857005b60006000fd5b33600052602060006020376034600c2080",
    "7f8f42481679c8e6fefa040974b3c905e0ce3f2e464ba93acdb074a41181617efc6040524b60685260605260",
    "2060206088376068604020817fbdc897da2177d260ff5f4be5d4b2aad43f89c3347a305b584fa5a2546d053d",
    "aa60a852611fff4b1660d05260c852604860a8205500",
);

fn recent_root() -> Deployment {
    Deployment {
        gas: 0x13d620,
        input: RECENT_ROOT_INPUT,
        s: "fadf66b1e192785c",
        hash: "56c2adbbfa3ba5dfe47adf48828f6c581134e39e14c3b80e06214de5e5875272",
        sender: "14bf16d4c9842bf1EbF396e553477C66EB0a8A82",
        address: frame_tx_recent_root(),
        runtime: &RECENT_ROOT_RUNTIME_BYTECODE,
    }
}

fn transaction(deployment: &Deployment) -> Transaction {
    Transaction::LegacyTransaction(LegacyTransaction {
        nonce: 0,
        gas_price: U256::from(GAS_PRICE),
        gas: deployment.gas,
        to: TxKind::Create,
        value: U256::zero(),
        data: Bytes::from(hex::decode(deployment.input).unwrap()),
        v: U256::from(27),
        r: U256::from(0x539),
        s: U256::from_str_radix(deployment.s, 16).unwrap(),
        ..Default::default()
    })
}

/// Executes the deployment transaction on a pre-fork chain whose only account is its
/// funded sender, and returns the code it leaves at the created address.
fn deployed_code(deployment: &Deployment, sender: Address) -> Bytes {
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());
    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    cache.insert(
        sender,
        Account::new(
            U256::from(deployment.gas) * U256::from(GAS_PRICE),
            Code::from_bytecode(Bytes::new(), &NativeCrypto),
            0,
            FxHashMap::default(),
        ),
    );
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(store), cache);
    let env = Environment {
        origin: sender,
        gas_limit: deployment.gas,
        block_gas_limit: 30_000_000,
        config: EVMConfig::new(Fork::Osaka, EVMConfig::canonical_values(Fork::Osaka)),
        gas_price: U256::from(GAS_PRICE),
        ..Default::default()
    };
    let tx = transaction(deployment);
    let report = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new succeeds for the deployment transaction")
    .execute()
    .expect("the deployment transaction is valid");
    assert!(
        report.is_success(),
        "the deployment must succeed: {report:?}"
    );
    db.get_account_code(deployment.address)
        .expect("the created account is readable")
        .code_bytes()
}

fn check(deployment: Deployment) {
    let tx = transaction(&deployment);
    assert_eq!(
        tx.hash(&NativeCrypto),
        H256::from_str(deployment.hash).unwrap(),
        "the transaction must hash to the published hash"
    );
    let sender = tx.sender(&NativeCrypto).expect("the signature recovers");
    assert_eq!(sender, Address::from_str(deployment.sender).unwrap());
    assert_eq!(
        calculate_create_address(sender, 0),
        deployment.address,
        "ethrex's address must be the one the deployment creates"
    );
    assert_eq!(
        deployed_code(&deployment, sender).as_ref(),
        deployment.runtime,
        "the deployment must leave exactly the runtime code ethrex matches against"
    );
}

#[test]
fn the_expiry_verifier_deployment_matches_eip_8141() {
    check(expiry_verifier());
}

#[test]
fn the_nonce_manager_deployment_matches_eip_8250() {
    check(nonce_manager());
}

#[test]
fn the_recent_root_deployment_matches_eip_8272() {
    check(recent_root());
}

// ==================== Before the deployment ====================
//
// Activation installs nothing, so a chain can activate the fork before the expiry
// verifier or the recent root contract exists. Until then the address has no code,
// a frame aimed at it runs the protocol default code, and in a VERIFY frame with
// flags 0 that code reverts, so the transaction is invalid.

/// A contract sender whose runtime is `APPROVE(3)`.
const SENDER: Address = Address::repeat_byte(0xAA);
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA];

fn frame_tx_result(
    deployed: Option<(Address, &'static [u8])>,
    leading_frame: Frame,
) -> Result<ethrex_levm::errors::ExecutionReport, ethrex_levm::errors::VMError> {
    let in_memory_db = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
    let header = BlockHeader {
        state_root: *EMPTY_TRIE_HASH,
        ..Default::default()
    };
    let store: DynVmDatabase = Box::new(StoreVmDatabase::new(in_memory_db, header).unwrap());
    let mut cache: FxHashMap<Address, Account> = FxHashMap::default();
    cache.insert(
        SENDER,
        Account::new(
            U256::from(10u64).pow(U256::from(18u64)),
            Code::from_bytecode(Bytes::from_static(APPROVE_BOTH_CODE), &NativeCrypto),
            0,
            FxHashMap::default(),
        ),
    );
    if let Some((address, runtime)) = deployed {
        cache.insert(
            address,
            Account::new(
                U256::zero(),
                Code::from_bytecode(Bytes::from_static(runtime), &NativeCrypto),
                1,
                FxHashMap::default(),
            ),
        );
    }
    let mut db = GeneralizedDatabase::new_with_account_state(Arc::new(store), cache);
    let tx = FrameTransaction {
        chain_id: 1,
        nonce_keys: vec![U256::zero()],
        nonce_seq: 0,
        sender: SENDER,
        frames: vec![
            leading_frame,
            Frame {
                mode: u8::from(FrameMode::Verify),
                flags: 0x03,
                target: Some(SENDER),
                gas_limit: 100_000,
                state_gas_limit: 0,
                value: U256::zero(),
                data: Bytes::new(),
            },
        ],
        max_priority_fee_per_gas: U256::from(1),
        max_fee_per_gas: U256::from(1_000),
        ..Default::default()
    };
    let env = Environment {
        origin: SENDER,
        gas_limit: tx.max_gas(),
        block_gas_limit: (i64::MAX - 1) as u64,
        config: EVMConfig::new(Fork::Hegota, EVMConfig::canonical_values(Fork::Hegota)),
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(1),
        gas_price: tx.max_fee_per_gas,
        slot_number: U256::from(10),
        ..Default::default()
    };
    let transaction = Transaction::FrameTransaction(tx);
    VM::new(
        env,
        &mut db,
        &transaction,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .expect("VM::new succeeds for a frame tx")
    .execute()
}

fn verify_frame_to(target: Address, data: Vec<u8>) -> Frame {
    Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0,
        target: Some(target),
        gas_limit: 100_000,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::from(data),
    }
}

fn is_failed_verify_frame(
    result: &Result<ethrex_levm::errors::ExecutionReport, ethrex_levm::errors::VMError>,
) -> bool {
    matches!(
        result,
        Err(ethrex_levm::errors::VMError::TxValidation(
            ethrex_levm::errors::TxValidationError::InvalidFrameTransaction
        ))
    )
}

#[test]
fn an_expiry_frame_fails_until_the_verifier_is_deployed() {
    let expiry = || verify_frame_to(frame_tx_expiry_verifier(), u64::MAX.to_be_bytes().to_vec());
    let before = frame_tx_result(None, expiry());
    assert!(is_failed_verify_frame(&before), "got {before:?}");
    let after = frame_tx_result(
        Some((
            frame_tx_expiry_verifier(),
            &EXPIRY_VERIFIER_RUNTIME_BYTECODE,
        )),
        expiry(),
    );
    assert!(
        after.is_ok(),
        "a far deadline passes once deployed, got {after:?}"
    );
}

#[test]
fn a_recent_root_verifier_frame_fails_until_the_contract_is_deployed() {
    // One well-formed 72-byte tuple; without the contract its content is irrelevant.
    let result = frame_tx_result(
        None,
        verify_frame_to(frame_tx_recent_root(), vec![0x11; 72]),
    );
    assert!(is_failed_verify_frame(&result), "got {result:?}");
}
