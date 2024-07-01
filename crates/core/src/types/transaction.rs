use bytes::Bytes;
use ethereum_types::{Address, H256, U256};
use secp256k1::{ecdsa::RecoveryId, Message, SECP256K1};
use sha3::{Digest, Keccak256};

use crate::rlp::encode::RLPEncode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transaction {
    LegacyTransaction(LegacyTransaction),
    EIP1559Transaction(EIP1559Transaction),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyTransaction {
    pub nonce: u64,
    pub gas_price: u64,
    pub gas: u64,
    pub to: Address,
    pub value: U256,
    pub data: Bytes,
    pub v: U256,
    pub r: U256,
    pub s: U256,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EIP1559Transaction {
    pub chain_id: u64,
    pub signer_nonce: u64,
    pub max_priority_fee_per_gas: u64,
    pub max_fee_per_gas: u64,
    pub gas_limit: u64,
    pub destination: Address,
    pub amount: U256,
    pub payload: Bytes,
    pub access_list: Vec<(Address, Vec<H256>)>,
    pub signature_y_parity: bool,
    pub signature_r: U256,
    pub signature_s: U256,
}

impl RLPEncode for Transaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        match self {
            Transaction::LegacyTransaction(t) => t.encode(buf),
            Transaction::EIP1559Transaction(t) => t.encode(buf),
        };
    }
}

impl RLPEncode for LegacyTransaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        self.nonce.encode(buf);
        self.gas_price.encode(buf);
        self.gas.encode(buf);
        self.to.encode(buf);
        self.value.encode(buf);
        self.data.encode(buf);
        self.v.encode(buf);
        self.r.encode(buf);
        self.s.encode(buf);
    }
}

impl RLPEncode for EIP1559Transaction {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
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

impl Transaction {
    pub fn sender(&self) -> Address {
        match self {
            Transaction::LegacyTransaction(tx) => {
                let signature_y_parity = match self.chain_id() {
                    Some(chain_id) => tx.v.as_u64().saturating_sub(35 + chain_id * 2) != 0,
                    None => tx.v.as_u64().saturating_sub(27) != 0,
                };
                let data = (
                    tx.nonce,
                    tx.gas_price,
                    tx.gas,
                    tx.to,
                    tx.value,
                    tx.data.clone(),
                );
                let mut buf = vec![];
                data.encode(&mut buf);
                recover_address(&tx.r, &tx.s, signature_y_parity, &Bytes::from(buf))
            }
            Transaction::EIP1559Transaction(tx) => {
                let data = (
                    tx.signer_nonce,
                    // TODO: The following two fields are not part of EIP1559Transaction, other fields were used instead
                    // consider adding them
                    tx.max_fee_per_gas, // gas_price
                    tx.gas_limit,       // gas
                    tx.destination,
                    tx.amount,
                    tx.payload.clone(),
                    tx.chain_id,
                    0_u64,
                    0_u64,
                );
                let mut buf = vec![];
                data.encode(&mut buf);
                recover_address(
                    &tx.signature_r,
                    &tx.signature_s,
                    tx.signature_y_parity,
                    &Bytes::from(buf),
                )
            }
        }
    }

    pub fn gas_limit(&self) -> u64 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.gas,
            Transaction::EIP1559Transaction(tx) => tx.gas_limit,
        }
    }

    pub fn gas_price(&self) -> u64 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.gas_price,
            Transaction::EIP1559Transaction(tx) => tx.max_fee_per_gas,
        }
    }

    pub fn to(&self) -> Address {
        match self {
            Transaction::LegacyTransaction(tx) => tx.to,
            Transaction::EIP1559Transaction(tx) => tx.destination,
        }
    }

    pub fn value(&self) -> U256 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.value,
            Transaction::EIP1559Transaction(tx) => tx.amount,
        }
    }

    pub fn max_priority_fee(&self) -> Option<u64> {
        match self {
            Transaction::LegacyTransaction(_tx) => None,
            Transaction::EIP1559Transaction(tx) => Some(tx.max_priority_fee_per_gas),
        }
    }

    pub fn chain_id(&self) -> Option<u64> {
        match self {
            Transaction::LegacyTransaction(_tx) => None,
            Transaction::EIP1559Transaction(tx) => Some(tx.chain_id),
        }
    }

    pub fn access_list(&self) -> Vec<(Address, Vec<H256>)> {
        match self {
            Transaction::LegacyTransaction(_tx) => Vec::new(),
            Transaction::EIP1559Transaction(tx) => tx.access_list.clone(),
        }
    }

    pub fn nonce(&self) -> u64 {
        match self {
            Transaction::LegacyTransaction(tx) => tx.nonce,
            Transaction::EIP1559Transaction(tx) => tx.signer_nonce,
        }
    }

    pub fn data(&self) -> &Bytes {
        match self {
            Transaction::LegacyTransaction(tx) => &tx.data,
            Transaction::EIP1559Transaction(tx) => &tx.payload,
        }
    }
}

fn recover_address(
    signature_r: &U256,
    signature_s: &U256,
    signature_y_parity: bool,
    message: &Bytes,
) -> Address {
    // Create signature
    let mut signature_bytes = [0; 64];
    signature_r.to_big_endian(&mut signature_bytes[0..32]);
    signature_s.to_big_endian(&mut signature_bytes[32..]);
    let signature = secp256k1::ecdsa::RecoverableSignature::from_compact(
        &signature_bytes,
        RecoveryId::from_i32(signature_y_parity as i32).unwrap(), // cannot fail
    )
    .unwrap();
    // Hash message
    let msg_digest: [u8; 32] = Keccak256::new_with_prefix(message.as_ref())
        .finalize()
        .into();
    // Recover public key
    let public = SECP256K1
        .recover_ecdsa(&Message::from_digest(msg_digest), &signature)
        .unwrap();
    // Hash public key to obtain address
    let hash = Keccak256::new_with_prefix(&public.serialize_uncompressed()[1..]).finalize();
    Address::from_slice(&hash[12..])
}
