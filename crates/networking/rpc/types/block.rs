use super::transaction::RpcTransaction;
use bytes::Bytes;
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
    /// EIP-8288's block proof, served with full transactions only, as the reference
    /// client serves it.
    #[serde(
        rename = "starkProof",
        skip_serializing_if = "Option::is_none",
        with = "super::payload::optional_hex_bytes",
        default
    )]
    pub stark_proof: Option<Bytes>,
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
                stark_proof: block_body.stark_proof,
                transactions,
                ommers: Vec::new(),
                withdrawals: Some(block_body.withdrawals),
            },
        })
    }
}

impl RpcBlock {
    pub fn build(
        mut header: BlockHeader,
        body: BlockBody,
        hash: H256,
        full_transactions: bool,
    ) -> Result<RpcBlock, RpcErr> {
        let size = Block::new(header.clone(), body.clone()).length();
        // EIP-7843's slot number arrives with EIP-7928's block access list hash.
        // Without that hash the header's slot is only the zero placeholder that
        // positions later fields, such as EIP-8288's, so it is not served; the
        // encoding puts it back.
        if header.block_access_list_hash.is_none() {
            header.slot_number = None;
        }
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
            stark_proof: body.stark_proof,
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
            signature_y_parity: false,
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
            stark_proof: None,
            transactions: vec![Transaction::EIP1559Transaction(tx)],
            ommers: vec![],
            withdrawals: Some(vec![]),
        };
        let hash = block_header.hash();

        let block = RpcBlock::build(block_header, block_body, hash, true).unwrap();
        let expected_block = r#"{"hash":"0x94fb81ef7259ad4cef032745a2a5254babe26037f2850d320b872692f7c60178","size":"0x2f7","parentHash":"0x48e29e7357408113a4166e04e9f1aeff0680daa2b97ba93df6512a73ddf7a154","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","miner":"0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba","stateRoot":"0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9","transactionsRoot":"0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d","receiptsRoot":"0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","difficulty":"0x0","number":"0x1","gasLimit":"0x16345785d8a0000","gasUsed":"0xa8de","timestamp":"0x3e8","extraData":"0x","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","baseFeePerGas":"0x7","withdrawalsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","blobGasUsed":"0x0","excessBlobGas":"0x0","parentBeaconBlockRoot":"0x0000000000000000000000000000000000000000000000000000000000000000","requestsHash":"0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470","transactions":[{"type":"0x2","nonce":"0x0","to":"0x6177843db3138ae69679a54b95cf345ed759450d","gas":"0xf618","value":"0xaa87bee538000","input":"0x307831353638","maxPriorityFeePerGas":"0x11","maxFeePerGas":"0x4e","gasPrice":"0x4e","accessList":[{"address":"0x6177843db3138ae69679a54b95cf345ed759450d","storageKeys":[]}],"chainId":"0x301824","yParity":"0x0","v":"0x0","r":"0x151ccc02146b9b11adf516e6787b59acae3e76544fdcd75e77e67c6b598ce65d","s":"0x64c5dd5aae2fbb535830ebbdad0234975cd7ece3562013b63ea18cc0df6c97d4","blockNumber":"0x1","blockHash":"0x94fb81ef7259ad4cef032745a2a5254babe26037f2850d320b872692f7c60178","from":"0x35af8ea983a3ba94c655e19b82b932a30d6b9558","hash":"0x0b8c8f37731d9493916b06d666c3fd5dee2c3bbda06dfe866160d717e00dda91","transactionIndex":"0x0"}],"uncles":[],"withdrawals":[]}"#;
        assert_eq!(serde_json::to_string(&block).unwrap(), expected_block)
    }
}

#[cfg(test)]
mod daisugi_json_tests {
    use super::*;
    use ethrex_common::types::{
        RecursiveStark, compute_transactions_root, compute_withdrawals_root,
    };

    /// A block after the Daisugi fork, as storage returns it: the header carries the
    /// slot placeholder the encoding decoded, and the body its proof.
    fn daisugi_block() -> Block {
        let proof = Bytes::from(vec![0xAB; 40]);
        let body = BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(vec![]),
            stark_proof: Some(proof.clone()),
        };
        let header = BlockHeader {
            transactions_root: compute_transactions_root(&[], &NativeCrypto),
            withdrawals_root: Some(compute_withdrawals_root(&[], &NativeCrypto)),
            base_fee_per_gas: Some(7),
            blob_gas_used: Some(0),
            excess_blob_gas: Some(0),
            parent_beacon_block_root: Some(H256::repeat_byte(1)),
            requests_hash: Some(H256::repeat_byte(2)),
            slot_number: Some(0),
            recursive_stark: Some(RecursiveStark::commit(&proof, body.block_deps_hash())),
            ..Default::default()
        };
        Block::new(header, body)
    }

    #[test]
    fn a_daisugi_block_serves_its_commitments_flat_and_its_proof_with_full_transactions() {
        let block = daisugi_block();
        let commitment = block.header.recursive_stark.clone().unwrap();
        for full in [false, true] {
            let rpc = RpcBlock::build(block.header.clone(), block.body.clone(), block.hash(), full)
                .unwrap();
            let json = serde_json::to_value(&rpc).unwrap();
            assert_eq!(
                json["starkProofHash"],
                serde_json::to_value(commitment.stark_proof_hash).unwrap()
            );
            assert_eq!(
                json["blockDepsHash"],
                serde_json::to_value(commitment.block_deps_hash).unwrap()
            );
            assert!(json.get("recursiveStark").is_none());
            assert!(
                json.get("slotNumber").is_none(),
                "the slot is only a placeholder"
            );
            assert_eq!(json.get("starkProof").is_some(), full);
            assert_eq!(
                json["size"],
                serde_json::Value::String(format!("{:#x}", block.length()))
            );
        }
    }

    #[test]
    fn a_full_daisugi_block_read_back_from_json_hashes_the_same() {
        let block = daisugi_block();
        let rpc =
            RpcBlock::build(block.header.clone(), block.body.clone(), block.hash(), true).unwrap();
        let json = serde_json::to_string(&rpc).unwrap();
        let read: RpcBlock = serde_json::from_str(&json).unwrap();
        let rebuilt: Block = read.try_into().unwrap();
        assert_eq!(rebuilt.hash(), block.hash());
        assert_eq!(rebuilt.body.stark_proof, block.body.stark_proof);
    }
}
