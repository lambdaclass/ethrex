use super::transaction::RpcTransaction;
use ethrex_core::{
    serde_utils,
    types::{Block, BlockBody, BlockHash, BlockHeader, BlockNumber, Withdrawal},
    H256, U256,
};
use ethrex_rlp::encode::RLPEncode;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcBlock {
    hash: H256,
    #[serde(with = "serde_utils::u64::hex_str")]
    size: u64,
    // TODO (#307): Remove TotalDifficulty.
    total_difficulty: U256,
    #[serde(flatten)]
    pub header: BlockHeader,
    #[serde(flatten)]
    pub body: BlockBodyWrapper,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BlockBodyWrapper {
    Full(FullBlockBody),
    OnlyHashes(OnlyHashesBlockBody),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FullBlockBody {
    pub transactions: Vec<RpcTransaction>,
    pub uncles: Vec<BlockHeader>,
    pub withdrawals: Vec<Withdrawal>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OnlyHashesBlockBody {
    // Only tx hashes
    pub transactions: Vec<H256>,
    pub uncles: Vec<BlockHeader>,
    pub withdrawals: Vec<Withdrawal>,
}

impl RpcBlock {
    pub fn build(
        header: BlockHeader,
        body: BlockBody,
        hash: H256,
        full_transactions: bool,
        total_difficulty: U256,
    ) -> RpcBlock {
        let size = Block::new(header.clone(), body.clone())
            .encode_to_vec()
            .len();
        let body_wrapper = if full_transactions {
            BlockBodyWrapper::Full(FullBlockBody::from_body(body, header.number, hash))
        } else {
            BlockBodyWrapper::OnlyHashes(OnlyHashesBlockBody {
                transactions: body.transactions.iter().map(|t| t.compute_hash()).collect(),
                uncles: body.ommers,
                withdrawals: body.withdrawals.unwrap_or_default(),
            })
        };

        RpcBlock {
            hash,
            total_difficulty,
            size: size as u64,
            header,
            body: body_wrapper,
        }
    }
}

impl FullBlockBody {
    pub fn from_body(
        body: BlockBody,
        block_number: BlockNumber,
        block_hash: BlockHash,
    ) -> FullBlockBody {
        let mut transactions = Vec::new();
        for (index, tx) in body.transactions.iter().enumerate() {
            transactions.push(RpcTransaction::build(
                tx.clone(),
                block_number,
                block_hash,
                index,
            ));
        }
        FullBlockBody {
            transactions,
            uncles: body.ommers,
            withdrawals: body.withdrawals.unwrap_or_default(),
        }
    }
}
#[cfg(test)]
mod test {

    use bytes::Bytes;
    use ethrex_core::{
        types::{EIP1559Transaction, Transaction, TxKind},
        Address, Bloom, H256, U256,
    };
    use std::str::FromStr;

    use super::*;

    #[test]
    fn serialize_block() {
        let block_header = BlockHeader {
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
            number: 1,
            gas_limit: 0x016345785d8a0000,
            gas_used: 0xa8de,
            timestamp: 0x03e8,
            extra_data: Bytes::new(),
            prev_randao: H256::zero(),
            nonce: 0x0000000000000000,
            base_fee_per_gas: Some(0x07),
            withdrawals_root: Some(
                H256::from_str(
                    "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
                )
                .unwrap(),
            ),
            blob_gas_used: Some(0x00),
            excess_blob_gas: Some(0x00),
            parent_beacon_block_root: Some(H256::zero()),
        };

        let tx = EIP1559Transaction {
            nonce: 0,
            max_fee_per_gas: 78,
            max_priority_fee_per_gas: 17,
            to: TxKind::Call(Address::from_slice(
                &hex::decode("6177843db3138ae69679A54b95cf345ED759450d").unwrap(),
            )),
            value: 3000000000000000_u64.into(),
            data: Bytes::from_static(b"0x1568"),
            signature_r: U256::from_str_radix(
                "151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65d",
                16,
            )
            .unwrap(),
            signature_s: U256::from_str_radix(
                "64c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4",
                16,
            )
            .unwrap(),
            signature_y_parity: false,
            chain_id: 3151908,
            gas_limit: 63000,
            access_list: vec![(
                Address::from_slice(
                    &hex::decode("6177843db3138ae69679A54b95cf345ED759450d").unwrap(),
                ),
                vec![],
            )],
        };

        let block_body = BlockBody {
            transactions: vec![Transaction::EIP1559Transaction(tx)],
            ommers: vec![],
            withdrawals: Some(vec![]),
        };
        let hash = block_header.compute_block_hash();

        let block = RpcBlock::build(block_header, block_body, hash, true, U256::zero());
        let expected_block = r#"{"hash":"0x63d6a2504601fc2db0ccf02a28055eb0cdb40c444ecbceec0f613980421a035e","size":"0x2d6","totalDifficulty":"0x0","parentHash":"0x1ac1bf1eef97dc6b03daba5af3b89881b7ae4bc1600dc434f450a9ec34d44999","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","miner":"0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba","stateRoot":"0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9","transactionsRoot":"0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d","receiptsRoot":"0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","difficulty":"0x0","number":"0x1","gasLimit":"0x16345785d8a0000","gasUsed":"0xa8de","timestamp":"0x3e8","extraData":"0x","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","baseFeePerGas":"0x7","withdrawalsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","blobGasUsed":"0x0","excessBlobGas":"0x0","parentBeaconBlockRoot":"0x0000000000000000000000000000000000000000000000000000000000000000","transactions":[{"type":"0x2","nonce":"0x0","to":"0x6177843db3138ae69679a54b95cf345ed759450d","gas":"0xf618","value":"0xaa87bee538000","input":"0x307831353638","maxPriorityFeePerGas":"0x11","maxFeePerGas":"0x4e","gasPrice":"0x4e","accessList":[{"address":"0x6177843db3138ae69679a54b95cf345ed759450d","storageKeys":[]}],"chainId":"0x301824","yParity":"0x0","v":"0x0","r":"0x151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65d","s":"0x64c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4","blockNumber":"0x1","blockHash":"0x63d6a2504601fc2db0ccf02a28055eb0cdb40c444ecbceec0f613980421a035e","from":"0x35af8ea983a3ba94c655e19b82b932a30d6b9558","hash":"0x0b8c8f37731d9493916b06d666c3fd5dee2c3bbda06dfe866160d717e00dda91","transactionIndex":"0x0"}],"uncles":[],"withdrawals":[]}"#;
        assert_eq!(serde_json::to_string(&block).unwrap(), expected_block)
    }
}
