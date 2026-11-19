//! daisugi-devnet-0's genesis and fork schedule.
//!
//! The devnet runs Prague with EIP-8141 frame transactions from genesis
//! (`eip8141PrototypeTime`) and EIP-8288, EIP-8250 and EIP-7906 from its Daisugi fork
//! (`eip8288PrototypeTime`). The expected values come from the network's published
//! configuration (ethpandaops/daisugi-devnets, `network-configs/devnet-0/metadata`).

use std::path::PathBuf;

use bytes::Bytes;
use ethrex_common::types::{
    ChainConfig, ChainFeatures, DEPENDENCY_SCHEME_LEANSTARK, FRAME_SIG_SCHEME_P256, Fork, ForkId,
    Frame, FrameMode, FrameSignature, FrameTransaction, Genesis, Transaction, dependencies_hash,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use std::str::FromStr;

const GENESIS_TIMESTAMP: u64 = 1_795_013_940;
const DAISUGI_FORK_TIMESTAMP: u64 = 1_795_017_840;

fn genesis() -> Genesis {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures/genesis/daisugi-devnet-0.json");
    Genesis::try_from(path.as_path()).expect("the devnet genesis loads")
}

#[test]
fn the_genesis_block_hash_matches_the_network() {
    let expected =
        H256::from_str("0x3c3a46744395b7beae03e0f994ec80c46d1b8d7646aced7e7b0501dcded7765d")
            .unwrap();
    assert_eq!(genesis().get_block().hash(), expected);
}

#[test]
fn frame_transactions_run_from_genesis_and_the_rest_from_the_daisugi_fork() {
    let config = genesis().config;
    let frames_only = ChainFeatures {
        frame_transactions: true,
        ..Default::default()
    };
    let everything = ChainFeatures {
        frame_transactions: true,
        keyed_nonces: true,
        assert_frames: true,
        dependency_proofs: true,
    };
    assert_eq!(config.features(GENESIS_TIMESTAMP), frames_only);
    assert_eq!(config.features(DAISUGI_FORK_TIMESTAMP - 1), frames_only);
    assert_eq!(config.features(DAISUGI_FORK_TIMESTAMP), everything);
}

#[test]
fn the_fork_id_counts_the_daisugi_fork_but_not_genesis() {
    let genesis = genesis();
    let header = genesis.get_block().header;
    let before = ForkId::new(genesis.config, header.clone(), GENESIS_TIMESTAMP, 0);
    assert_eq!(before.fork_hash.as_bytes(), &[0x7a, 0x7a, 0xe4, 0x84]);
    assert_eq!(before.fork_next, DAISUGI_FORK_TIMESTAMP);

    let after = ForkId::new(genesis.config, header, DAISUGI_FORK_TIMESTAMP, 325);
    assert_eq!(after.fork_hash.as_bytes(), &[0x82, 0xaa, 0x1e, 0x1e]);
    assert_eq!(after.fork_next, 0);
}

#[test]
fn eip8288_alone_schedules_frame_transactions_at_the_same_time() {
    let mut config = ChainConfig {
        eip8288_prototype_time: Some(100),
        ..Default::default()
    };
    config.check_prototype_schedule().unwrap();
    assert_eq!(config.eip8141_prototype_time, Some(100));
}

#[test]
fn eip8288_may_not_activate_before_frame_transactions() {
    let mut config = ChainConfig {
        eip8141_prototype_time: Some(200),
        eip8288_prototype_time: Some(100),
        ..Default::default()
    };
    assert!(config.check_prototype_schedule().is_err());
}

#[test]
fn the_aggregated_key_override_needs_its_timestamp() {
    let mut key_only = ChainConfig {
        eip8288_aggregated_vk: Some(H256::repeat_byte(1)),
        ..Default::default()
    };
    assert!(key_only.check_prototype_schedule().is_err());

    let mut timestamp_only = ChainConfig {
        eip8288_aggregated_vk_timestamp: Some(100),
        ..Default::default()
    };
    assert!(timestamp_only.check_prototype_schedule().is_err());
}

// ==================== Frame transaction envelopes ====================

/// The devnet's first three frame transactions (blocks 673, 675 and 676), rebuilt from
/// their JSON-RPC form and checked against their published hashes. 675 and 676 use the
/// keyed envelope with one key; 673 uses the empty key set.
const TX_673: &str = "0x06f901238429d103d8c0809405a20b6083bd71c58c10cbfa9db8036bf5a8e206f898cb010380c5830186a0808080f86b048080c48275308080b860000000000000000000000000000000000000000000000000000000000000001100000000000000000000000000000000000000000000000000000000000000004f0922a927365fec2cf13867145a3de76b3d26385681cd100c9b3d23c3c03d46de028094000000000000000000000000000000000000deadc482c350807b80f85cf85a019405a20b6083bd71c58c10cbfa9db8036bf5a8e20680b841016f7832e83fd7c388595d7b446ab9d1b05922bdd84cbc190e73a298f540e3d7a03470c708c423657fae82b574f91da2d15c0565c8aa564ea924e19b969145a544cd873ff2e795f500008477359400c0";
const TX_675: &str = "0x06f9012b8429d103d8c584ccbd53ec809405a20b6083bd71c58c10cbfa9db8036bf5a8e206f89bce010380c8830186a083017e808080f86b048080c48275308080b860000000000000000000000000000000000000000000000000000000000000001100000000000000000000000000000000000000000000000000000000000000004f0922a927365fec2cf13867145a3de76b3d26385681cd100c9b3d23c3c03d46de028094000000000000000000000000000000000000deadc482c350807b80f85cf85a019405a20b6083bd71c58c10cbfa9db8036bf5a8e20680b84101f9e4f70b43dcfddbd1ce7062d64373a45945d6772d68e28c35ca070c0fef77137420ebe89837279324ab8195934169a2b9f04542de1ec4eff441471a78c959c6cd8762bcad3f9900008477359400c0";
const TX_676: &str = "0x06f9012b8429d103d8c584ccbd53ec019405a20b6083bd71c58c10cbfa9db8036bf5a8e206f89bce010380c8830186a083017e808080f86b048080c48275308080b860000000000000000000000000000000000000000000000000000000000000001100000000000000000000000000000000000000000000000000000000000000004f0922a927365fec2cf13867145a3de76b3d26385681cd100c9b3d23c3c03d46de028094000000000000000000000000000000000000deadc482c350807b80f85cf85a019405a20b6083bd71c58c10cbfa9db8036bf5a8e20680b8410138250a893bc2bb6b6d27e7a1b49ca1fce367fa9646ff5fca69e294a83e345fca3c2d7af2357f06da24e1a6a9ae65c9728a2830460d142f674acb7dcd36960cb0cd8762bcad3f9900008477359400c0";

fn decode_frame_tx(raw: &str) -> (Vec<u8>, FrameTransaction) {
    let bytes = hex::decode(raw.trim_start_matches("0x")).unwrap();
    match Transaction::decode_canonical(&bytes).unwrap() {
        Transaction::FrameTransaction(tx) => (bytes, tx),
        other => panic!("expected a frame transaction, got {other:?}"),
    }
}

#[test]
fn the_devnet_frame_transactions_decode_and_hash_as_published() {
    for (raw, hash, keys, nonce_seq) in [
        (
            TX_673,
            "0x01fb5a14e6af4d73e8e7f62ef2fbbc4bb013ce5bc4e4400716d5c69a09434773",
            vec![],
            0,
        ),
        (
            TX_675,
            "0x576438a5eff9cac236779606800c9ed4b172faf3e760557eef527c3739477f38",
            vec![U256::from(0xccbd53ecu64)],
            0,
        ),
        (
            TX_676,
            "0x9c55285a3bdb6f3815ec45de4c158e8d0a8110b51201c23df5acebd0b485b2e3",
            vec![U256::from(0xccbd53ecu64)],
            1,
        ),
    ] {
        let (bytes, tx) = decode_frame_tx(raw);
        assert!(tx.has_nonce_keys);
        assert_eq!(tx.nonce_keys, keys);
        assert_eq!(tx.nonce_seq, nonce_seq);
        let tx = Transaction::FrameTransaction(tx);
        assert_eq!(tx.hash(&NativeCrypto), H256::from_str(hash).unwrap());
        assert_eq!(tx.encode_canonical_to_vec(), bytes);
    }
}

/// A frame transaction in EIP-8141's own envelope, before EIP-8250: a scalar
/// `nonce` right after `chain_id`.
fn scalar_nonce_tx() -> FrameTransaction {
    FrameTransaction {
        chain_id: 701_563_864,
        has_nonce_keys: false,
        nonce_seq: 5,
        sender: Address::from_low_u64_be(0xABCD),
        frames: vec![Frame {
            mode: FrameMode::Verify as u8,
            flags: 3,
            gas_limit: 50_000,
            ..Default::default()
        }],
        max_fee: U256::from(1_000_000_000_000_000u64),
        ..Default::default()
    }
}

#[test]
fn the_scalar_nonce_envelope_round_trips_without_nonce_calldata() {
    let tx = scalar_nonce_tx();
    let encoded = Transaction::FrameTransaction(tx.clone()).encode_canonical_to_vec();
    // chain_id (a 4-byte string) is followed directly by the scalar nonce 5, not by a list.
    assert!(
        encoded
            .windows(6)
            .any(|w| w == [0x84, 0x29, 0xd1, 0x03, 0xd8, 0x05])
    );

    let (_, decoded) = decode_frame_tx(&format!("0x{}", hex::encode(&encoded)));
    assert!(!decoded.has_nonce_keys);
    assert!(decoded.nonce_keys.is_empty());
    assert_eq!(decoded.nonce_seq, 5);
    assert!(decoded.nonce_calldata().is_empty());

    // The keyed envelope with the empty key set is a different transaction.
    let keyed = FrameTransaction {
        has_nonce_keys: true,
        ..tx.clone()
    };
    assert_ne!(keyed.compute_sig_hash(), tx.compute_sig_hash());
    assert!(!keyed.nonce_calldata().is_empty());
}

#[test]
fn each_envelope_is_valid_only_on_its_side_of_the_daisugi_fork() {
    let config = genesis().config;
    let before = config.features(DAISUGI_FORK_TIMESTAMP - 1);
    let after = config.features(DAISUGI_FORK_TIMESTAMP);
    let scalar = scalar_nonce_tx();
    let keyed = FrameTransaction {
        has_nonce_keys: true,
        ..scalar.clone()
    };
    assert!(scalar.validate_fork_rules(before, Fork::Prague).is_ok());
    assert!(scalar.validate_fork_rules(after, Fork::Prague).is_err());
    assert!(keyed.validate_fork_rules(before, Fork::Prague).is_err());
    assert!(keyed.validate_fork_rules(after, Fork::Prague).is_ok());
}

#[test]
fn assert_frames_need_the_daisugi_fork() {
    let config = genesis().config;
    let mut tx = scalar_nonce_tx();
    tx.frames.push(Frame {
        mode: FrameMode::Assert as u8,
        ..Default::default()
    });
    assert!(
        tx.validate_fork_rules(config.features(DAISUGI_FORK_TIMESTAMP - 1), Fork::Prague)
            .is_err()
    );
    tx.has_nonce_keys = true;
    assert!(
        tx.validate_fork_rules(config.features(DAISUGI_FORK_TIMESTAMP), Fork::Prague)
            .is_ok()
    );
}

#[test]
fn p256_signatures_need_the_osaka_precompile() {
    let features = genesis().config.features(GENESIS_TIMESTAMP);
    let mut tx = scalar_nonce_tx();
    tx.signatures.push(FrameSignature {
        scheme: FRAME_SIG_SCHEME_P256,
        signer: None,
        msg: Bytes::new(),
        signature: Bytes::from(vec![1u8; 128]),
    });
    assert!(tx.validate_fork_rules(features, Fork::Prague).is_err());
    assert!(tx.validate_fork_rules(features, Fork::Osaka).is_ok());
}

// ==================== Prague pricing ====================

/// Nethermind's receipts for the three devnet frame transactions imply their intrinsic
/// gas and calldata floor on Prague: EIP-7623's weighted token count (a zero byte is one
/// token, any other byte four, an empty-`msg` signature's bytes all four), four gas per
/// token for the intrinsic cost and ten for the floor, no EIP-2780 value charge, 12,100
/// gas per nonce key, and EIP-8288's 30,000 per declared dependency.
#[test]
fn frame_transactions_on_prague_price_calldata_as_the_network_does() {
    for (raw, intrinsic, floor) in [
        (TX_673, 48_397, 51_655),
        (TX_675, 60_577, 63_955),
        (TX_676, 60_577, 63_955),
    ] {
        let (_, tx) = decode_frame_tx(raw);
        assert_eq!(tx.recursive_stark_gas(), 30_000);
        assert_eq!(tx.mandatory_gas(Fork::Prague) + tx.data_cost(), intrinsic);
        assert_eq!(tx.calldata_floor_total(Fork::Prague), floor);
        // Amsterdam's flat count and higher floor never price below Prague's.
        assert!(tx.calldata_floor_total(Fork::Amsterdam) >= tx.calldata_floor_total(Fork::Prague));
    }
}

/// Each block's `blockDepsHash` as the network's header carries it. Every transaction
/// declares one leanSTARK (`0x11`) dependency with a zero `data_hash`, which stands
/// for the transaction's own signature hash.
#[test]
fn the_dependency_set_hashes_to_each_blocks_header_commitment() {
    for (raw, block_deps_hash) in [
        (
            TX_673,
            "0xe79b80cdcc7c68c07bfa15f80eef9fe10a92b5adc0246b5074692d88e4543f72",
        ),
        (
            TX_675,
            "0x803ea13cfe84fb8a2e3d777f67d71f5c2205f06e001c5e1ac0cc83b6842d2d08",
        ),
        (
            TX_676,
            "0xdbb284cfad5170caa8a2f7fb8e755e7d085aa32e88581d0037ea62c656ce4191",
        ),
    ] {
        let (_, tx) = decode_frame_tx(raw);
        let deps = tx.dependencies();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].scheme, DEPENDENCY_SCHEME_LEANSTARK);
        assert_eq!(deps[0].data_hash, tx.compute_sig_hash());
        assert_eq!(
            dependencies_hash(&deps),
            H256::from_str(block_deps_hash).unwrap()
        );
    }
}

#[test]
fn the_empty_dependency_set_hashes_to_the_empty_keccak() {
    assert_eq!(
        dependencies_hash(&[]),
        H256::from_str("0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
            .unwrap()
    );
}

#[test]
fn dependency_frames_need_the_daisugi_fork() {
    let config = genesis().config;
    let (_, tx) = decode_frame_tx(TX_673);
    assert!(tx.validate_static_constraints().is_ok());
    let mut scalar = tx.clone();
    scalar.has_nonce_keys = false;
    assert!(
        scalar
            .validate_fork_rules(config.features(DAISUGI_FORK_TIMESTAMP - 1), Fork::Prague)
            .is_err()
    );
    assert!(
        tx.validate_fork_rules(config.features(DAISUGI_FORK_TIMESTAMP), Fork::Prague)
            .is_ok()
    );
}

// ==================== Executing the devnet's transactions ====================

mod execution {
    use super::*;
    use ethrex_blockchain::vm::StoreVmDatabase;
    use ethrex_common::constants::EMPTY_TRIE_HASH;
    use ethrex_common::types::{Account, BlockHeader, Code, FRAME_RECEIPT_STATUS_SUCCESS};
    use ethrex_levm::db::gen_db::GeneralizedDatabase;
    use ethrex_levm::environment::{EVMConfig, Environment};
    use ethrex_levm::tracing::LevmCallTracer;
    use ethrex_levm::vm::{VM, VMType};
    use ethrex_storage::Store;
    use ethrex_vm::DynVmDatabase;
    use rustc_hash::FxHashMap;
    use std::sync::Arc;

    const BASE_FEE: u64 = 7;
    const GAS_LIMIT: u64 = 60_000_000;
    const CHAIN_ID: u64 = 701_563_864;
    const COINBASE: Address = Address::repeat_byte(0xc0);

    fn daisugi_config() -> EVMConfig {
        EVMConfig {
            fork: Fork::Prague,
            blob_schedule: EVMConfig::canonical_values(Fork::Prague),
            features: ChainFeatures {
                frame_transactions: true,
                keyed_nonces: true,
                assert_frames: true,
                dependency_proofs: true,
            },
        }
    }

    fn funded(address: Address) -> GeneralizedDatabase {
        let store = Store::new("", ethrex_storage::EngineType::InMemory).unwrap();
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let store: DynVmDatabase = Box::new(StoreVmDatabase::new(store, header).unwrap());
        let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
        accounts.insert(
            address,
            Account::new(
                U256::from(10u64).pow(U256::from(18u64)),
                Code::from_bytecode(Bytes::new(), &NativeCrypto),
                0,
                FxHashMap::default(),
            ),
        );
        GeneralizedDatabase::new_with_account_state(Arc::new(store), accounts)
    }

    /// The devnet's block 673 holds one frame transaction: a VERIFY frame its sender's
    /// default code approves with a SECP256K1 signature, a DEP_VERIFY frame declaring
    /// one leanSTARK dependency, and a SENDER frame paying 123 wei to `0x…dead`. Its
    /// Nethermind receipt: 81,097 gas, frames at 100, 30,000 and 2,600 execution gas
    /// and no state gas, at an effective gas price of 2,000,000,007.
    #[test]
    fn block_673s_transaction_executes_as_the_network_recorded_it() {
        let (_, tx) = decode_frame_tx(TX_673);
        let sender = tx.sender;
        let config = daisugi_config();
        let gas_price = tx
            .fee_budget(Fork::Prague, U256::from(BASE_FEE), U256::zero())
            .expect("the budget covers the base fee")
            .gas_price;
        assert_eq!(gas_price, U256::from(2_000_000_007u64));
        let env = Environment {
            origin: sender,
            gas_limit: tx.max_gas(Fork::Prague),
            config,
            block_number: 673,
            coinbase: COINBASE,
            timestamp: 1_795_022_088,
            chain_id: U256::from(CHAIN_ID),
            base_fee_per_gas: U256::from(BASE_FEE),
            gas_price,
            tx_nonce: tx.nonce_seq,
            block_gas_limit: GAS_LIMIT,
            ..Default::default()
        };
        let mut db = funded(sender);
        let transaction = Transaction::FrameTransaction(tx);
        let report = VM::new(
            env,
            &mut db,
            &transaction,
            LevmCallTracer::disabled(),
            VMType::L1,
            &NativeCrypto,
            None,
        )
        .expect("VM::new")
        .execute()
        .expect("the transaction executes");

        assert!(report.is_success());
        assert_eq!(report.gas_spent, 81_097);
        assert_eq!(report.gas_used, 81_097);
        assert_eq!(report.state_gas_used, 0);
        let frames: Vec<(u8, u64, u64)> = report
            .frame_results
            .expect("a frame transaction reports its frames")
            .into_iter()
            .map(|(status, gas, state, _)| (status, gas, state))
            .collect();
        assert_eq!(
            frames,
            vec![
                (FRAME_RECEIPT_STATUS_SUCCESS, 100, 0),
                (FRAME_RECEIPT_STATUS_SUCCESS, 30_000, 0),
                (FRAME_RECEIPT_STATUS_SUCCESS, 2_600, 0),
            ]
        );
    }
}

// ==================== Headers ====================

/// Headers from the devnet's public RPC, rebuilt as RLP and checked against their
/// published hashes: block 100 before the Daisugi fork (21 Prague fields), and blocks
/// 319, 673 and 6987 after it (the 21 Prague fields, the empty-string block-access-list
/// placeholder, slot 0, then `stark_proof_hash` and `block_deps_hash`). 319 has no
/// dependencies; 673 and 6987 carry a proof.
const HEADERS: [(u64, &str, &str); 4] = [
    (
        100,
        "0xff018ae973ee55a1ff733b19f07153ee04b59014adfea2b1a9dbf374893ab3dc",
        "0xf9026fa09e986e27c4146f83cd3cb4dc2c0ee050b847494e8603e9b181bd41591f855046a01dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d4934794f97e180c050e5ab072211ad2c213eb5aee4df134a0eeffa6faf9fb2aebc4ffda057488a7b1bcdb4dfe32d2ac90bcca30484681b72ca056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421b90100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008064840393870080846afdc22c924e65746865726d696e642076322e332e3061a0aee3fd46b02cd973895a0dcbaf636a6de8c1865aade9d2599c16c396ef7c5899880000000000000000820638a0ebd31067922c85be9d26ee553518588f17da632d41b55da51c69f273e0ff4de18080a04cd9c75960d878e6eb99c22e76611107d3cc34bc53e003f9f77c8e0e053d826fa0e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    ),
    (
        319,
        "0x7c1d2dc159fbc16c1f0ae83152873a972dbc799b6beaaec12f68da0175f394a2",
        "0xf902b3a0ebc035c0f4c3fcaa4a2a41063f2ab1a4973584426c846b8bfb2d953e57892511a01dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d4934794f97e180c050e5ab072211ad2c213eb5aee4df134a02a6a3dfd0f08dcf8c2c56a42d6d778d00c48f929bacc0121d24c82615971132da056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421b90100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008082013f840393870080846afdcc70924e65746865726d696e642076322e332e3061a092778fffcf52955c9964d4baa3bb0920937ee7375eda26164d67ffb96d5a51df88000000000000000007a0f13528cf7aa4d1b31b169d4863022d8dcdbcd1e7ebe99ef4d1904e0dedcd90068080a020f93203134ee5b75daa92ca68861f771f6771e2c78520e82e49f91de12dfa7aa0e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b8558080a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
    ),
    (
        673,
        "0x1cf16619bff31e96a6078cc00bbbe92e66a74f4152c01cb51f38515f8e7f10a1",
        "0xf902b6a016805fa2022be6e0304119f12ed4e98893181c92f9356205e006473e293c2a16a01dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d4934794f97e180c050e5ab072211ad2c213eb5aee4df134a019e31c3ac08cd6e1536f317dfa82e16795ee5753d3c42794c6058cb4cfa4f46ba0c3badddcf3b959f1efcbac7f2675f6cf4daafd7b93c15ad067e56f3a20695787a0f310b8fa279f7529d8f8bf012a9dcc30bb0338fca48d6145b04fe5afa962c0beb9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000808202a1840393870083013cc9846afddd08924e65746865726d696e642076322e332e3061a08c38a5a3bb8c312e39b8571215c61184b353dc4361a1f007fe7f64df3071ccc888000000000000000007a069f63eb1882e657cc4f90619a48a4c7b5e143b76aacffed9a7b68879511d9f758080a0bc0f348b220d38f11dbf6c30a7e727ef3dbd5e4241c2a5cd770d7ac15869234ba0e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b8558080a0455907bb1fc322d21d0e5a5ed9b09be3f78be381fb134a2af30fe5a7512d817ea0e79b80cdcc7c68c07bfa15f80eef9fe10a92b5adc0246b5074692d88e4543f72",
    ),
    (
        6987,
        "0x095e8177a253be1532b49503b403c6d833d2a4de88bcd0604020c84da9e385c7",
        "0xf902b5a007e3ced2949850296903e6e2acbfc136c439e21e512cd19a92f1f959ecc1d3dca01dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d4934794f97e180c050e5ab072211ad2c213eb5aee4df134a0bb6ea188cb4441b31fa143f71125932746bca077e1e6526477a095f3af518f78a0e6fd7c2528a1e87625c96050509ac98eccf115808ab33c5472913dfe1928f3c5a00e8c9d8810594a4187b1c4a864e3e81e1e45bd9095435c90235bf589e5c98b71b901000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000080821b4b840393870082cc14846aff0500924e65746865726d696e642076322e332e3061a00bcc9bf6353bb00522e0b10f78b19842cde892d7c368e7c64c93d8a44feefaf988000000000000000007a0975f6dad50af45ffca42140b840916edfba8eb1380515e598dfc50453a911a0b8080a02e40c5eff7b13f814a4c795ca8c28c55c89dc0a34516566938873cd85e9f0c07a0e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b8558080a08f4fc1fb9bee59a05e19cc232da3fdda7120e2c7483473e13355e55845c90a5aa0d89d9a1ccc0f36927f4e2597a2ae6b7d4fb0d4a10ed52dc9177cda5d9a21c5b1",
    ),
];

#[test]
fn devnet_headers_decode_hash_and_reencode_as_published() {
    use ethrex_common::types::BlockHeader;
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
    let empty =
        H256::from_str("0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
            .unwrap();
    for (number, hash, rlp) in HEADERS {
        let bytes = hex::decode(rlp.trim_start_matches("0x")).unwrap();
        let header = BlockHeader::decode(&bytes).unwrap();
        assert_eq!(header.number, number);
        assert_eq!(
            header.hash(),
            H256::from_str(hash).unwrap(),
            "block {number}"
        );
        assert_eq!(header.encode_to_vec(), bytes, "block {number}");
        assert_eq!(header.block_access_list_hash, None);
        if number < 319 {
            assert!(header.recursive_stark.is_none());
            assert_eq!(header.slot_number, None);
        } else {
            let commitment = header
                .recursive_stark
                .expect("a Daisugi header commits to its proof");
            assert_eq!(header.slot_number, Some(0));
            if number == 319 {
                assert_eq!(commitment.stark_proof_hash, empty);
                assert_eq!(commitment.block_deps_hash, empty);
            } else {
                assert_ne!(commitment.stark_proof_hash, empty);
            }
        }
    }
}

// ==================== Block bodies ====================

mod bodies {
    use super::*;
    use ethrex_common::types::{
        BlockBody, BlockHeader, InvalidBlockBodyError, MAX_RECURSIVE_STARK_PROOF_BYTES,
        RecursiveStark, compute_transactions_root, compute_withdrawals_root, validate_block_body,
    };
    use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

    fn body(stark_proof: Option<Bytes>) -> BlockBody {
        BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(vec![]),
            stark_proof,
        }
    }

    fn header_for(body: &BlockBody, recursive_stark: Option<RecursiveStark>) -> BlockHeader {
        BlockHeader {
            transactions_root: compute_transactions_root(&body.transactions, &NativeCrypto),
            withdrawals_root: Some(compute_withdrawals_root(&[], &NativeCrypto)),
            recursive_stark,
            ..Default::default()
        }
    }

    #[test]
    fn a_body_carries_its_proof_as_a_fourth_item_only_after_the_fork() {
        let before = body(None);
        let decoded = BlockBody::decode(&before.encode_to_vec()).unwrap();
        assert_eq!(decoded.stark_proof, None);

        let after = body(Some(Bytes::from(vec![7u8; 1_000])));
        let decoded = BlockBody::decode(&after.encode_to_vec()).unwrap();
        assert_eq!(decoded.stark_proof, after.stark_proof);
        // The empty proof of a block without dependencies is a present, empty item.
        let empty = body(Some(Bytes::new()));
        let decoded = BlockBody::decode(&empty.encode_to_vec()).unwrap();
        assert_eq!(decoded.stark_proof, Some(Bytes::new()));
    }

    #[test]
    fn an_oversized_proof_is_refused_while_decoding() {
        let oversized = body(Some(Bytes::from(vec![
            1u8;
            MAX_RECURSIVE_STARK_PROOF_BYTES + 1
        ])));
        assert!(BlockBody::decode(&oversized.encode_to_vec()).is_err());
    }

    #[test]
    fn a_body_must_carry_the_proof_its_header_commits_to() {
        let proof = Bytes::from(vec![3u8; 64]);
        let with_proof = body(Some(proof.clone()));
        let commitment = RecursiveStark::commit(&proof, H256::repeat_byte(9));
        let header = header_for(&with_proof, Some(commitment.clone()));
        assert!(validate_block_body(&header, &with_proof, &NativeCrypto).is_ok());

        let other = body(Some(Bytes::from(vec![4u8; 64])));
        assert!(matches!(
            validate_block_body(&header, &other, &NativeCrypto),
            Err(InvalidBlockBodyError::StarkProofNotMatch)
        ));

        // No header commitment: a proof in the body is a body from another block.
        let pre_fork = header_for(&with_proof, None);
        assert!(matches!(
            validate_block_body(&pre_fork, &with_proof, &NativeCrypto),
            Err(InvalidBlockBodyError::StarkProofPresent)
        ));
    }
}
