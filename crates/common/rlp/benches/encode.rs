use bytes::Bytes;
use criterion::{Criterion, criterion_group, criterion_main};
use ethereum_types::U256;
use ethrex_common::{
    Address, Bloom, H32, H160, H256, H264, H512,
    constants::EMPTY_KECCACK_HASH,
    types::{
        AccessList, AccountInfo, AccountState, AuthorizationList, AuthorizationTuple,
        BYTES_PER_BLOB, BlobsBundle, Block, BlockBody, BlockHeader, EIP1559Transaction,
        EIP2930Transaction, EIP4844Transaction, EIP7702Transaction, FeeTokenTransaction, ForkId,
        LegacyTransaction, Log, MempoolTransaction, P2PTransaction, PrivilegedL2Transaction,
        Receipt, ReceiptWithBloom, Transaction, TxKind, TxType, Withdrawal,
        WrappedEIP4844Transaction, requests::EncodedRequests,
    },
};
use ethrex_p2p::{
    discv4::messages::{ENRRequestMessage, FindNodeMessage, NeighborsMessage, PingMessage},
    rlpx::{
        p2p::Capability,
        snap::{AccountRangeUnit, AccountStateSlim, StorageSlot},
    },
    types::{Endpoint, Node, NodeRecord, NodeRecordPairs},
};
use ethrex_rlp::{encode::RLPEncode, structs::Encoder};
use ethrex_trie::{
    Nibbles, Node as TrieNode, NodeHash, NodeRef,
    node::{BranchNode, ExtensionNode, LeafNode},
};
use rand::{Rng, SeedableRng, rngs::StdRng};
use std::{
    hint::black_box,
    net::{IpAddr, Ipv4Addr},
    str::FromStr,
};

fn make_string_list(count: usize) -> Vec<String> {
    let entry = "abcdefghij".to_string();
    vec![entry; count]
}

fn create_nibbles(len: usize) -> Nibbles {
    let pattern = (0..len).map(|i| (i % 16) as u8).collect::<Vec<u8>>();
    Nibbles::from_hex(pattern)
}

fn random_u256(rng: &mut StdRng) -> U256 {
    let bytes: [u8; 32] = rng.r#gen();
    U256::from_big_endian(&bytes)
}

fn create_access_list() -> AccessList {
    vec![(
        Address::from_str("0x000000000000000000000000000000000000000a").unwrap(),
        vec![HASH],
    )]
}

fn create_authorization_list() -> AuthorizationList {
    vec![AuthorizationTuple {
        chain_id: U256::from(1u64),
        address: Address::from_str("0x00000000000000000000000000000000000000bb").unwrap(),
        nonce: 1,
        y_parity: U256::from(1u64),
        r_signature: U256::from(2u64),
        s_signature: U256::from(3u64),
    }]
}

fn create_endpoint(octet: u8, udp_port: u16, tcp_port: u16) -> Endpoint {
    Endpoint {
        ip: IpAddr::V4(Ipv4Addr::new(192, 168, 0, octet)),
        udp_port,
        tcp_port,
    }
}

fn create_node(index: u8) -> Node {
    Node::new(
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, index)),
        30_300 + index as u16,
        40_400 + index as u16,
        H512::repeat_byte(index),
    )
}

fn create_node_record() -> NodeRecord {
    let pairs: Vec<(Bytes, Bytes)> = NodeRecordPairs {
        id: Some("v4".to_string()),
        ip: Some(Ipv4Addr::new(127, 0, 0, 1)),
        ip6: None,
        tcp_port: Some(30303),
        udp_port: Some(30303),
        secp256k1: Some(H264::repeat_byte(0x33)),
        eth: None,
    }
    .into();

    NodeRecord {
        signature: H512::repeat_byte(0x22),
        seq: 1,
        pairs,
    }
}

static HASH: H256 = H256::repeat_byte(0xab);

fn bench_encode_integer(c: &mut Criterion) {
    let mut group = c.benchmark_group("basic_types");

    let u8_values = {
        let mut rng = StdRng::seed_from_u64(1);
        (0..1_000_000)
            .map(|_| {
                let byte: u8 = rng.r#gen();
                byte & 0x7f
            })
            .collect::<Vec<u8>>()
    };
    group.bench_function("encode_u8_random_seeded", move |b| {
        let mut buf = Vec::with_capacity(2 * u8_values.len());
        b.iter(|| {
            buf.clear();
            for &value in &u8_values {
                value.encode(&mut buf);
            }
            black_box(&buf);
        });
    });

    let u16_values = {
        let mut rng = StdRng::seed_from_u64(2);
        (0..1_000_000)
            .map(|_| {
                let value: u16 = rng.r#gen();
                value % 10_001
            })
            .collect::<Vec<u16>>()
    };
    group.bench_function("encode_u16_random_seeded", move |b| {
        let mut buf = Vec::with_capacity(3 * u16_values.len());
        b.iter(|| {
            buf.clear();
            for &value in &u16_values {
                value.encode(&mut buf);
            }
            black_box(&buf);
        });
    });

    let u32_values = {
        let mut rng = StdRng::seed_from_u64(3);
        (0..1_000_000)
            .map(|_| {
                let value: u32 = rng.r#gen();
                value % 1_000_001
            })
            .collect::<Vec<u32>>()
    };
    group.bench_function("encode_u32_random_seeded", move |b| {
        let mut buf = Vec::with_capacity(5 * u32_values.len());
        b.iter(|| {
            buf.clear();
            for &value in &u32_values {
                value.encode(&mut buf);
            }
            black_box(&buf);
        });
    });

    let u64_values = {
        let mut rng = StdRng::seed_from_u64(4);
        (0..1_000_000)
            .map(|_| {
                let value: u64 = rng.r#gen();
                value % 1_000_000_001
            })
            .collect::<Vec<u64>>()
    };
    group.bench_function("encode_u64_random_seeded", move |b| {
        let mut buf = Vec::with_capacity(9 * u64_values.len());
        b.iter(|| {
            buf.clear();
            for &value in &u64_values {
                value.encode(&mut buf);
            }
            black_box(&buf);
        });
    });

    let u128_values = {
        let mut rng = StdRng::seed_from_u64(5);
        (0..1_000_000).map(|_| rng.r#gen()).collect::<Vec<u128>>()
    };
    group.bench_function("encode_u128_random_seeded", move |b| {
        let mut buf = Vec::with_capacity(17 * u128_values.len());
        b.iter(|| {
            buf.clear();
            for &value in &u128_values {
                value.encode(&mut buf);
            }
            black_box(&buf);
        });
    });

    let u256_values = {
        let mut rng = StdRng::seed_from_u64(6);
        (0..1_000_000)
            .map(|_| random_u256(&mut rng))
            .collect::<Vec<U256>>()
    };
    group.bench_function("encode_u256_random_seeded", move |b| {
        let mut buf = Vec::with_capacity(33 * u256_values.len());
        b.iter(|| {
            buf.clear();
            for value in &u256_values {
                value.encode(&mut buf);
            }
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_strings(c: &mut Criterion) {
    let mut group = c.benchmark_group("basic_types");
    for &len in &[5usize, 60, 500] {
        let label = format!("encode_len_{len}");
        let mut rng = StdRng::seed_from_u64(len as u64);
        let values: Vec<String> = (0..10_000)
            .map(|_| {
                let mut s = String::with_capacity(len);
                for _ in 0..len {
                    s.push(rng.r#gen());
                }
                s
            })
            .collect();
        let values = black_box(values);
        group.bench_function(label, move |b| {
            let mut buf = Vec::with_capacity(values[0].length() * values.len());
            b.iter(|| {
                buf.clear();
                for v in &values {
                    v.encode(&mut buf);
                }
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_int_lists(c: &mut Criterion) {
    let mut group = c.benchmark_group("basic_types");
    for &count in &[10usize, 100, 1000] {
        let label = format!("encode_int_list_len_{count}");
        let mut rng = StdRng::seed_from_u64(count as u64);
        let values: Vec<u64> = (0..count).map(|_| rng.r#gen()).collect();
        let value = black_box(values);
        group.bench_function(label, move |b| {
            let mut buf = Vec::with_capacity(value.length());
            b.iter(|| {
                buf.clear();
                value.encode(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_string_lists(c: &mut Criterion) {
    let mut group = c.benchmark_group("basic_types");
    for &count in &[10usize, 100, 1000] {
        let label = format!("encode_string_list_len_{count}");
        let value = black_box(make_string_list(count));
        group.bench_function(label, move |b| {
            let mut buf = Vec::with_capacity(value.length());
            b.iter(|| {
                buf.clear();
                value.encode(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_account_info(c: &mut Criterion) {
    let mut group = c.benchmark_group("common_types");

    let account_info = black_box(AccountInfo {
        code_hash: HASH,
        balance: U256::from(0xf34ab23u64),
        nonce: 1,
    });

    group.bench_function("encode_account_info", move |b| {
        let mut buf = Vec::with_capacity(account_info.length());
        b.iter(|| {
            buf.clear();
            account_info.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_account_state(c: &mut Criterion) {
    let mut group = c.benchmark_group("common_types");

    let account_state = black_box(AccountState {
        nonce: 1,
        balance: U256::from(0xf34ab23u64),
        storage_root: HASH,
        code_hash: HASH,
    });

    group.bench_function("encode_account_state", move |b| {
        let mut buf = Vec::with_capacity(account_state.length());
        b.iter(|| {
            buf.clear();
            account_state.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_tx_kind(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    let create_kind = black_box(TxKind::Create);
    let call_kind = black_box(TxKind::Call(
        Address::from_str("0x00000000000000000000000000000000000000ff").unwrap(),
    ));

    group.bench_function("encode_create", move |b| {
        let mut buf = Vec::with_capacity(create_kind.length());
        b.iter(|| {
            buf.clear();
            create_kind.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("encode_call", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            call_kind.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_legacy_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    let legacy_tx = black_box(LegacyTransaction {
        nonce: 1,
        gas_price: U256::from(50_000),
        gas: 21_000,
        to: TxKind::Create,
        value: U256::from(1_000_000u64),
        data: Bytes::from(vec![0u8; 32]),
        v: U256::from(27u64),
        r: U256::from(1u64),
        s: U256::from(2u64),
        inner_hash: Default::default(),
    });

    group.bench_function("encode_legacy_transaction", move |b| {
        let mut buf = Vec::with_capacity(legacy_tx.length());
        b.iter(|| {
            buf.clear();
            legacy_tx.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_eip2930_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    let eip2930_tx = black_box(EIP2930Transaction {
        chain_id: 1,
        nonce: 2,
        gas_price: U256::from(30_000),
        gas_limit: 50_000,
        to: TxKind::Call(Address::from_str("0x0000000000000000000000000000000000000aaa").unwrap()),
        value: U256::from(42u64),
        data: Bytes::from(vec![0x12; 16]),
        access_list: create_access_list(),
        signature_y_parity: true,
        signature_r: U256::from(5u64),
        signature_s: U256::from(6u64),
        inner_hash: Default::default(),
    });

    group.bench_function("encode_eip2930_transaction", move |b| {
        let mut buf = Vec::with_capacity(eip2930_tx.length());
        b.iter(|| {
            buf.clear();
            eip2930_tx.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_eip1559_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    let eip1559_tx = black_box(EIP1559Transaction {
        chain_id: 1,
        nonce: 3,
        max_priority_fee_per_gas: 1,
        max_fee_per_gas: 100,
        gas_limit: 100_000,
        to: TxKind::Create,
        value: U256::from(900u64),
        data: Bytes::from(vec![0x34; 24]),
        access_list: create_access_list(),
        signature_y_parity: false,
        signature_r: U256::from(7u64),
        signature_s: U256::from(8u64),
        inner_hash: Default::default(),
    });

    group.bench_function("encode_eip1559_transaction", move |b| {
        let mut buf = Vec::with_capacity(eip1559_tx.length());
        b.iter(|| {
            buf.clear();
            eip1559_tx.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_eip4844_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    let eip4844_tx = EIP4844Transaction {
        chain_id: 1,
        nonce: 4,
        max_priority_fee_per_gas: 2,
        max_fee_per_gas: 200,
        gas: 120_000,
        to: Address::from_str("0x0000000000000000000000000000000000000bbb").unwrap(),
        value: U256::from(1_500u64),
        data: Bytes::from(vec![0x56; 48]),
        access_list: create_access_list(),
        max_fee_per_blob_gas: U256::from(10u64),
        blob_versioned_hashes: vec![H256::repeat_byte(0x44)],
        signature_y_parity: true,
        signature_r: U256::from(9u64),
        signature_s: U256::from(10u64),
        inner_hash: Default::default(),
    };

    group.bench_function("encode_eip4844_transaction", move |b| {
        let mut buf = Vec::with_capacity(eip4844_tx.length());
        b.iter(|| {
            buf.clear();
            eip4844_tx.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_wrapped_eip4844_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    let inner_tx = EIP4844Transaction {
        chain_id: 1,
        nonce: 5,
        max_priority_fee_per_gas: 3,
        max_fee_per_gas: 300,
        gas: 130_000,
        to: Address::from_str("0x0000000000000000000000000000000000000ccc").unwrap(),
        value: U256::from(2_500u64),
        data: Bytes::from(vec![0x78; 64]),
        access_list: create_access_list(),
        max_fee_per_blob_gas: U256::from(12u64),
        blob_versioned_hashes: vec![H256::repeat_byte(0x55); 2],
        signature_y_parity: true,
        signature_r: U256::from(11u64),
        signature_s: U256::from(12u64),
        inner_hash: Default::default(),
    };

    let blobs_bundle = BlobsBundle {
        blobs: vec![[7u8; BYTES_PER_BLOB]; 2],
        commitments: vec![[0x23u8; 48]; 2],
        proofs: vec![[0x34u8; 48]; 2],
        version: 1,
    };

    let wrapped = black_box(WrappedEIP4844Transaction {
        tx: black_box(inner_tx),
        wrapper_version: Some(1),
        blobs_bundle,
    });

    group.bench_function("encode_wrapped_eip4844_transaction", move |b| {
        let mut buf = Vec::with_capacity(wrapped.length());
        b.iter(|| {
            buf.clear();
            wrapped.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_eip7702_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    let eip7702_tx = black_box(EIP7702Transaction {
        chain_id: 1,
        nonce: 6,
        max_priority_fee_per_gas: 4,
        max_fee_per_gas: 400,
        gas_limit: 140_000,
        to: Address::from_str("0x0000000000000000000000000000000000000ddd").unwrap(),
        value: U256::from(3_500u64),
        data: Bytes::from(vec![0x9a; 72]),
        access_list: create_access_list(),
        authorization_list: create_authorization_list(),
        signature_y_parity: false,
        signature_r: U256::from(13u64),
        signature_s: U256::from(14u64),
        inner_hash: Default::default(),
    });

    group.bench_function("encode_eip7702_transaction", move |b| {
        let mut buf = Vec::with_capacity(eip7702_tx.length());
        b.iter(|| {
            buf.clear();
            eip7702_tx.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_privileged_l2_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    let privileged_tx = black_box(PrivilegedL2Transaction {
        chain_id: 1,
        nonce: 7,
        max_priority_fee_per_gas: 5,
        max_fee_per_gas: 500,
        gas_limit: 150_000,
        to: TxKind::Create,
        value: U256::from(4_500u64),
        data: Bytes::from(vec![0xbc; 40]),
        access_list: create_access_list(),
        from: Address::from_str("0x0000000000000000000000000000000000000eee").unwrap(),
        inner_hash: Default::default(),
    });

    group.bench_function("encode_privileged_l2_transaction", move |b| {
        let mut buf = Vec::with_capacity(privileged_tx.length());
        b.iter(|| {
            buf.clear();
            privileged_tx.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_fee_token_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    let fee_token_tx = black_box(FeeTokenTransaction {
        chain_id: 1,
        nonce: 8,
        max_priority_fee_per_gas: 6,
        max_fee_per_gas: 600,
        gas_limit: 160_000,
        to: TxKind::Call(Address::from_str("0x0000000000000000000000000000000000000fff").unwrap()),
        value: U256::from(5_500u64),
        data: Bytes::from(vec![0xde; 44]),
        access_list: create_access_list(),
        fee_token: Address::from_str("0x0000000000000000000000000000000000000fed").unwrap(),
        signature_y_parity: true,
        signature_r: U256::from(15u64),
        signature_s: U256::from(16u64),
        inner_hash: Default::default(),
    });

    group.bench_function("encode_fee_token_transaction", move |b| {
        let mut buf = Vec::with_capacity(fee_token_tx.length());
        b.iter(|| {
            buf.clear();
            fee_token_tx.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_p2p_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    let wrapped_tx = black_box(WrappedEIP4844Transaction {
        tx: EIP4844Transaction {
            chain_id: 1,
            nonce: 9,
            max_priority_fee_per_gas: 7,
            max_fee_per_gas: 700,
            gas: 170_000,
            to: Address::from_str("0x0000000000000000000000000000000000000abc").unwrap(),
            value: U256::from(6_500u64),
            data: Bytes::from(vec![0xef; 52]),
            access_list: create_access_list(),
            max_fee_per_blob_gas: U256::from(18u64),
            blob_versioned_hashes: vec![H256::repeat_byte(0x66)],
            signature_y_parity: false,
            signature_r: U256::from(17u64),
            signature_s: U256::from(18u64),
            inner_hash: Default::default(),
        },
        wrapper_version: Some(1),
        blobs_bundle: BlobsBundle {
            blobs: vec![[8u8; BYTES_PER_BLOB]],
            commitments: vec![[0x44u8; 48]],
            proofs: vec![[0x55u8; 48]],
            version: 1,
        },
    });

    let p2p_tx = P2PTransaction::EIP4844TransactionWithBlobs(wrapped_tx);

    group.bench_function("encode_p2p_transaction", move |b| {
        let mut buf = Vec::with_capacity(p2p_tx.length());
        b.iter(|| {
            buf.clear();
            p2p_tx.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_mempool_transaction(c: &mut Criterion) {
    let mut group = c.benchmark_group("transactions");

    let tx = black_box(Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 10,
        max_priority_fee_per_gas: 8,
        max_fee_per_gas: 800,
        gas_limit: 180_000,
        to: TxKind::Create,
        value: U256::from(7_500u64),
        data: Bytes::from(vec![0xaa; 36]),
        access_list: create_access_list(),
        signature_y_parity: true,
        signature_r: U256::from(19u64),
        signature_s: U256::from(20u64),
        inner_hash: Default::default(),
    }));
    let mempool_tx = black_box(MempoolTransaction::new(
        tx,
        Address::from_str("0x0000000000000000000000000000000000000cab").unwrap(),
    ));

    group.bench_function("encode_mempool_transaction", move |b| {
        let mut buf = Vec::with_capacity(mempool_tx.length());
        b.iter(|| {
            buf.clear();
            mempool_tx.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_p2p_endpoint(c: &mut Criterion) {
    let mut group = c.benchmark_group("networking_p2p");

    let endpoint = create_endpoint(10, 30303, 30303);

    group.bench_function("encode_endpoint", move |b| {
        let mut buf = Vec::with_capacity(endpoint.length());
        b.iter(|| {
            buf.clear();
            endpoint.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_p2p_node(c: &mut Criterion) {
    let mut group = c.benchmark_group("networking_p2p");

    let node = create_node(1);

    group.bench_function("encode_node", move |b| {
        let mut buf = Vec::with_capacity(node.length());
        b.iter(|| {
            buf.clear();
            node.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_node_record(c: &mut Criterion) {
    let mut group = c.benchmark_group("networking_p2p");

    let node_record = create_node_record();

    group.bench_function("encode_node_record", move |b| {
        let mut buf = Vec::with_capacity(node_record.length());
        b.iter(|| {
            buf.clear();
            node_record.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_ping_message(c: &mut Criterion) {
    let mut group = c.benchmark_group("networking_p2p");

    let from = create_endpoint(1, 30301, 30301);
    let to = create_endpoint(2, 30302, 30302);
    let ping = PingMessage::new(from, to, 1_700_000_000).with_enr_seq(42);

    group.bench_function("encode_ping_message", move |b| {
        let mut buf = Vec::with_capacity(ping.length());
        b.iter(|| {
            buf.clear();
            ping.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_find_node_message(c: &mut Criterion) {
    let mut group = c.benchmark_group("networking_p2p");

    let msg = FindNodeMessage::new(H512::repeat_byte(0x77), 1_700_000_000);

    group.bench_function("encode_find_node_message", move |b| {
        let mut buf = Vec::with_capacity(msg.length());
        b.iter(|| {
            buf.clear();
            msg.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_neighbors_message(c: &mut Criterion) {
    let mut group = c.benchmark_group("networking_p2p");

    let neighbors = NeighborsMessage::new(vec![create_node(3), create_node(4)], 1_700_000_000);

    group.bench_function("encode_neighbors_message", move |b| {
        let mut buf = Vec::with_capacity(neighbors.length());
        b.iter(|| {
            buf.clear();
            neighbors.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_enr_request_message(c: &mut Criterion) {
    let mut group = c.benchmark_group("networking_p2p");

    let msg = ENRRequestMessage::new(1_700_000_000);

    group.bench_function("encode_enr_request_message", move |b| {
        let mut buf = Vec::with_capacity(msg.length());
        b.iter(|| {
            buf.clear();
            msg.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_capability(c: &mut Criterion) {
    let mut group = c.benchmark_group("networking_p2p");

    let capability = Capability::eth(68);

    group.bench_function("encode_capability", move |b| {
        let mut buf = Vec::with_capacity(capability.length());
        b.iter(|| {
            buf.clear();
            capability.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_account_state_slim(c: &mut Criterion) {
    let mut group = c.benchmark_group("networking_p2p");

    let account_state = black_box(AccountStateSlim {
        nonce: 1,
        balance: U256::from(1000u64),
        storage_root: Bytes::from(vec![0xaa; 32]),
        code_hash: Bytes::from(vec![0xbb; 32]),
    });

    group.bench_function("encode_account_state_slim", move |b| {
        let mut buf = Vec::with_capacity(account_state.length());
        b.iter(|| {
            buf.clear();
            account_state.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_account_range_unit(c: &mut Criterion) {
    let mut group = c.benchmark_group("networking_p2p");

    let account_state = black_box(AccountStateSlim {
        nonce: 2,
        balance: U256::from(2_000u64),
        storage_root: Bytes::from(vec![0xcc; 32]),
        code_hash: Bytes::from(vec![0xdd; 32]),
    });

    let unit = black_box(AccountRangeUnit {
        hash: H256::repeat_byte(0x99),
        account: account_state,
    });

    group.bench_function("encode_account_range_unit", move |b| {
        let mut buf = Vec::with_capacity(unit.length());
        b.iter(|| {
            buf.clear();
            unit.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_storage_slot(c: &mut Criterion) {
    let mut group = c.benchmark_group("networking_p2p");

    let slot = black_box(StorageSlot {
        hash: H256::repeat_byte(0x42),
        data: U256::from(1234u64),
    });

    group.bench_function("encode_storage_slot", move |b| {
        let mut buf = Vec::with_capacity(slot.length());
        b.iter(|| {
            buf.clear();
            slot.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_nibbles(c: &mut Criterion) {
    let mut group = c.benchmark_group("trie");
    for &len in &[65usize, 129, 130, 500] {
        let label = format!("encode_nibbles_len_{len}");
        let nibbles = black_box(create_nibbles(len));
        group.bench_function(label, move |b| {
            let mut buf = Vec::with_capacity(nibbles.length());
            b.iter(|| {
                buf.clear();
                nibbles.encode(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_node_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("trie");

    let inline_hash = {
        let mut data = [0u8; 31];
        data[..10].fill(0x2a);
        NodeHash::Inline((data, 10))
    };
    let hashed_hash = NodeHash::from(H256::repeat_byte(0x42));

    for (label, node_hash) in [
        ("encode_node_hash_inline", inline_hash),
        ("encode_node_hash_hashed", hashed_hash),
    ] {
        let node_hash = black_box(node_hash);
        group.bench_function(label, move |b| {
            let mut buf = Vec::with_capacity(node_hash.length());
            b.iter(|| {
                buf.clear();
                let encoder = node_hash.encode(Encoder::new(&mut buf));
                black_box(&encoder);
                black_box(&buf);
            });
        });
    }

    group.finish();
}

fn bench_encode_branch_node(c: &mut Criterion) {
    let mut group = c.benchmark_group("trie");

    let mut choices = BranchNode::EMPTY_CHOICES;
    choices[0] = NodeHash::from(H256::repeat_byte(0x11)).into();
    choices[1] = TrieNode::from(LeafNode::new(create_nibbles(6), vec![0x33; 12])).into();
    choices[15] = NodeHash::from(H256::repeat_byte(0xaa)).into();

    let branch = black_box(BranchNode {
        choices,
        value: vec![0x55; 32],
    });

    group.bench_function("encode_branch_node", move |b| {
        let mut buf = Vec::with_capacity(branch.length());
        b.iter(|| {
            buf.clear();
            branch.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_extension_node(c: &mut Criterion) {
    let mut group = c.benchmark_group("trie");

    let extension = black_box(ExtensionNode {
        prefix: create_nibbles(20),
        child: NodeRef::from(NodeHash::from(H256::repeat_byte(0x53))),
    });

    group.bench_function("encode_extension_node", move |b| {
        let mut buf = Vec::with_capacity(extension.length());
        b.iter(|| {
            buf.clear();
            extension.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_leaf_node(c: &mut Criterion) {
    let mut group = c.benchmark_group("trie");

    let leaf = black_box(LeafNode::new(create_nibbles(18), vec![0x44; 40]));

    group.bench_function("encode_leaf_node", move |b| {
        let mut buf = Vec::with_capacity(leaf.length());
        b.iter(|| {
            buf.clear();
            leaf.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_fork_id(c: &mut Criterion) {
    let mut group = c.benchmark_group("common_types");

    let fork_id = black_box(ForkId {
        fork_hash: H32::from_slice(&[0xde, 0xad, 0xbe, 0xef]),
        fork_next: 17_000_000,
    });

    group.bench_function("encode_fork_id", move |b| {
        let mut buf = Vec::with_capacity(fork_id.length());
        b.iter(|| {
            buf.clear();
            fork_id.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_log(c: &mut Criterion) {
    let mut group = c.benchmark_group("common_types");

    let log_entry = black_box(Log {
        address: Address::from_str("0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba").unwrap(),
        topics: vec![HASH, H256::repeat_byte(0x11), H256::repeat_byte(0x22)],
        data: Bytes::from(vec![0x55; 128]),
    });

    group.bench_function("encode_log", move |b| {
        let mut buf = Vec::with_capacity(log_entry.length());
        b.iter(|| {
            buf.clear();
            log_entry.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_receipt(c: &mut Criterion) {
    let mut group = c.benchmark_group("common_types");

    let receipt_logs = black_box(vec![
        Log {
            address: Address::from_str("0x1000000000000000000000000000000000000001").unwrap(),
            topics: vec![HASH, H256::repeat_byte(0x33)],
            data: Bytes::from(vec![0xaa; 96]),
        },
        Log {
            address: Address::from_str("0x2000000000000000000000000000000000000002").unwrap(),
            topics: vec![H256::repeat_byte(0xbb)],
            data: Bytes::from(vec![0xbb; 64]),
        },
    ]);

    let receipt = black_box(Receipt {
        tx_type: TxType::EIP1559,
        succeeded: true,
        cumulative_gas_used: 120_000,
        logs: receipt_logs,
    });

    group.bench_function("encode_receipt", move |b| {
        let mut buf = Vec::with_capacity(receipt.length());
        b.iter(|| {
            buf.clear();
            receipt.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_receipt_with_bloom(c: &mut Criterion) {
    let mut group = c.benchmark_group("common_types");

    let logs_with_bloom = black_box(vec![
        Log {
            address: Address::from_str("0x3000000000000000000000000000000000000003").unwrap(),
            topics: vec![H256::repeat_byte(0x44), H256::repeat_byte(0x55)],
            data: Bytes::from(vec![0xcc; 80]),
        },
        Log {
            address: Address::from_str("0x4000000000000000000000000000000000000004").unwrap(),
            topics: vec![HASH],
            data: Bytes::from(vec![0xdd; 48]),
        },
    ]);

    let receipt = black_box(ReceiptWithBloom::new(
        TxType::EIP4844,
        true,
        240_000,
        logs_with_bloom,
    ));

    group.bench_function("encode_receipt_with_bloom", move |b| {
        let mut buf = Vec::with_capacity(receipt.length());
        b.iter(|| {
            buf.clear();
            receipt.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_encoded_requests(c: &mut Criterion) {
    let mut group = c.benchmark_group("common_types");

    let mut request_bytes: Vec<u8> = (0..192).map(|i| i as u8).collect();
    request_bytes.insert(0, 0x00);
    let encoded_requests = black_box(EncodedRequests(Bytes::from(request_bytes)));

    group.bench_function("encode_encoded_requests", move |b| {
        let mut buf = Vec::with_capacity(encoded_requests.length());
        b.iter(|| {
            buf.clear();
            encoded_requests.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_blobs_bundle(c: &mut Criterion) {
    let mut group = c.benchmark_group("common_types");

    let blobs_bundle = black_box(BlobsBundle {
        blobs: vec![[6u8; BYTES_PER_BLOB]; 4],
        commitments: vec![[0x78u8; 48]],
        proofs: vec![[0x78u8; 48]],
        version: 1,
    });

    group.bench_function("encode_blobs_bundle", move |b| {
        let mut buf = Vec::with_capacity(blobs_bundle.length());
        b.iter(|| {
            buf.clear();
            blobs_bundle.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_block_header(c: &mut Criterion) {
    let mut group = c.benchmark_group("common_types");

    let block_header = black_box(BlockHeader {
        parent_hash: H256::from_str(
            "0x48e29e7357408113a4166e04e9f1aeff0680daa2b97ba93df6512a73ddf7a154",
        )
        .unwrap(),
        ommers_hash: H256::from_str(
            "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        )
        .unwrap(),
        coinbase: Address::from_str("0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba").unwrap(),
        state_root: H256::from_str(
            "0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9",
        )
        .unwrap(),
        transactions_root: H256::from_str(
            "0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d",
        )
        .unwrap(),
        receipts_root: H256::from_str(
            "0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa",
        )
        .unwrap(),
        logs_bloom: Bloom::from([0; 256]),
        difficulty: U256::zero(),
        number: 1,
        gas_limit: 0x016345785d8a0000,
        gas_used: 0xa8de,
        timestamp: 0x03e8,
        extra_data: Bytes::new(),
        prev_randao: H256::zero(),
        nonce: 0x0000000000000000,
        base_fee_per_gas: Some(0x07),
        withdrawals_root: Some(
            H256::from_str("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                .unwrap(),
        ),
        blob_gas_used: Some(0x00),
        excess_blob_gas: Some(0x00),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(*EMPTY_KECCACK_HASH),
        ..Default::default()
    });

    group.bench_function("encode_block_header", move |b| {
        let mut buf = Vec::with_capacity(block_header.length());
        b.iter(|| {
            buf.clear();
            block_header.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_block(c: &mut Criterion) {
    let mut group = c.benchmark_group("common_types");

    let block_header = BlockHeader {
        parent_hash: H256::from_str(
            "0x48e29e7357408113a4166e04e9f1aeff0680daa2b97ba93df6512a73ddf7a154",
        )
        .unwrap(),
        ommers_hash: H256::from_str(
            "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        )
        .unwrap(),
        coinbase: Address::from_str("0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba").unwrap(),
        state_root: H256::from_str(
            "0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9",
        )
        .unwrap(),
        transactions_root: H256::from_str(
            "0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d",
        )
        .unwrap(),
        receipts_root: H256::from_str(
            "0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa",
        )
        .unwrap(),
        logs_bloom: Bloom::from([0; 256]),
        difficulty: U256::zero(),
        number: 1,
        gas_limit: 0x016345785d8a0000,
        gas_used: 0xa8de,
        timestamp: 0x03e8,
        extra_data: Bytes::new(),
        prev_randao: H256::zero(),
        nonce: 0x0000000000000000,
        base_fee_per_gas: Some(0x07),
        withdrawals_root: Some(
            H256::from_str("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                .unwrap(),
        ),
        blob_gas_used: Some(0x00),
        excess_blob_gas: Some(0x00),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(*EMPTY_KECCACK_HASH),
        ..Default::default()
    };

    let encoded_transactions = [
        "0x01f8d68330182404842daf517a830186a08080b880c1597f3c842558e64df52c3e0f0973067577c030c0c6578dbb2eef63155a21106fd4426057527f296b2ecdfabc81e34ffc82e89dec20f6b7c41fa1969d3c3bc44262c86f08b5b76077527fb7ece918787c50c878052c30a8b1d4abc07331e6d14b8ded52bbc58a6e9992b76097527f0110937c38cc13b914f201fc09dc6f7a80c001a09930cb92b4a27dce971c697a8c47fa34c98d076abc7b36e1239d6abcfc7c8403a041b35118447fe77c38c0b3a92a2dd3ecba4a9e4b35cc6534cd787f56c0cf2e21",
        "0xf86e81fa843127403882f61894db8d964741c53e55df9c2d4e9414c6c96482874e870aa87bee538000808360306ca03aa421df67a101c45ff9cb06ce28f518a5d8d8dbb76a79361280071909650a27a05a447ff053c4ae601cfe81859b58d5603f2d0a73481c50f348089032feb0b073",
        "0x02f8ef83301824048413f157f8842daf517a830186a094000000000000000000000000000000000000000080b8807a0a600060a0553db8600060c855c77fb29ecd7661d8aefe101a0db652a728af0fded622ff55d019b545d03a7532932a60ad52604260cd5360bf60ce53609460cf53603e60d05360f560d153bc596000609e55600060c6556000601f556000609155535660556057536055605853606e60595360e7605a5360d0605b5360eb60c080a03acb03b1fc20507bc66210f7e18ff5af65038fb22c626ae488ad9513d9b6debca05d38459e9d2a221eb345b0c2761b719b313d062ff1ea3d10cf5b8762c44385a6",
        "0x01f8ea8330182402842daf517a830186a094000000000000000000000000000000000000000080b880bdb30d976000604e557145600060a155d67fe7e473caf6e33cba341136268fc1189ba07837ef8a266570289ff53afc43436260c7527f333dfe837f4838f6053e5e46e4151aeec28f356ec39a2db9769f36ec92e3e3f660e7527f0b261608674300d4621eff679096a6ed786591aca69f2b22a3ea6949621daade610107527f3cc080a01f3f906540fb56b0576c51b3ffa86df213fd1f407378c9441cfdd9d5f3c1df3da035691b16c053b68ec74683ae020293cbc6a47ac773dc8defb96cb680c576e5a3",
    ];
    let transactions: Vec<Transaction> = encoded_transactions
        .iter()
        .map(|hex| {
            Transaction::decode_canonical(&hex::decode(hex.trim_start_matches("0x")).unwrap())
                .unwrap()
        })
        .collect();

    let block = black_box(Block {
        header: block_header.clone(),
        body: BlockBody {
            transactions: transactions,
            ommers: vec![block_header],
            withdrawals: Some(vec![
                Withdrawal {
                    index: 0x00,
                    validator_index: 0x00,
                    address: H160::repeat_byte(0xf9),
                    amount: 0x00_u64,
                };
                4
            ]),
        },
    });

    group.bench_function("encode_block", move |b| {
        let mut buf = Vec::with_capacity(block.length());
        b.iter(|| {
            buf.clear();
            block.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_withdrawals(c: &mut Criterion) {
    let mut group = c.benchmark_group("common_types");

    let withdrawal = black_box(Withdrawal {
        index: 0x00,
        validator_index: 0x00,
        address: H160::repeat_byte(0xf9),
        amount: 0x80_u64,
    });

    group.bench_function("encode_withdrawals", move |b| {
        let mut buf = Vec::with_capacity(withdrawal.length());
        b.iter(|| {
            buf.clear();
            withdrawal.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_encode_integer,
    bench_encode_strings,
    bench_encode_int_lists,
    bench_encode_string_lists,
    bench_encode_account_info,
    bench_encode_account_state,
    bench_encode_p2p_endpoint,
    bench_encode_p2p_node,
    bench_encode_node_record,
    bench_encode_ping_message,
    bench_encode_find_node_message,
    bench_encode_neighbors_message,
    bench_encode_enr_request_message,
    bench_encode_capability,
    bench_encode_account_state_slim,
    bench_encode_account_range_unit,
    bench_encode_storage_slot,
    bench_encode_nibbles,
    bench_encode_node_hash,
    bench_encode_branch_node,
    bench_encode_extension_node,
    bench_encode_leaf_node,
    bench_encode_tx_kind,
    bench_encode_legacy_transaction,
    bench_encode_eip2930_transaction,
    bench_encode_eip1559_transaction,
    bench_encode_eip4844_transaction,
    bench_encode_wrapped_eip4844_transaction,
    bench_encode_eip7702_transaction,
    bench_encode_privileged_l2_transaction,
    bench_encode_fee_token_transaction,
    bench_encode_p2p_transaction,
    bench_encode_mempool_transaction,
    bench_encode_fork_id,
    bench_encode_log,
    bench_encode_receipt,
    bench_encode_receipt_with_bloom,
    bench_encode_encoded_requests,
    bench_encode_blobs_bundle,
    bench_encode_block_header,
    bench_encode_block,
    bench_encode_withdrawals
);
criterion_main!(benches);
