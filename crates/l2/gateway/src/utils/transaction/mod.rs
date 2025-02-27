use crate::utils::communication::messages::BlockSyncMessage;
use bytes::Bytes;
use ethrex_common::types::{EIP1559Transaction, Signable, Transaction as TxEnvelope, TxKind};
use ethrex_common::{Address, U256};
use ethrex_rlp::error::RLPDecodeError;
use secp256k1::rand::random;
use std::{ops::Deref, sync::Arc};

pub use simulated::{SimulatedTx, SimulatedTxList};
pub use tx_list::TxList;

pub mod simulated;
pub mod tx_list;

#[derive(Clone, Debug)]
pub struct Transaction {
    pub tx: TxEnvelope,
    /// The sender of the transaction.
    /// Recovered from the tx on initialisation.
    sender: Address,
    pub envelope: Bytes,
}

impl Transaction {
    pub fn new(tx: TxEnvelope, sender: Address, envelope: Bytes) -> Self {
        Self {
            tx,
            sender,
            envelope,
        }
    }

    #[inline]
    pub fn sender(&self) -> Address {
        self.sender
    }

    #[inline]
    pub fn sender_ref(&self) -> &Address {
        &self.sender
    }

    #[inline]
    pub fn nonce_ref(&self) -> &u64 {
        &self.tx.nonce()
    }

    /// Returns the gas price for type 0 and 1 transactions.
    /// Returns the max fee for EIP-1559 transactions.
    /// Returns `None` for deposit transactions.
    #[inline]
    pub fn gas_price_or_max_fee(&self) -> Option<u128> {
        self.tx.gas_price()
    }

    /// Returns true if the transaction is valid for a block with the given base fee.
    #[inline]
    pub fn valid_for_block(&self, base_fee: u64) -> bool {
        self.gas_price_or_max_fee()
            .map_or(true, |price| price > base_fee as u128)
    }

    // #[inline]
    // pub fn fill_tx_env(&self, tx_env: &mut TxEnv) {
    //     let envelope = self.encode();

    //     tx_env.caller = self.sender;
    //     match &self.tx {
    //         TxEnvelope::Legacy(tx) => {
    //             tx_env.gas_limit = tx.tx().gas_limit;
    //             tx_env.gas_price = alloy_primitives::U256::from(tx.tx().gas_price);
    //             tx_env.gas_priority_fee = None;
    //             tx_env.transact_to = tx.tx().to;
    //             tx_env.value = tx.tx().value;
    //             tx_env.data = tx.tx().input.clone();
    //             tx_env.chain_id = tx.tx().chain_id;
    //             tx_env.nonce = Some(tx.tx().nonce);
    //             tx_env.access_list.clear();
    //             tx_env.blob_hashes.clear();
    //             tx_env.max_fee_per_blob_gas.take();
    //             tx_env.authorization_list = None;
    //         }
    //         TxEnvelope::Eip2930(tx) => {
    //             tx_env.gas_limit = tx.tx().gas_limit;
    //             tx_env.gas_price = alloy_primitives::U256::from(tx.tx().gas_price);
    //             tx_env.gas_priority_fee = None;
    //             tx_env.transact_to = tx.tx().to;
    //             tx_env.value = tx.tx().value;
    //             tx_env.data = tx.tx().input.clone();
    //             tx_env.chain_id = Some(tx.tx().chain_id);
    //             tx_env.nonce = Some(tx.tx().nonce);
    //             tx_env.access_list.clone_from(&tx.tx().access_list.0);
    //             tx_env.blob_hashes.clear();
    //             tx_env.max_fee_per_blob_gas.take();
    //             tx_env.authorization_list = None;
    //         }
    //         TxEnvelope::Eip1559(tx) => {
    //             tx_env.gas_limit = tx.tx().gas_limit;
    //             tx_env.gas_price = alloy_primitives::U256::from(tx.tx().max_fee_per_gas);
    //             tx_env.gas_priority_fee = Some(alloy_primitives::U256::from(
    //                 tx.tx().max_priority_fee_per_gas,
    //             ));
    //             tx_env.transact_to = tx.tx().to;
    //             tx_env.value = tx.tx().value;
    //             tx_env.data = tx.tx().input.clone();
    //             tx_env.chain_id = Some(tx.tx().chain_id);
    //             tx_env.nonce = Some(tx.tx().nonce);
    //             tx_env.access_list.clone_from(&tx.tx().access_list.0);
    //             tx_env.blob_hashes.clear();
    //             tx_env.max_fee_per_blob_gas.take();
    //             tx_env.authorization_list = None;
    //         }
    //         TxEnvelope::Eip7702(tx) => {
    //             tx_env.gas_limit = tx.tx().gas_limit;
    //             tx_env.gas_price = alloy_primitives::U256::from(tx.tx().max_fee_per_gas);
    //             tx_env.gas_priority_fee = Some(alloy_primitives::U256::from(
    //                 tx.tx().max_priority_fee_per_gas,
    //             ));
    //             tx_env.transact_to = tx.tx().to.into();
    //             tx_env.value = tx.tx().value;
    //             tx_env.data = tx.tx().input.clone();
    //             tx_env.chain_id = Some(tx.tx().chain_id);
    //             tx_env.nonce = Some(tx.tx().nonce);
    //             tx_env.access_list.clone_from(&tx.tx().access_list.0);
    //             tx_env.blob_hashes.clear();
    //             tx_env.max_fee_per_blob_gas.take();
    //             tx_env.authorization_list = Some(revm_primitives::AuthorizationList::Signed(
    //                 tx.tx().authorization_list.clone(),
    //             ));
    //         }
    //         TxEnvelope::Deposit(tx) => {
    //             tx_env.access_list.clear();
    //             tx_env.gas_limit = tx.gas_limit;
    //             tx_env.gas_price = alloy_primitives::U256::ZERO;
    //             tx_env.gas_priority_fee = None;
    //             tx_env.transact_to = tx.to;
    //             tx_env.value = tx.value;
    //             tx_env.data = tx.input.clone();
    //             tx_env.chain_id = None;
    //             tx_env.nonce = None;
    //             tx_env.authorization_list = None;

    //             tx_env.optimism = revm_primitives::OptimismFields {
    //                 source_hash: Some(tx.source_hash),
    //                 mint: tx.mint,
    //                 is_system_transaction: Some(tx.is_system_transaction),
    //                 enveloped_tx: Some(envelope),
    //             };
    //             return;
    //         }
    //         _ => unreachable!(),
    //     }

    //     tx_env.optimism = revm_primitives::OptimismFields {
    //         source_hash: None,
    //         mint: None,
    //         is_system_transaction: Some(false),
    //         enveloped_tx: Some(envelope),
    //     }
    // }

    #[inline]
    pub fn random() -> Self {
        let value = 50;
        let max_gas_units = 50;
        let max_fee_per_gas = 50;
        let nonce = 1;
        let chain_id = 1000;
        let max_priority_fee_per_gas = 1000;

        let tx = TxEnvelope::EIP1559Transaction(EIP1559Transaction {
            chain_id,
            nonce,
            gas_limit: max_gas_units,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to: TxKind::Call(Address::random()),
            value: U256::from(value),
            ..Default::default()
        });
        let signed_tx = tx.sign(random());
        let tx = TxEnvelope::Eip1559(signed_tx);
        let envelope = tx.encoded_2718().into();
        Self {
            sender: Address::random(),
            tx,
            envelope,
        }
    }

    pub fn decode(bytes: Bytes) -> Result<Self, RLPDecodeError> {
        let tx = TxEnvelope::decode_canonical(&bytes)?;
        Ok(Self {
            sender: tx.sender(),
            tx,
            envelope: bytes,
        })
    }

    pub fn encode(&self) -> Bytes {
        debug_assert_eq!(self.envelope, self.tx.encoded_2718());
        self.envelope.clone()
    }

    pub fn from_block(block: &BlockSyncMessage) -> Vec<Arc<Transaction>> {
        block
            .body
            .transactions
            .iter()
            .map(|t| Arc::new(t.clone().into()))
            .collect()
    }
}

impl Deref for Transaction {
    type Target = TxEnvelope;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}
