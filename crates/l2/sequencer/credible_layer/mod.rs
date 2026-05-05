/// This module implements a gRPC client actor that communicates with the sidecar
/// during block building. Transactions that fail assertion validation are dropped
/// before block inclusion.
///
/// The integration is opt-in via the `--credible-layer-url` CLI flag.
/// When disabled, there is zero overhead.
pub mod client;
pub mod errors;

pub use client::CredibleLayerClient;
pub use errors::CredibleLayerError;

/// Generated protobuf/gRPC types for sidecar.proto
pub mod sidecar_proto {
    tonic::include_proto!("sidecar.transport.v1");
}

// Conversions from ethrex types to sidecar protobuf types.

use ethrex_common::Address;
use ethrex_common::types::{Transaction, TxKind};

impl From<(Transaction, Address)> for sidecar_proto::TransactionEnv {
    fn from((tx, sender): (Transaction, Address)) -> Self {
        let transact_to = match tx.to() {
            TxKind::Call(addr) => addr.as_bytes().to_vec(),
            TxKind::Create => vec![],
        };

        let value_bytes = tx.value().to_big_endian();

        let mut gas_price_bytes = [0u8; 16];
        let gas_price_u128 = tx.gas_price().as_u128();
        gas_price_bytes.copy_from_slice(&gas_price_u128.to_be_bytes());

        let gas_priority_fee = tx.max_priority_fee().map(|fee| {
            let mut buf = [0u8; 16];
            buf[8..].copy_from_slice(&fee.to_be_bytes());
            buf.to_vec()
        });

        let access_list = tx
            .access_list()
            .iter()
            .map(|(addr, keys)| sidecar_proto::AccessListItem {
                address: addr.as_bytes().to_vec(),
                storage_keys: keys.iter().map(|k| k.as_bytes().to_vec()).collect(),
            })
            .collect();

        let authorization_list = tx
            .authorization_list()
            .map(|list| {
                list.iter()
                    .map(|auth| {
                        let chain_id_bytes = auth.chain_id.to_big_endian();
                        let r_bytes = auth.r_signature.to_big_endian();
                        let s_bytes = auth.s_signature.to_big_endian();
                        sidecar_proto::Authorization {
                            chain_id: chain_id_bytes.to_vec(),
                            address: auth.address.as_bytes().to_vec(),
                            nonce: auth.nonce,
                            y_parity: u32::try_from(auth.y_parity).unwrap_or(0),
                            r: r_bytes.to_vec(),
                            s: s_bytes.to_vec(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self {
            tx_type: u32::from(u8::from(tx.tx_type())),
            caller: sender.as_bytes().to_vec(),
            gas_limit: tx.gas_limit(),
            gas_price: gas_price_bytes.to_vec(),
            transact_to,
            value: value_bytes.to_vec(),
            data: tx.data().to_vec(),
            nonce: tx.nonce(),
            chain_id: tx.chain_id(),
            access_list,
            gas_priority_fee,
            blob_hashes: tx
                .blob_versioned_hashes()
                .iter()
                .map(|h| h.as_bytes().to_vec())
                .collect(),
            max_fee_per_blob_gas: vec![0u8; 16],
            authorization_list,
        }
    }
}
