use crate::{
    rlp::{encode::RLPEncode, structs},
    Address, H256, U256,
};
use bytes::Bytes;
use patricia_merkle_tree::PatriciaMerkleTree;
use sha3::Keccak256;

pub type BlockNumber = u64;
pub type Bloom = [u8; 256];

/// Header part of a block on the chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockHeader {
    parent_hash: H256,
    ommers_hash: H256,
    coinbase: Address,
    state_root: H256,
    transactions_root: H256,
    receipt_root: H256,
    logs_bloom: Bloom,
    difficulty: U256,
    number: BlockNumber,
    gas_limit: u64,
    gas_used: u64,
    timestamp: u64,
    extra_data: Bytes,
    prev_randao: H256,
    nonce: u64,
    base_fee_per_gas: u64,
    withdrawals_root: H256,
    blob_gas_used: u64,
    excess_blob_gas: u64,
    parent_beacon_block_root: H256,
}

impl RLPEncode for BlockHeader {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        self.parent_hash.encode(buf);
        self.ommers_hash.encode(buf);
        self.coinbase.encode(buf);
        self.state_root.encode(buf);
        self.transactions_root.encode(buf);
        self.receipt_root.encode(buf);
        self.logs_bloom.encode(buf);
        self.difficulty.encode(buf);
        self.number.encode(buf);
        self.gas_limit.encode(buf);
        self.gas_used.encode(buf);
        self.timestamp.encode(buf);
        self.extra_data.encode(buf);
        self.prev_randao.encode(buf);
        self.nonce.encode(buf);
        self.base_fee_per_gas.encode(buf);
        self.withdrawals_root.encode(buf);
        self.blob_gas_used.encode(buf);
        self.excess_blob_gas.encode(buf);
        self.parent_beacon_block_root.encode(buf);
    }
}

// The body of a block on the chain
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockBody {
    transactions: Vec<Transaction>,
    // TODO: ommers list is always empty, so we can remove it
    ommers: Vec<BlockHeader>,
    withdrawals: Vec<Withdrawal>,
}

impl BlockBody {
    pub const fn empty() -> Self {
        Self {
            transactions: Vec::new(),
            ommers: Vec::new(),
            withdrawals: Vec::new(),
        }
    }

    pub fn compute_transactions_root(&self) -> H256 {
        let transactions_iter: Vec<_> = self
            .transactions
            .iter()
            .enumerate()
            .map(|(i, tx)| {
                // TODO: check if tree is RLP encoding the value

                // Key: RLP(tx_index)
                let mut k = Vec::new();
                i.encode(&mut k);

                // Value: tx_type || RLP(tx)  if tx_type != 0
                //                   RLP(tx)  else
                let mut v = Vec::new();
                match tx {
                    // Legacy transactions don't have a prefix
                    Transaction::LegacyTransaction(_) => {}
                    _ => v.push(tx.tx_type()),
                }

                tx.encode(&mut v);

                (k, v)
            })
            .collect();
        let root = PatriciaMerkleTree::<_, _, Keccak256>::compute_hash_from_sorted_iter(
            &transactions_iter,
        );
        H256(root.into())
    }
}

impl RLPEncode for BlockBody {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        self.transactions.encode(buf);
        self.ommers.encode(buf);
        self.withdrawals.encode(buf);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Withdrawal {
    index: u64,
    validator_index: u64,
    address: Address,
    amount: U256,
}

impl RLPEncode for Withdrawal {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        self.index.encode(buf);
        self.validator_index.encode(buf);
        self.address.encode(buf);
        self.amount.encode(buf);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transaction {
    LegacyTransaction(LegacyTransaction),
    EIP1559Transaction(EIP1559Transaction),
}

impl Transaction {
    pub fn tx_type(&self) -> u8 {
        match self {
            Transaction::LegacyTransaction(_) => 0,
            Transaction::EIP1559Transaction(_) => 2,
        }
    }
}

impl RLPEncode for Transaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        match self {
            Transaction::LegacyTransaction(t) => t.encode(buf),
            Transaction::EIP1559Transaction(t) => t.encode(buf),
        };
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyTransaction {
    nonce: u64,
    gas_price: U256,
    gas: u64,
    to: Option<Address>,
    value: U256,
    data: Bytes,
    v: U256,
    r: U256,
    s: U256,
}

impl RLPEncode for LegacyTransaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let encoder = structs::Encoder::new(buf)
            .encode_field(&self.nonce)
            .encode_field(&self.gas_price)
            .encode_field(&self.gas);

        // TODO: implement encode for Option?
        let encoder = match &self.to {
            Some(to) => encoder.encode_field(to),
            // TODO: move to a constant?
            None => encoder.encode_field(&0_u64),
        };
        encoder
            .encode_field(&self.value)
            .encode_field(&self.data)
            .encode_field(&self.v)
            .encode_field(&self.r)
            .encode_field(&self.s)
            .finish();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EIP1559Transaction {
    chain_id: u64,
    signer_nonce: U256,
    max_priority_fee_per_gas: u64,
    max_fee_per_gas: u64,
    gas_limit: u64,
    destination: Address,
    amount: u64,
    payload: Bytes,
    access_list: Vec<(Address, Vec<H256>)>,
    signature_y_parity: bool,
    signature_r: U256,
    signature_s: U256,
}

impl RLPEncode for EIP1559Transaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        // TODO: prepend size header
        self.chain_id.encode(buf);
        self.signer_nonce.encode(buf);
        self.max_priority_fee_per_gas.encode(buf);
        self.max_fee_per_gas.encode(buf);
        self.gas_limit.encode(buf);
        self.destination.encode(buf);
        self.amount.encode(buf);
        self.payload.encode(buf);
        self.access_list.encode(buf);
        self.signature_y_parity.encode(buf);
        self.signature_r.encode(buf);
        self.signature_s.encode(buf);
    }
}

#[cfg(test)]
mod tests {
    use hex_literal::hex;

    use super::{BlockBody, LegacyTransaction};
    use crate::{types::Transaction, U256};

    #[test]
    fn test_compute_transactions_root() {
        let mut body = BlockBody::empty();
        let tx = LegacyTransaction {
            nonce: 0,
            gas_price: 0x0a.into(),
            gas: 0x05f5e100,
            to: Some(hex!("1000000000000000000000000000000000000000").into()),
            value: 0.into(),
            data: Default::default(),
            v: U256::from(0x1b),
            r: U256::from(hex!(
                "7e09e26678ed4fac08a249ebe8ed680bf9051a5e14ad223e4b2b9d26e0208f37"
            )),
            s: U256::from(hex!(
                "5f6e3f188e3e6eab7d7d3b6568f5eac7d687b08d307d3154ccd8c87b4630509b"
            )),
        };
        body.transactions.push(Transaction::LegacyTransaction(tx));
        let expected_root =
            hex!("8151d548273f6683169524b66ca9fe338b9ce42bc3540046c828fd939ae23bcb");
        let result = body.compute_transactions_root();

        assert_eq!(result, expected_root.into());
    }
}
