pub(crate) mod account;
pub(crate) mod block;
pub(crate) mod client;
pub(crate) mod fee_market;
pub(crate) mod filter;
pub(crate) mod logs;
pub(crate) mod transaction;

mod fee_calculator;
pub(crate) mod gas_price;
pub(crate) mod max_priority_fee;

#[cfg(test)]
pub mod test_utils {
    use bytes::Bytes;
    use ethrex_core::{
        types::{
            Block, BlockBody, BlockHeader, EIP1559Transaction, Genesis, LegacyTransaction, Transaction, TxKind, DEFAULT_REQUESTS_HASH, EMPTY_KECCACK_HASH
        },
        Address, Bloom, H256, U256,
    };
    use ethrex_storage::{EngineType, Store};
    use hex_literal::hex;
    use std::str::FromStr;

    // Base price for each test transaction.
    pub const BASE_PRICE_IN_WEI: u64 = 10_u64.pow(9);

    fn test_header(block_num: u64) -> BlockHeader {
        BlockHeader {
            parent_hash: H256::from_str(
                "0x1ac1bf1eef97dc6b03daba5af3b89881b7ae4bc1600dc434f450a9ec34d44999",
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
            number: block_num,
            gas_limit: 0x016345785d8a0000,
            gas_used: 0xa8de,
            timestamp: 0x03e8,
            extra_data: Bytes::new(),
            prev_randao: H256::zero(),
            nonce: 0x0000000000000000,
            base_fee_per_gas: Some(BASE_PRICE_IN_WEI),
            withdrawals_root: Some(
                H256::from_str(
                    "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
                )
                .unwrap(),
            ),
            blob_gas_used: Some(0x00),
            excess_blob_gas: Some(0x00),
            parent_beacon_block_root: Some(H256::zero()),
            requests_hash: Some(*DEFAULT_REQUESTS_HASH),
        }
    }

    fn add_blocks_with_transactions(
        storage: &Store,
        block_count: u64,
        txs_per_block: Vec<Transaction>,
    ) {
        for block_num in 1..=block_count {
            let block_body = BlockBody {
                transactions: txs_per_block.clone(),
                ommers: Default::default(),
                withdrawals: Default::default(),
            };
            let block_header = test_header(block_num);
            let block = Block::new(block_header.clone(), block_body);
            storage.add_block(block).unwrap();
            storage
                .set_canonical_block(block_num, block_header.compute_block_hash())
                .unwrap();
            storage.update_latest_block_number(block_num).unwrap();
        }
    }

    fn legacy_tx_for_test(nonce: u64) -> Transaction {
        Transaction::LegacyTransaction(LegacyTransaction {
            nonce,
            gas_price: nonce * BASE_PRICE_IN_WEI,
            gas: 10000,
            to: TxKind::Create,
            value: 100.into(),
            data: Default::default(),
            v: U256::from(0x1b),
            r: U256::from_big_endian(&hex!(
                "7e09e26678ed4fac08a249ebe8ed680bf9051a5e14ad223e4b2b9d26e0208f37"
            )),
            s: U256::from_big_endian(&hex!(
                "5f6e3f188e3e6eab7d7d3b6568f5eac7d687b08d307d3154ccd8c87b4630509b"
            )),
        })
    }
    fn eip1559_tx_for_test(nonce: u64) -> Transaction {
        Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id: 1,
            nonce,
            max_fee_per_gas: nonce * BASE_PRICE_IN_WEI,
            // This is less than gas_price in legacy txs because we should add base_fee to it
            // base_fee is 10^9, so (nonce - 1) * 10^9 + base_fee equals the legacy gas_price
            // for the same nonce. For consistency, we use the same value here.
            max_priority_fee_per_gas: (nonce - 1) * BASE_PRICE_IN_WEI,
            gas_limit: 10000,
            to: TxKind::Create,
            value: 100.into(),
            data: Default::default(),
            access_list: vec![],
            signature_y_parity: true,
            signature_r: U256::default(),
            signature_s: U256::default(),
        })
    }

    pub fn setup_store() -> Store {
        let genesis: &str = include_str!("../../../../test_data/genesis-l1.json");
        let genesis: Genesis =
            serde_json::from_str(genesis).expect("Fatal: test config is invalid");
        let store = Store::new("test-store", EngineType::InMemory)
            .expect("Fail to create in-memory db test");
        store.add_initial_state(genesis).unwrap();
        store
    }

    pub fn add_legacy_tx_blocks(storage: &Store, block_count: u64, tx_count: u64) {
        for block_num in 1..=block_count {
            let mut txs = vec![];
            for nonce in 1..=tx_count {
                txs.push(legacy_tx_for_test(nonce));
            }
            add_blocks_with_transactions(storage, block_num, txs);
        }
    }

    pub fn add_eip1559_tx_blocks(storage: &Store, block_count: u64, tx_count: u64) {
        for block_num in 1..=block_count {
            let mut txs = vec![];
            for nonce in 1..=tx_count {
                txs.push(eip1559_tx_for_test(nonce));
            }
            add_blocks_with_transactions(storage, block_num, txs);
        }
    }

    pub fn add_mixed_tx_blocks(storage: &Store, block_count: u64, tx_count: u64) {
        for block_num in 1..=block_count {
            let mut txs = vec![];
            for nonce in 1..=tx_count {
                if nonce % 2 == 0 {
                    txs.push(legacy_tx_for_test(nonce));
                } else {
                    txs.push(eip1559_tx_for_test(nonce));
                }
            }
            add_blocks_with_transactions(storage, block_num, txs);
        }
    }
}
