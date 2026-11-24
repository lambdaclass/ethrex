use super::transaction::RpcTransaction;
use ethrex_common::{
    H256, serde_utils,
    types::{Block, BlockBody, BlockHash, BlockHeader, BlockNumber, Withdrawal},
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::encode::RLPEncode;

use crate::utils::RpcErr;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcBlock {
    pub hash: H256,
    #[serde(with = "serde_utils::u64::hex_str")]
    pub size: u64,
    #[serde(flatten)]
    pub header: BlockHeader,
    #[serde(flatten)]
    pub body: BlockBodyWrapper,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcHeader {
    pub hash: H256,
    #[serde(flatten)]
    pub header: BlockHeader,
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
    pub uncles: Vec<H256>,
    pub withdrawals: Vec<Withdrawal>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OnlyHashesBlockBody {
    // Only tx hashes
    pub transactions: Vec<H256>,
    pub uncles: Vec<H256>,
    pub withdrawals: Vec<Withdrawal>,
}

impl TryInto<Block> for RpcBlock {
    type Error = String;

    fn try_into(self) -> Result<Block, Self::Error> {
        let block_body = if let BlockBodyWrapper::Full(body) = self.body {
            body
        } else {
            return Err("Expected full block body from RPC".to_owned());
        };

        let transactions = block_body.transactions.into_iter().map(|t| t.tx).collect();

        Ok(Block {
            header: self.header,
            body: BlockBody {
                transactions,
                ommers: Vec::new(),
                withdrawals: Some(block_body.withdrawals),
            },
        })
    }
}

impl RpcBlock {
    pub fn build(
        header: BlockHeader,
        body: BlockBody,
        hash: H256,
        full_transactions: bool,
    ) -> Result<RpcBlock, RpcErr> {
        let size = Block::new(header.clone(), body.clone()).length();
        let body_wrapper = if full_transactions {
            BlockBodyWrapper::Full(FullBlockBody::from_body(body, header.number, hash)?)
        } else {
            BlockBodyWrapper::OnlyHashes(OnlyHashesBlockBody {
                transactions: body
                    .transactions
                    .iter()
                    .map(|t| t.hash(&NativeCrypto))
                    .collect(),
                uncles: body.ommers.iter().map(|ommer| ommer.hash()).collect(),
                withdrawals: body.withdrawals.unwrap_or_default(),
            })
        };

        Ok(RpcBlock {
            hash,
            size: size as u64,
            header,
            body: body_wrapper,
        })
    }
}

impl FullBlockBody {
    pub fn from_body(
        body: BlockBody,
        block_number: BlockNumber,
        block_hash: BlockHash,
    ) -> Result<FullBlockBody, RpcErr> {
        let mut transactions = Vec::new();
        for (index, tx) in body.transactions.iter().enumerate() {
            transactions.push(RpcTransaction::build(
                tx.clone(),
                Some(block_number),
                Some(block_hash),
                Some(index),
            )?);
        }
        Ok(FullBlockBody {
            transactions,
            uncles: body.ommers.iter().map(|ommer| ommer.hash()).collect(),
            withdrawals: body.withdrawals.unwrap_or_default(),
        })
    }
}
#[cfg(test)]
mod test {

    use bytes::Bytes;
    use ethrex_common::{
        Address, Bloom, H256, U256,
        constants::EMPTY_KECCAK_HASH,
        types::{EIP1559Transaction, Transaction, TxKind},
    };
    use std::str::FromStr;

    use super::*;

    #[test]
    fn serialize_block() {
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
                H256::from_str(
                    "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
                )
                .unwrap(),
            ),
            blob_gas_used: Some(0x00),
            excess_blob_gas: Some(0x00),
            parent_beacon_block_root: Some(H256::zero()),
            requests_hash: Some(*EMPTY_KECCAK_HASH),
            ..Default::default()
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
            signature_y_parity: 0u8.into(),
            chain_id: 3151908,
            gas_limit: 63000,
            access_list: vec![(
                Address::from_slice(
                    &hex::decode("6177843db3138ae69679A54b95cf345ED759450d").unwrap(),
                ),
                vec![],
            )],
            ..Default::default()
        };

        let block_body = BlockBody {
            transactions: vec![Transaction::EIP1559Transaction(tx)],
            ommers: vec![],
            withdrawals: Some(vec![]),
        };
        let hash = block_header.hash();

        let block = RpcBlock::build(block_header, block_body, hash, true).unwrap();
        let expected_block = r#"{"hash":"0x94fb81ef7259ad4cef032745a2a5254babe26037f2850d320b872692f7c60178","size":"0x2f7","parentHash":"0x48e29e7357408113a4166e04e9f1aeff0680daa2b97ba93df6512a73ddf7a154","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","miner":"0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba","stateRoot":"0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9","transactionsRoot":"0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d","receiptsRoot":"0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","difficulty":"0x0","number":"0x1","gasLimit":"0x16345785d8a0000","gasUsed":"0xa8de","timestamp":"0x3e8","extraData":"0x","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","baseFeePerGas":"0x7","withdrawalsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","blobGasUsed":"0x0","excessBlobGas":"0x0","parentBeaconBlockRoot":"0x0000000000000000000000000000000000000000000000000000000000000000","requestsHash":"0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470","transactions":[{"type":"0x2","nonce":"0x0","to":"0x6177843db3138ae69679a54b95cf345ed759450d","gas":"0xf618","value":"0xaa87bee538000","input":"0x307831353638","maxPriorityFeePerGas":"0x11","maxFeePerGas":"0x4e","gasPrice":"0x4e","accessList":[{"address":"0x6177843db3138ae69679a54b95cf345ed759450d","storageKeys":[]}],"chainId":"0x301824","yParity":"0x0","v":"0x0","r":"0x151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65d","s":"0x64c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4","blockNumber":"0x1","blockHash":"0x94fb81ef7259ad4cef032745a2a5254babe26037f2850d320b872692f7c60178","from":"0x35af8ea983a3ba94c655e19b82b932a30d6b9558","hash":"0x0b8c8f37731d9493916b06d666c3fd5dee2c3bbda06dfe866160d717e00dda91","transactionIndex":"0x0"}],"uncles":[],"withdrawals":[]}"#;
        assert_eq!(serde_json::to_string(&block).unwrap(), expected_block)
    }

    /// eth_getHeaderBy* results from the execution-apis fixtures for a header
    /// before London (genesis), before Shanghai (the merge) and before Cancun:
    /// each must hash to the fixture's hash and serialize back to the fixture,
    /// so the fields of later forks are omitted rather than null.
    #[test]
    fn serialize_header_omits_fields_before_their_fork() {
        for fixture in [
            r#"{"difficulty":"0x20000","extraData":"0x68697665636861696e","gasLimit":"0x5f5e100","gasUsed":"0x0","hash":"0x202f4eff9f2f9447e497280d37eca304ec075091bc3c694f887cfe067f9cd43f","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","miner":"0x0000000000000000000000000000000000000000","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","number":"0x0","parentHash":"0x0000000000000000000000000000000000000000000000000000000000000000","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","stateRoot":"0x633b0d275a5fbc4a1fd106d1f90a0bfec5c97481689be3dcb742c23ce45ca1da","timestamp":"0x0","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"}"#,
            r#"{"baseFeePerGas":"0x11f8ad13","difficulty":"0x0","extraData":"0x","gasLimit":"0xbebc200","gasUsed":"0x52f67","hash":"0x7e1aabc66b44ff069567f0e989331773e43080a7178b6c54b2df8156cbadcfe9","logsBloom":"0x000000000000000000800040000000000000000000000400000000008000000000000000000010000000000040100000000000000200000000000000000000010000000400000000000040000000000003000000000000000000000000000020000000010a0000000000008000000000000200000000000000200000000004000000000000000000000004000000000000800000000010000000000000000000000000000000009c00800000000100048000000000000000008000000002800000400040000000000000000000000000000000000000000000000000001001000000200000000000000000000000000000000000000020000000000000000001","miner":"0x0000000000000000000000000000000000000000","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","number":"0x24","parentHash":"0x51eae114c4d4bceabc24e65005c05d2e9ad5ac54c1bbc5aadb71da8ed20a7eee","receiptsRoot":"0xb6a7d7803aadb8ef1ecd55c6abe8b31c492c1700c54c573e0e6e2c1573fc9a72","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","stateRoot":"0xfa0387c4ffb63a30449f5a285f1fe956df38c9cc8b67bb65afbac476667c0b1d","timestamp":"0x168","transactionsRoot":"0x90cf0cbc3cf2081deeebe3bdb3520b4f99b72a790dbddbffc35347f08f084ccd"}"#,
            r#"{"baseFeePerGas":"0xc0d8ef9","difficulty":"0x0","extraData":"0x","gasLimit":"0xbebc200","gasUsed":"0x16eac","hash":"0x96bd017a6b8670f5f11f709cb878722f6645d03c710a3cce03c3d48aec87acfc","logsBloom":"0x00000000000000000000000000000000000000000000004000000000800000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004000000000000200000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000009000000000000000000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000","miner":"0x0000000000000000000000000000000000000000","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","number":"0x27","parentHash":"0xacec2ab89369106be1f6ffae88b1e9ecef7a7f903f05688cd51cac05066cb59f","receiptsRoot":"0x682673c321431d2d4429bccb9b178f61c4a06e101a85a6e9c8fc7cf5e8c1276b","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","stateRoot":"0x95e4413aab05ed5110340dc2f91a1740f7bbd11837b8453658aa6b34da97e0bf","timestamp":"0x186","transactionsRoot":"0xe111ae90cbae117bd8fb16e0bde612fb475f92790aec83bc7235401f86fa7ad2","withdrawalsRoot":"0x0cbd4206b2131f21f541589e2bb2ad8cfceb1875f04acde910ea91933245f92b"}"#,
        ] {
            let expected: serde_json::Value = serde_json::from_str(fixture).unwrap();
            let header: BlockHeader = serde_json::from_value(expected.clone()).unwrap();
            let hash = header.hash();
            assert_eq!(serde_json::json!(format!("{hash:#x}")), expected["hash"]);
            let header = RpcHeader { hash, header };
            assert_eq!(serde_json::to_value(&header).unwrap(), expected);
        }
    }

    /// A null fork field still deserializes, as before fields were skipped.
    #[test]
    fn deserialize_header_accepts_null_fork_fields() {
        let mut fixture: serde_json::Value = serde_json::from_str(r#"{"baseFeePerGas":"0xc0d8ef9","difficulty":"0x0","extraData":"0x","gasLimit":"0xbebc200","gasUsed":"0x16eac","hash":"0x96bd017a6b8670f5f11f709cb878722f6645d03c710a3cce03c3d48aec87acfc","logsBloom":"0x00000000000000000000000000000000000000000000004000000000800000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004000000000000200000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000009000000000000000000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000","miner":"0x0000000000000000000000000000000000000000","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","number":"0x27","parentHash":"0xacec2ab89369106be1f6ffae88b1e9ecef7a7f903f05688cd51cac05066cb59f","receiptsRoot":"0x682673c321431d2d4429bccb9b178f61c4a06e101a85a6e9c8fc7cf5e8c1276b","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","stateRoot":"0x95e4413aab05ed5110340dc2f91a1740f7bbd11837b8453658aa6b34da97e0bf","timestamp":"0x186","transactionsRoot":"0xe111ae90cbae117bd8fb16e0bde612fb475f92790aec83bc7235401f86fa7ad2","withdrawalsRoot":"0x0cbd4206b2131f21f541589e2bb2ad8cfceb1875f04acde910ea91933245f92b"}"#).unwrap();
        fixture["parentBeaconBlockRoot"] = serde_json::Value::Null;
        fixture["blobGasUsed"] = serde_json::Value::Null;
        let header: BlockHeader = serde_json::from_value(fixture).unwrap();
        assert!(header.parent_beacon_block_root.is_none());
        assert!(header.blob_gas_used.is_none());
    }
}
