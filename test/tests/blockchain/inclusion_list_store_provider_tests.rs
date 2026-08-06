//! `StoreIlStateProvider` (the only production `IlStateProvider`) against a
//! real `Store`, exercising the EIP-3607/EIP-7702 sender-code classification
//! path that Task 1.2's `AccountStateView::default` hazard lives in. Every
//! other IL test in this crate runs against in-memory fakes.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::focil_eligibility::SenderCode;
use ethrex_blockchain::inclusion_list_builder::IlStateProvider;
use ethrex_blockchain::inclusion_list_validator::StoreIlStateProvider;
use ethrex_common::constants::EMPTY_KECCAK_HASH;
use ethrex_common::types::{Genesis, GenesisAccount, code_hash};
use ethrex_common::{Address, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// A 23-byte EIP-7702 delegation indicator pointing at `target`.
fn delegation_indicator(target: Address) -> Bytes {
    let mut code = vec![0xef, 0x01, 0x00];
    code.extend_from_slice(target.as_bytes());
    Bytes::from(code)
}

#[tokio::test]
async fn store_backed_provider_classifies_sender_code_like_the_fakes() {
    let empty_code_addr = Address::repeat_byte(0x01);
    let delegated_addr = Address::repeat_byte(0x02);
    let contract_addr = Address::repeat_byte(0x03);
    // Same length as a delegation indicator (23 bytes) but the wrong
    // prefix — proves the length-based fast path does not, by itself,
    // misclassify a same-length contract as a delegation.
    let same_length_contract_addr = Address::repeat_byte(0x04);

    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: Genesis =
        serde_json::from_reader(reader).expect("failed to deserialize genesis file");

    genesis.alloc.insert(
        empty_code_addr,
        GenesisAccount {
            balance: U256::from(1u64),
            code: Bytes::new(),
            storage: Default::default(),
            nonce: 0,
        },
    );
    genesis.alloc.insert(
        delegated_addr,
        GenesisAccount {
            balance: U256::from(2u64),
            code: delegation_indicator(Address::repeat_byte(0x55)),
            storage: Default::default(),
            nonce: 0,
        },
    );
    genesis.alloc.insert(
        contract_addr,
        GenesisAccount {
            balance: U256::from(3u64),
            // Ordinary bytecode: PUSH1 0 PUSH1 0 RETURN. 5 bytes, not 23.
            code: Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xf3]),
            storage: Default::default(),
            nonce: 0,
        },
    );
    genesis.alloc.insert(
        same_length_contract_addr,
        GenesisAccount {
            balance: U256::from(4u64),
            // Exactly 23 bytes, but not `0xef0100`-prefixed.
            code: Bytes::from(vec![0xaa; 23]),
            storage: Default::default(),
            nonce: 0,
        },
    );

    let mut store =
        Store::new("il-store-provider.db", EngineType::InMemory).expect("failed to build store");
    store
        .add_initial_state(genesis)
        .await
        .expect("failed to add genesis state");

    let genesis_header = store
        .get_block_header(0)
        .expect("read genesis header")
        .expect("genesis header must exist");
    let provider = StoreIlStateProvider {
        store: &store,
        state_root: genesis_header.state_root,
    };

    // --- empty code -> Eoa ---
    let empty_view = provider
        .get_account(empty_code_addr)
        .expect("read empty-code account")
        .expect("empty-code account must exist");
    assert_eq!(empty_view.balance, U256::from(1u64));
    assert_eq!(
        empty_view.code_hash, *EMPTY_KECCAK_HASH,
        "an empty-code account must carry the empty-code hash"
    );
    assert_eq!(
        provider
            .classify_code(empty_view.code_hash)
            .expect("classify empty code"),
        SenderCode::Eoa
    );

    // --- 23-byte 0xef0100||address -> Delegated ---
    let delegated_code = delegation_indicator(Address::repeat_byte(0x55));
    let expected_delegated_hash = code_hash(&delegated_code, &NativeCrypto);
    let delegated_view = provider
        .get_account(delegated_addr)
        .expect("read delegated account")
        .expect("delegated account must exist");
    assert_eq!(delegated_view.balance, U256::from(2u64));
    assert_eq!(
        delegated_view.code_hash, expected_delegated_hash,
        "get_account must return the real code hash, not a placeholder"
    );
    assert_eq!(
        provider
            .classify_code(delegated_view.code_hash)
            .expect("classify delegated code"),
        SenderCode::Delegated
    );

    // --- ordinary contract body, length != 23 -> Contract ---
    let contract_code = Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xf3]);
    let expected_contract_hash = code_hash(&contract_code, &NativeCrypto);
    let contract_view = provider
        .get_account(contract_addr)
        .expect("read contract account")
        .expect("contract account must exist");
    assert_eq!(contract_view.balance, U256::from(3u64));
    assert_eq!(contract_view.code_hash, expected_contract_hash);
    assert_eq!(
        provider
            .classify_code(contract_view.code_hash)
            .expect("classify contract code"),
        SenderCode::Contract
    );

    // --- 23-byte body, wrong prefix -> Contract (length fast path alone
    // must not misclassify) ---
    let same_length_code = Bytes::from(vec![0xaa; 23]);
    let expected_same_length_hash = code_hash(&same_length_code, &NativeCrypto);
    let same_length_view = provider
        .get_account(same_length_contract_addr)
        .expect("read same-length contract account")
        .expect("same-length contract account must exist");
    assert_eq!(same_length_view.balance, U256::from(4u64));
    assert_eq!(same_length_view.code_hash, expected_same_length_hash);
    assert_eq!(
        provider
            .classify_code(same_length_view.code_hash)
            .expect("classify same-length non-delegation code"),
        SenderCode::Contract,
        "a 23-byte body without the 0xef0100 prefix must not be classified as a delegation"
    );
}
