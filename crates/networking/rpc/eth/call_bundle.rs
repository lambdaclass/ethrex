//! `eth_callBundle` (Flashbots), compatible with reth's implementation and alloy's types.

use bytes::Bytes;
use ethrex_blockchain::call_bundle::{
    CallBundleBlockEnv, CallBundleError, CallBundleResult, CallBundleTxResult, call_bundle_header,
};
use ethrex_common::{
    Address, H256, U256,
    constants::GAS_PER_BLOB,
    serde_utils,
    types::{BlobsBundle, BlockHeader, ChainConfig, Transaction},
};
use ethrex_crypto::NativeCrypto;
use ethrex_vm::TxValidationError;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::{Value, value::RawValue};

use crate::{
    rpc::{RpcApiContext, RpcHandler},
    types::{
        block_identifier::{BlockIdentifier, BlockTag},
        transaction::SendRawTransactionRequest,
    },
    utils::RpcErr,
};

pub const MAX_CALL_BUNDLE_TXS: usize = 256;

/// reth's fallback before Cancun.
const PRE_CANCUN_MAX_BLOBS: u64 = 6;

const TRANSACTION_REJECTED: i32 = -32003;

/// alloy's `EthCallBundle`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallBundleRequest {
    #[serde(deserialize_with = "serde_utils::bytes::vec::deserialize")]
    pub txs: Vec<Bytes>,
    #[serde(deserialize_with = "serde_utils::u64::hex_or_dec::deserialize")]
    pub block_number: u64,
    #[serde(deserialize_with = "deserialize_state_block")]
    pub state_block_number: BlockIdentifier,
    #[serde(default)]
    pub coinbase: Option<Address>,
    #[serde(
        default,
        deserialize_with = "serde_utils::u64::hex_or_dec_opt::deserialize"
    )]
    pub timestamp: Option<u64>,
    /// Ignored, as in reth.
    #[allow(dead_code)]
    #[serde(
        default,
        deserialize_with = "serde_utils::u64::hex_or_dec_opt::deserialize"
    )]
    pub timeout: Option<u64>,
    #[serde(
        default,
        deserialize_with = "serde_utils::u64::hex_or_dec_opt::deserialize"
    )]
    pub gas_limit: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_u256")]
    pub difficulty: Option<U256>,
    #[serde(default, deserialize_with = "deserialize_opt_u128")]
    pub base_fee: Option<u128>,
}

fn deserialize_state_block<'de, D: Deserializer<'de>>(d: D) -> Result<BlockIdentifier, D::Error> {
    let value = String::deserialize(d)?;
    Ok(match value.to_lowercase().as_str() {
        "latest" => BlockIdentifier::Tag(BlockTag::Latest),
        "finalized" => BlockIdentifier::Tag(BlockTag::Finalized),
        "safe" => BlockIdentifier::Tag(BlockTag::Safe),
        "earliest" => BlockIdentifier::Tag(BlockTag::Earliest),
        "pending" => BlockIdentifier::Tag(BlockTag::Pending),
        other => {
            let digits = other
                .strip_prefix("0x")
                .ok_or_else(|| D::Error::custom("hex string without 0x prefix"))?;
            BlockIdentifier::Number(u64::from_str_radix(digits, 16).map_err(D::Error::custom)?)
        }
    })
}

fn deserialize_opt_u256<'de, D: Deserializer<'de>>(d: D) -> Result<Option<U256>, D::Error> {
    Option::<Value>::deserialize(d)?
        .map(|value| serde_utils::u256::hex_or_dec::deserialize(value).map_err(D::Error::custom))
        .transpose()
}

fn deserialize_opt_u128<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u128>, D::Error> {
    deserialize_opt_u256(d)?
        .map(|value| u128::try_from(value).map_err(|_| D::Error::custom("number too large")))
        .transpose()
}

/// alloy's `EthCallBundleResponse`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallBundleResponse {
    pub bundle_hash: H256,
    #[serde(with = "serde_utils::u256::dec_str")]
    pub bundle_gas_price: U256,
    #[serde(with = "serde_utils::u256::dec_str")]
    pub coinbase_diff: U256,
    #[serde(with = "serde_utils::u256::dec_str")]
    pub eth_sent_to_coinbase: U256,
    #[serde(with = "serde_utils::u256::dec_str")]
    pub gas_fees: U256,
    pub results: Vec<CallBundleTxResponse>,
    pub state_block_number: u64,
    pub total_gas_used: u64,
}

/// alloy's `EthCallBundleTransactionResult`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallBundleTxResponse {
    #[serde(with = "serde_utils::u256::dec_str")]
    pub coinbase_diff: U256,
    #[serde(with = "serde_utils::u256::dec_str")]
    pub eth_sent_to_coinbase: U256,
    pub from_address: Address,
    #[serde(with = "serde_utils::u256::dec_str")]
    pub gas_fees: U256,
    #[serde(with = "serde_utils::u256::dec_str")]
    pub gas_price: U256,
    pub gas_used: u64,
    pub to_address: Option<Address>,
    pub tx_hash: H256,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_bytes"
    )]
    pub value: Option<Bytes>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_bytes"
    )]
    pub revert: Option<Bytes>,
}

fn serialize_opt_bytes<S: Serializer>(value: &Option<Bytes>, s: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(bytes) => serde_utils::bytes::serialize(bytes, s),
        None => s.serialize_none(),
    }
}

impl From<CallBundleTxResult> for CallBundleTxResponse {
    fn from(result: CallBundleTxResult) -> Self {
        let (value, revert) = if result.success {
            (Some(result.output), None)
        } else {
            (None, Some(result.output))
        };
        CallBundleTxResponse {
            coinbase_diff: result.coinbase_diff,
            eth_sent_to_coinbase: result.eth_sent_to_coinbase,
            from_address: result.from,
            gas_fees: result.gas_fees,
            gas_price: result.gas_price,
            gas_used: result.gas_used,
            to_address: result.to,
            tx_hash: result.tx_hash,
            value,
            revert,
        }
    }
}

impl From<CallBundleResult> for CallBundleResponse {
    fn from(result: CallBundleResult) -> Self {
        CallBundleResponse {
            bundle_hash: result.bundle_hash,
            bundle_gas_price: result.bundle_gas_price,
            coinbase_diff: result.coinbase_diff,
            eth_sent_to_coinbase: result.eth_sent_to_coinbase,
            gas_fees: result.gas_fees,
            results: result.results.into_iter().map(Into::into).collect(),
            state_block_number: result.state_block_number,
            total_gas_used: result.total_gas_used,
        }
    }
}

struct BundleTx {
    tx: Transaction,
    sender: Address,
    blobs_bundle: Option<BlobsBundle>,
}

impl RpcHandler for CallBundleRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<CallBundleRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 1 {
            return Err(RpcErr::BadParams(format!(
                "Expected one param, got {}",
                params.len()
            )));
        }
        let request: CallBundleRequest = serde_json::from_str(params[0].get())
            .map_err(|error| RpcErr::BadParams(error.to_string()))?;
        if request.txs.is_empty() {
            return Err(RpcErr::InvalidParams("bundle missing txs".to_string()));
        }
        if request.block_number == 0 {
            return Err(RpcErr::InvalidParams(
                "bundle missing blockNumber".to_string(),
            ));
        }
        if request.txs.len() > MAX_CALL_BUNDLE_TXS {
            return Err(RpcErr::LimitExceeded(format!(
                "too many transactions in bundle: {} > {MAX_CALL_BUNDLE_TXS}",
                request.txs.len()
            )));
        }
        Ok(request)
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let gas_cap = context.rpc_gas_cap;
        if let Some(gas_limit) = self.gas_limit
            && gas_cap != 0
            && gas_limit > gas_cap
        {
            return Err(RpcErr::ServerError("intrinsic gas too high".to_string()));
        }

        let txs = self
            .txs
            .iter()
            .map(|raw| decode_bundle_tx(raw))
            .collect::<Result<Vec<_>, _>>()?;

        let Some(parent) = self
            .state_block_number
            .resolve_block_header(&context.storage)
            .await?
        else {
            return Err(block_not_found(&self.state_block_number));
        };

        let env = CallBundleBlockEnv {
            number: self.block_number,
            coinbase: self.coinbase,
            timestamp: self.timestamp,
            gas_limit: self.gas_limit.unwrap_or(if gas_cap == 0 {
                parent.gas_limit
            } else {
                gas_cap
            }),
            difficulty: self.difficulty,
            base_fee: self
                .base_fee
                .map(|base_fee| u64::try_from(base_fee).unwrap_or(u64::MAX)),
        };
        let header = call_bundle_header(&parent, &env);
        let chain_config = context.storage.get_chain_config();
        validate_blobs(&txs, &header, &chain_config)?;

        let txs: Vec<(Transaction, Address)> = txs
            .into_iter()
            .map(|bundle_tx| (bundle_tx.tx, bundle_tx.sender))
            .collect();
        let blockchain = context.blockchain.clone();
        let result =
            tokio::task::spawn_blocking(move || blockchain.call_bundle(&parent, &header, &txs))
                .await
                .map_err(|error| RpcErr::Internal(error.to_string()))?
                .map_err(call_bundle_error_to_rpc)?;
        Ok(serde_json::to_value(CallBundleResponse::from(result))?)
    }
}

/// Blob transactions must carry their sidecar, as reth decodes pooled transactions.
fn decode_bundle_tx(raw: &[u8]) -> Result<BundleTx, RpcErr> {
    if raw.is_empty() {
        return Err(RpcErr::InvalidParams("empty transaction data".to_string()));
    }
    let undecodable = || RpcErr::InvalidParams("failed to decode signed transaction".to_string());
    let decoded = SendRawTransactionRequest::decode_canonical(raw).map_err(|_| undecodable())?;
    let (tx, blobs_bundle) = match decoded {
        SendRawTransactionRequest::PrivilegedL2(_) => return Err(undecodable()),
        SendRawTransactionRequest::EIP4844(wrapped) => {
            if wrapped.blobs_bundle.blobs.is_empty() {
                return Err(undecodable());
            }
            (
                Transaction::EIP4844Transaction(wrapped.tx),
                Some(wrapped.blobs_bundle),
            )
        }
        other => (other.to_transaction(), None),
    };
    let sender = tx
        .sender(&NativeCrypto)
        .map_err(|_| RpcErr::InvalidParams("invalid transaction signature".to_string()))?;
    Ok(BundleTx {
        tx,
        sender,
        blobs_bundle,
    })
}

fn validate_blobs(
    txs: &[BundleTx],
    header: &BlockHeader,
    chain_config: &ChainConfig,
) -> Result<(), RpcErr> {
    let blob_gas_used: u64 = txs
        .iter()
        .map(|bundle_tx| bundle_tx.tx.blob_versioned_hashes().len() as u64)
        .sum::<u64>()
        .saturating_mul(u64::from(GAS_PER_BLOB));
    if blob_gas_used == 0 {
        return Ok(());
    }
    let max_blob_gas = chain_config
        .get_fork_blob_schedule(header.timestamp)
        .map_or(PRE_CANCUN_MAX_BLOBS, |schedule| u64::from(schedule.max))
        * u64::from(GAS_PER_BLOB);
    if blob_gas_used > max_blob_gas {
        return Err(RpcErr::InvalidParams(format!(
            "blob gas usage exceeds the limit of {max_blob_gas} gas per block."
        )));
    }
    let fork = chain_config.fork(header.timestamp);
    for bundle_tx in txs {
        if let (Transaction::EIP4844Transaction(blob_tx), Some(blobs_bundle)) =
            (&bundle_tx.tx, &bundle_tx.blobs_bundle)
        {
            validate_sidecar(blob_tx, blobs_bundle, fork)?;
        }
    }
    Ok(())
}

#[cfg(feature = "c-kzg")]
fn validate_sidecar(
    tx: &ethrex_common::types::EIP4844Transaction,
    blobs_bundle: &BlobsBundle,
    fork: ethrex_common::types::Fork,
) -> Result<(), RpcErr> {
    blobs_bundle
        .validate(tx, fork)
        .map_err(|error| RpcErr::InvalidParams(error.to_string()))
}

#[cfg(not(feature = "c-kzg"))]
fn validate_sidecar(
    _tx: &ethrex_common::types::EIP4844Transaction,
    _blobs_bundle: &BlobsBundle,
    _fork: ethrex_common::types::Fork,
) -> Result<(), RpcErr> {
    Err(RpcErr::InvalidParams(
        "blob transactions need KZG support, which this build lacks".to_string(),
    ))
}

fn block_not_found(block: &BlockIdentifier) -> RpcErr {
    let block = match block {
        BlockIdentifier::Number(number) => format!("{number:#x}"),
        tag => tag.to_string(),
    };
    RpcErr::MethodError {
        code: -32001,
        message: format!("block not found: {block}"),
        data: None,
    }
}

fn call_bundle_error_to_rpc(error: CallBundleError) -> RpcErr {
    match error {
        CallBundleError::FeeCapTooLow | CallBundleError::InvalidChainId => {
            RpcErr::ServerError(error.to_string())
        }
        CallBundleError::InvalidTx(error) => invalid_tx_error(&error),
        CallBundleError::BlockGasLimitReached { .. } => {
            RpcErr::ServerError("intrinsic gas too high".to_string())
        }
        CallBundleError::Internal(message) => RpcErr::Internal(message),
    }
}

/// reth's codes: `-32000` for nonce, intrinsic gas and fee-cap failures, `-32003` otherwise.
fn invalid_tx_error(error: &TxValidationError) -> RpcErr {
    let rejected = |message: String| RpcErr::MethodError {
        code: TRANSACTION_REJECTED,
        message,
        data: None,
    };
    match error {
        TxValidationError::NonceMismatch { expected, actual } if actual < expected => {
            RpcErr::ServerError(format!(
                "nonce too low: next nonce {expected}, tx nonce {actual}"
            ))
        }
        TxValidationError::NonceMismatch { .. } => {
            RpcErr::ServerError("nonce too high".to_string())
        }
        TxValidationError::IntrinsicGasTooLow => {
            RpcErr::ServerError("intrinsic gas too low".to_string())
        }
        TxValidationError::GasAllowanceExceeded { .. }
        | TxValidationError::TxMaxGasLimitExceeded { .. } => {
            RpcErr::ServerError("intrinsic gas too high".to_string())
        }
        TxValidationError::InsufficientMaxFeePerGas => {
            RpcErr::ServerError("max fee per gas less than block base fee".to_string())
        }
        TxValidationError::NonceIsMax => rejected("nonce has max value".to_string()),
        TxValidationError::InsufficientAccountFunds
        | TxValidationError::GasLimitPriceProductOverflow => {
            rejected("insufficient funds for gas * price + value".to_string())
        }
        TxValidationError::IntrinsicGasBelowFloorGasCost => {
            rejected("gas floor exceeds the gas limit".to_string())
        }
        TxValidationError::InitcodeSizeExceeded { .. } => {
            rejected("max initcode size exceeded".to_string())
        }
        TxValidationError::PriorityGreaterThanMaxFeePerGas { .. } => {
            rejected("max priority fee per gas higher than max fee per gas".to_string())
        }
        TxValidationError::InsufficientMaxFeePerBlobGas { .. } => {
            rejected("max fee per blob gas less than block blob gas fee".to_string())
        }
        TxValidationError::SenderNotEOA(_) => rejected("sender is not an EOA".to_string()),
        TxValidationError::Type1TxPreFork
        | TxValidationError::Type2TxPreFork
        | TxValidationError::Type3TxPreFork
        | TxValidationError::Type4TxPreFork
        | TxValidationError::FrameTxPreFork => {
            rejected("transaction type not supported".to_string())
        }
        TxValidationError::Type3TxZeroBlobs => {
            rejected("blob transaction missing blob hashes".to_string())
        }
        TxValidationError::Type3TxInvalidBlobVersionedHash => {
            rejected("blob hash version mismatch".to_string())
        }
        TxValidationError::Type3TxBlobCountExceeded {
            actual_blob_count, ..
        } => rejected(format!(
            "blob transaction exceeds max blobs per block; got {actual_blob_count}"
        )),
        TxValidationError::Type3TxContractCreation => {
            rejected("blob transaction is a create transaction".to_string())
        }
        TxValidationError::Type4TxAuthorizationListIsEmpty => {
            rejected("EIP-7702 authorization list has invalid fields".to_string())
        }
        other => rejected(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::rpc::{DEFAULT_RPC_GAS_CAP, map_http_requests};
    use crate::test_utils::{TEST_GENESIS, context_without_p2p};
    use crate::utils::{RpcErrorMetadata, RpcRequest};
    use ethrex_common::types::{
        Block, BlockBody, EIP1559Transaction, EIP4844Transaction, Genesis, GenesisAccount,
        LegacyTransaction, TxKind,
    };
    use ethrex_crypto::Crypto;
    use ethrex_storage::{EngineType, Store};
    use secp256k1::SecretKey;
    use serde_json::json;
    use std::str::FromStr;

    const CHAIN_ID: u64 = 9;
    const GWEI: u64 = 1_000_000_000;
    const NEXT_BASE_FEE: u64 = GWEI - GWEI / 8;
    const ETHER: u128 = 1_000_000_000_000_000_000;

    /// `CALL(gas, COINBASE, CALLVALUE, 0, 0, 0, 0)`: forwards the call's value to the
    /// block's coinbase.
    const PAYER: Address = Address::repeat_byte(0xa1);
    const PAYER_CODE: &str = "600060006000600034415af100";
    /// `REVERT` with the 4 bytes `0xdeadbeef`.
    const REVERTER: Address = Address::repeat_byte(0xa2);
    const REVERTER_CODE: &str = "63deadbeef6000526004601cfd";
    /// Returns NUMBER, TIMESTAMP, BASEFEE, COINBASE and GASLIMIT as five words.
    const ENV: Address = Address::repeat_byte(0xa3);
    const ENV_CODE: &str = "436000524260205248604052416060524560805260a06000f3";
    /// Returns `BLOCKHASH(NUMBER - 1)`.
    const HASHER: Address = Address::repeat_byte(0xa4);
    const HASHER_CODE: &str = "600143034060005260206000f3";
    const FRESH: Address = Address::repeat_byte(0xb1);

    fn key(byte: u8) -> SecretKey {
        SecretKey::from_byte_array(&[byte; 32]).unwrap()
    }

    fn rich() -> SecretKey {
        key(0x11)
    }

    fn poor() -> SecretKey {
        key(0x22)
    }

    fn address_of(secret: &SecretKey) -> Address {
        let public = secp256k1::PublicKey::from_secret_key(secp256k1::SECP256K1, secret)
            .serialize_uncompressed();
        Address::from_slice(&NativeCrypto.keccak256(&public[1..])[12..])
    }

    async fn store() -> Store {
        let mut genesis: Genesis = serde_json::from_str(TEST_GENESIS).unwrap();
        genesis.alloc.insert(
            address_of(&rich()),
            GenesisAccount {
                code: Bytes::new(),
                storage: Default::default(),
                balance: U256::from(1_000u128 * ETHER),
                nonce: 0,
            },
        );
        for (address, code) in [
            (PAYER, PAYER_CODE),
            (REVERTER, REVERTER_CODE),
            (ENV, ENV_CODE),
            (HASHER, HASHER_CODE),
        ] {
            genesis.alloc.insert(
                address,
                GenesisAccount {
                    code: Bytes::from(hex::decode(code).unwrap()),
                    storage: Default::default(),
                    balance: U256::zero(),
                    nonce: 1,
                },
            );
        }
        let mut store = Store::new("call-bundle-test", EngineType::InMemory).unwrap();
        store.add_initial_state(genesis).await.unwrap();
        store
    }

    async fn add_block_1(store: &Store) -> BlockHeader {
        let genesis = store.get_block_header(0).unwrap().unwrap();
        let header = BlockHeader {
            parent_hash: genesis.hash(),
            number: 1,
            state_root: genesis.state_root,
            gas_limit: genesis.gas_limit,
            gas_used: 20_000_000,
            timestamp: genesis.timestamp + 12,
            base_fee_per_gas: Some(NEXT_BASE_FEE),
            coinbase: Address::repeat_byte(0xbe),
            hash: Default::default(),
            ..genesis
        };
        let block = Block::new(header.clone(), BlockBody::default());
        let hash = block.hash();
        store.add_block(block).await.unwrap();
        store
            .forkchoice_update(vec![], 1, hash, None, None)
            .await
            .unwrap();
        header
    }

    fn sign(mut tx: Transaction, secret: &SecretKey) -> Transaction {
        if let Transaction::LegacyTransaction(legacy) = &mut tx {
            legacy.v = U256::from(35 + 2 * CHAIN_ID);
        }
        let (payload, _) = tx.signing_payload().unwrap().unwrap();
        let message = secp256k1::Message::from_digest(NativeCrypto.keccak256(&payload));
        let (recovery_id, signature) = secp256k1::SECP256K1
            .sign_ecdsa_recoverable(&message, secret)
            .serialize_compact();
        let r = U256::from_big_endian(&signature[..32]);
        let s = U256::from_big_endian(&signature[32..]);
        let parity = i32::from(recovery_id) != 0;
        match &mut tx {
            Transaction::LegacyTransaction(legacy) => {
                legacy.r = r;
                legacy.s = s;
                legacy.v = U256::from(35 + 2 * CHAIN_ID + u64::from(parity));
            }
            Transaction::EIP1559Transaction(eip1559) => {
                eip1559.signature_r = r;
                eip1559.signature_s = s;
                eip1559.signature_y_parity = parity;
            }
            _ => unreachable!("only legacy and EIP-1559 test transactions"),
        }
        tx
    }

    struct Tx {
        nonce: u64,
        to: TxKind,
        value: u128,
        gas: u64,
        max_fee: u64,
        priority_fee: u64,
        data: Bytes,
        chain_id: u64,
    }

    impl Default for Tx {
        fn default() -> Self {
            Tx {
                nonce: 0,
                to: TxKind::Call(FRESH),
                value: 0,
                gas: 21_000,
                max_fee: 2 * GWEI,
                priority_fee: GWEI,
                data: Bytes::new(),
                chain_id: CHAIN_ID,
            }
        }
    }

    impl Tx {
        fn signed(self, secret: &SecretKey) -> Transaction {
            sign(
                Transaction::EIP1559Transaction(EIP1559Transaction {
                    chain_id: self.chain_id,
                    nonce: self.nonce,
                    max_priority_fee_per_gas: self.priority_fee,
                    max_fee_per_gas: self.max_fee,
                    gas_limit: self.gas,
                    to: self.to,
                    value: U256::from(self.value),
                    data: self.data,
                    ..Default::default()
                }),
                secret,
            )
        }
    }

    fn raw(tx: &Transaction) -> String {
        format!("0x{}", hex::encode(tx.encode_canonical_to_vec()))
    }

    fn hash(tx: &Transaction) -> String {
        format!("{:#x}", tx.hash(&NativeCrypto))
    }

    async fn call_bundle_on(store: Store, bundle: Value) -> Result<Value, RpcErr> {
        let context = context_without_p2p(store);
        let request: RpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0", "id": 1, "method": "eth_callBundle", "params": [bundle],
        }))
        .unwrap();
        map_http_requests(&request, context.clone()).await
    }

    async fn call_bundle(bundle: Value) -> Result<Value, RpcErr> {
        call_bundle_on(store().await, bundle).await
    }

    fn error(err: RpcErr) -> (i32, String) {
        let metadata = RpcErrorMetadata::from(err);
        (metadata.code, metadata.message)
    }

    async fn call_bundle_err(bundle: Value) -> (i32, String) {
        error(call_bundle(bundle).await.unwrap_err())
    }

    fn wei(value: &Value) -> u128 {
        value.as_str().unwrap().parse().unwrap()
    }

    fn word(output: &str, index: usize) -> U256 {
        let bytes = hex::decode(output.trim_start_matches("0x")).unwrap();
        U256::from_big_endian(&bytes[index * 32..(index + 1) * 32])
    }

    #[tokio::test]
    async fn plain_transfer() {
        let tx = Tx {
            value: ETHER,
            ..Default::default()
        }
        .signed(&rich());
        let response = call_bundle(json!({
            "txs": [raw(&tx)], "blockNumber": "0x1", "stateBlockNumber": "latest",
        }))
        .await
        .unwrap();
        let fees = 21_000 * u128::from(GWEI);
        let result = &response["results"][0];
        assert_eq!(result["txHash"], hash(&tx));
        assert_eq!(result["fromAddress"], format!("{:#x}", address_of(&rich())));
        assert_eq!(result["toAddress"], format!("{FRESH:#x}"));
        assert_eq!(result["gasUsed"], 21_000);
        assert_eq!(result["gasPrice"], GWEI.to_string());
        assert_eq!(result["gasFees"], fees.to_string());
        assert_eq!(result["coinbaseDiff"], fees.to_string());
        assert_eq!(result["ethSentToCoinbase"], "0");
        assert_eq!(result["value"], "0x");
        assert!(result.get("revert").is_none());
        assert_eq!(response["coinbaseDiff"], fees.to_string());
        assert_eq!(response["gasFees"], fees.to_string());
        assert_eq!(response["ethSentToCoinbase"], "0");
        assert_eq!(response["bundleGasPrice"], GWEI.to_string());
        assert_eq!(response["totalGasUsed"], 21_000);
        assert_eq!(response["stateBlockNumber"], 0);
    }

    #[tokio::test]
    async fn direct_coinbase_payment_is_eth_sent_to_coinbase() {
        let coinbase = Address::repeat_byte(0xcb);
        let tx = Tx {
            to: TxKind::Call(PAYER),
            value: ETHER,
            gas: 100_000,
            ..Default::default()
        }
        .signed(&rich());
        let response = call_bundle(json!({
            "txs": [raw(&tx)], "blockNumber": "0x1", "stateBlockNumber": "0x0",
            "coinbase": format!("{coinbase:#x}"),
        }))
        .await
        .unwrap();
        let result = &response["results"][0];
        let gas_used = u128::from(result["gasUsed"].as_u64().unwrap());
        assert!(gas_used > 21_000);
        let fees = gas_used * u128::from(GWEI);
        assert_eq!(wei(&result["gasFees"]), fees);
        assert_eq!(wei(&result["ethSentToCoinbase"]), ETHER);
        assert_eq!(wei(&result["coinbaseDiff"]), ETHER + fees);
        assert_eq!(wei(&response["ethSentToCoinbase"]), ETHER);
        assert_eq!(wei(&response["coinbaseDiff"]), ETHER + fees);
        assert_eq!(
            wei(&response["bundleGasPrice"]),
            (ETHER + fees) / gas_used,
            "bundleGasPrice is coinbaseDiff / totalGasUsed"
        );
    }

    #[tokio::test]
    async fn gas_fees_use_the_effective_tip_over_the_next_base_fee() {
        let legacy = sign(
            Transaction::LegacyTransaction(LegacyTransaction {
                nonce: 0,
                gas_price: U256::from(3 * GWEI),
                gas: 21_000,
                to: TxKind::Call(FRESH),
                ..Default::default()
            }),
            &rich(),
        );
        let capped = Tx {
            nonce: 1,
            max_fee: GWEI + GWEI / 2,
            priority_fee: GWEI,
            ..Default::default()
        }
        .signed(&rich());
        let response = call_bundle(json!({
            "txs": [raw(&legacy), raw(&capped)], "blockNumber": "0x1", "stateBlockNumber": "latest",
        }))
        .await
        .unwrap();
        let legacy_tip = 3 * GWEI - NEXT_BASE_FEE;
        let capped_tip = GWEI + GWEI / 2 - NEXT_BASE_FEE;
        let results = &response["results"];
        assert_eq!(results[0]["gasPrice"], legacy_tip.to_string());
        assert_eq!(wei(&results[0]["gasFees"]), 21_000 * u128::from(legacy_tip));
        assert_eq!(results[1]["gasPrice"], capped_tip.to_string());
        assert_eq!(wei(&results[1]["gasFees"]), 21_000 * u128::from(capped_tip));
        let total = 21_000 * u128::from(legacy_tip + capped_tip);
        assert_eq!(wei(&response["gasFees"]), total);
        assert_eq!(wei(&response["coinbaseDiff"]), total);
        assert_eq!(response["totalGasUsed"], 42_000);
        assert_eq!(wei(&response["bundleGasPrice"]), total / 42_000);
    }

    #[tokio::test]
    async fn revert_is_a_result_and_the_bundle_continues() {
        let reverting = Tx {
            to: TxKind::Call(REVERTER),
            gas: 100_000,
            ..Default::default()
        }
        .signed(&rich());
        let transfer = Tx {
            nonce: 1,
            value: 1,
            ..Default::default()
        }
        .signed(&rich());
        let response = call_bundle(json!({
            "txs": [raw(&reverting), raw(&transfer)], "blockNumber": "0x1", "stateBlockNumber": "latest",
        }))
        .await
        .unwrap();
        let results = &response["results"];
        assert_eq!(results[0]["revert"], "0xdeadbeef");
        assert!(results[0].get("value").is_none());
        let reverted_gas = results[0]["gasUsed"].as_u64().unwrap();
        assert!(reverted_gas > 21_000);
        assert_eq!(
            wei(&results[0]["gasFees"]),
            u128::from(reverted_gas) * u128::from(GWEI)
        );
        assert_eq!(results[0]["coinbaseDiff"], results[0]["gasFees"]);
        assert_eq!(results[1]["value"], "0x");
        assert!(results[1].get("revert").is_none());
        assert_eq!(response["totalGasUsed"], reverted_gas + 21_000);
    }

    #[tokio::test]
    async fn transactions_chain_state() {
        let rich_to_poor = Tx {
            to: TxKind::Call(address_of(&poor())),
            value: ETHER,
            ..Default::default()
        }
        .signed(&rich());
        let rich_again = Tx {
            nonce: 1,
            ..Default::default()
        }
        .signed(&rich());
        let poor_spends = Tx {
            value: ETHER / 2,
            ..Default::default()
        }
        .signed(&poor());
        let response = call_bundle(json!({
            "txs": [raw(&rich_to_poor), raw(&rich_again), raw(&poor_spends)],
            "blockNumber": "0x1", "stateBlockNumber": "latest",
        }))
        .await
        .unwrap();
        let results = response["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|result| result.get("revert").is_none()));
        assert_eq!(
            results[2]["fromAddress"],
            format!("{:#x}", address_of(&poor()))
        );
        assert_eq!(response["totalGasUsed"], 63_000);
    }

    #[tokio::test]
    async fn invalid_transactions_fail_the_bundle_with_reths_errors() {
        let nonce_too_high = Tx {
            nonce: 5,
            ..Default::default()
        }
        .signed(&rich());
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&nonce_too_high)], "blockNumber": "0x1", "stateBlockNumber": "latest",
            }))
            .await,
            (-32000, "nonce too high".to_string())
        );

        let first = Tx::default().signed(&rich());
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&first), raw(&first)], "blockNumber": "0x1", "stateBlockNumber": "latest",
            }))
            .await,
            (
                -32000,
                "nonce too low: next nonce 1, tx nonce 0".to_string()
            )
        );

        let unfunded = Tx::default().signed(&poor());
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&unfunded)], "blockNumber": "0x1", "stateBlockNumber": "latest",
            }))
            .await,
            (
                -32003,
                "insufficient funds for gas * price + value".to_string()
            )
        );

        let wrong_chain = Tx {
            chain_id: 1,
            ..Default::default()
        }
        .signed(&rich());
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&wrong_chain)], "blockNumber": "0x1", "stateBlockNumber": "latest",
            }))
            .await,
            (-32000, "invalid chain ID".to_string())
        );
    }

    #[tokio::test]
    async fn fee_cap_below_the_base_fee_is_rejected() {
        let tx = Tx::default().signed(&rich());
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&tx)], "blockNumber": "0x1", "stateBlockNumber": "latest",
                "baseFee": format!("{:#x}", 3 * GWEI),
            }))
            .await,
            (
                -32000,
                "max fee per gas less than block base fee".to_string()
            )
        );
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&tx)], "blockNumber": "0x1", "stateBlockNumber": "latest",
                "baseFee": "0xffffffffffffffffffffffffffffffff",
            }))
            .await
            .0,
            -32000
        );
    }

    #[tokio::test]
    async fn gas_limit_bounds() {
        let tx = Tx {
            gas: 100_000,
            ..Default::default()
        }
        .signed(&rich());
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&tx)], "blockNumber": "0x1", "stateBlockNumber": "latest",
                "gasLimit": DEFAULT_RPC_GAS_CAP + 1,
            }))
            .await,
            (-32000, "intrinsic gas too high".to_string())
        );
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&tx)], "blockNumber": "0x1", "stateBlockNumber": "latest",
                "gasLimit": "0xc350",
            }))
            .await,
            (-32000, "intrinsic gas too high".to_string())
        );
    }

    #[tokio::test]
    async fn the_bundle_has_to_fit_the_block_gas_limit() {
        let first = Tx::default().signed(&rich());
        let second = Tx {
            nonce: 1,
            ..Default::default()
        }
        .signed(&rich());
        // Each transaction fits the block on its own; together they do not.
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&first), raw(&second)], "blockNumber": "0x1",
                "stateBlockNumber": "latest", "gasLimit": 30_000,
            }))
            .await,
            (-32000, "intrinsic gas too high".to_string())
        );
        let response = call_bundle(json!({
            "txs": [raw(&first), raw(&second)], "blockNumber": "0x1",
            "stateBlockNumber": "latest", "gasLimit": 42_000,
        }))
        .await
        .unwrap();
        assert_eq!(response["totalGasUsed"], 42_000);
    }

    #[tokio::test]
    async fn blockhash_resolves_up_to_the_state_block_and_is_zero_past_it() {
        let store = store().await;
        let block_1 = add_block_1(&store).await;
        let tx = Tx {
            to: TxKind::Call(HASHER),
            gas: 100_000,
            ..Default::default()
        }
        .signed(&rich());
        let previous_hash = |block_number: &str| {
            let store = store.clone();
            let bundle = json!({
                "txs": [raw(&tx)], "blockNumber": block_number, "stateBlockNumber": "latest",
            });
            async move {
                let response = call_bundle_on(store, bundle).await.unwrap();
                response["results"][0]["value"]
                    .as_str()
                    .unwrap()
                    .to_string()
            }
        };
        // Right after the state block, BLOCKHASH(1) is the state block's hash.
        assert_eq!(previous_hash("0x2").await, format!("{:#x}", block_1.hash()));
        // Further ahead, BLOCKHASH(29) is a block that does not exist yet: zero, not an
        // error.
        assert_eq!(previous_hash("0x1e").await, format!("{:#x}", H256::zero()));
    }

    async fn env_words(store: Store, overrides: Value) -> (Value, [U256; 5]) {
        let tx = Tx {
            to: TxKind::Call(ENV),
            gas: 100_000,
            max_fee: 10 * GWEI,
            ..Default::default()
        }
        .signed(&rich());
        let mut bundle =
            json!({"txs": [raw(&tx)], "blockNumber": "0x64", "stateBlockNumber": "latest"});
        for (key, value) in overrides.as_object().unwrap() {
            bundle[key] = value.clone();
        }
        let response = call_bundle_on(store, bundle).await.unwrap();
        let value = response["results"][0]["value"]
            .as_str()
            .unwrap()
            .to_string();
        (response, [0, 1, 2, 3, 4].map(|index| word(&value, index)))
    }

    #[tokio::test]
    async fn block_env_defaults() {
        let store = store().await;
        let genesis = store.get_block_header(0).unwrap().unwrap();
        let (response, [number, timestamp, base_fee, coinbase, gas_limit]) =
            env_words(store, json!({})).await;
        assert_eq!(number, U256::from(100));
        assert_eq!(timestamp, U256::from(genesis.timestamp + 12));
        assert_eq!(base_fee, U256::from(NEXT_BASE_FEE));
        assert_eq!(coinbase, U256::from_big_endian(genesis.coinbase.as_bytes()));
        assert_eq!(gas_limit, U256::from(DEFAULT_RPC_GAS_CAP));
        assert_eq!(response["stateBlockNumber"], 0);
    }

    #[tokio::test]
    async fn block_env_overrides() {
        let coinbase = Address::repeat_byte(0xcb);
        let (response, [number, timestamp, base_fee, coinbase_word, gas_limit]) = env_words(
            store().await,
            json!({
                "blockNumber": 1234,
                "timestamp": 2_000_000_000u64,
                "baseFee": "0x7",
                "coinbase": format!("{coinbase:#x}"),
                "gasLimit": "30000000",
                "difficulty": "0x0",
            }),
        )
        .await;
        assert_eq!(number, U256::from(1234));
        assert_eq!(timestamp, U256::from(2_000_000_000u64));
        assert_eq!(base_fee, U256::from(7));
        assert_eq!(coinbase_word, U256::from_big_endian(coinbase.as_bytes()));
        assert_eq!(gas_limit, U256::from(30_000_000));
        assert_eq!(response["results"][0]["gasPrice"], GWEI.to_string());
    }

    #[tokio::test]
    async fn state_block_number_takes_numbers_and_tags() {
        let store = store().await;
        let block_1 = add_block_1(&store).await;
        let next_base_fee = NEXT_BASE_FEE + NEXT_BASE_FEE * 3 / 40;
        for (state_block, number) in [
            ("latest", 1),
            ("LATEST", 1),
            ("pending", 1),
            ("0x1", 1),
            ("0x0", 0),
            ("earliest", 0),
        ] {
            let (response, [_, timestamp, base_fee, coinbase, _]) =
                env_words(store.clone(), json!({ "stateBlockNumber": state_block })).await;
            assert_eq!(response["stateBlockNumber"], number, "{state_block}");
            if number == 1 {
                assert_eq!(timestamp, U256::from(block_1.timestamp + 12));
                assert_eq!(base_fee, U256::from(next_base_fee));
                assert_eq!(coinbase, U256::from_big_endian(block_1.coinbase.as_bytes()));
            }
        }
    }

    #[tokio::test]
    async fn unknown_state_block() {
        let tx = Tx::default().signed(&rich());
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&tx)], "blockNumber": "0x11", "stateBlockNumber": "0x10",
            }))
            .await,
            (-32001, "block not found: 0x10".to_string())
        );
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&tx)], "blockNumber": "0x1", "stateBlockNumber": 0,
            }))
            .await
            .0,
            -32602
        );
        assert_eq!(
            call_bundle_err(json!({
                "txs": [raw(&tx)], "blockNumber": "0x1", "stateBlockNumber": "10",
            }))
            .await
            .0,
            -32602
        );
    }

    #[tokio::test]
    async fn request_shape_errors() {
        let tx = raw(&Tx::default().signed(&rich()));
        assert_eq!(
            call_bundle_err(json!({"txs": [], "blockNumber": "0x1", "stateBlockNumber": "latest"}))
                .await,
            (-32602, "bundle missing txs".to_string())
        );
        assert_eq!(
            call_bundle_err(
                json!({"txs": [tx], "blockNumber": "0x0", "stateBlockNumber": "latest"})
            )
            .await,
            (-32602, "bundle missing blockNumber".to_string())
        );
        assert_eq!(
            call_bundle_err(json!({"txs": [tx], "stateBlockNumber": "latest"}))
                .await
                .0,
            -32602
        );
        assert_eq!(
            call_bundle_err(json!({"txs": [tx], "blockNumber": "0x1"}))
                .await
                .0,
            -32602
        );
        let too_many = vec![tx.clone(); MAX_CALL_BUNDLE_TXS + 1];
        assert_eq!(
            call_bundle_err(
                json!({"txs": too_many, "blockNumber": "0x1", "stateBlockNumber": "latest"})
            )
            .await
            .0,
            -32005
        );
    }

    #[tokio::test]
    async fn malformed_raw_transactions() {
        let bundle =
            |tx: &str| json!({"txs": [tx], "blockNumber": "0x1", "stateBlockNumber": "latest"});
        let undecodable = (-32602, "failed to decode signed transaction".to_string());
        assert_eq!(call_bundle_err(bundle("0x1234")).await, undecodable);
        assert_eq!(
            call_bundle_err(bundle("0x")).await,
            (-32602, "empty transaction data".to_string())
        );
        let trailing = format!("{}00", raw(&Tx::default().signed(&rich())));
        assert_eq!(call_bundle_err(bundle(&trailing)).await, undecodable);
        let blobless = Transaction::EIP4844Transaction(EIP4844Transaction {
            chain_id: CHAIN_ID,
            to: FRESH,
            blob_versioned_hashes: vec![H256::repeat_byte(1)],
            ..Default::default()
        });
        assert_eq!(call_bundle_err(bundle(&raw(&blobless))).await, undecodable);
        let unsigned = Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id: CHAIN_ID,
            gas_limit: 21_000,
            to: TxKind::Call(FRESH),
            ..Default::default()
        });
        assert_eq!(
            call_bundle_err(bundle(&raw(&unsigned))).await,
            (-32602, "invalid transaction signature".to_string())
        );
        assert_eq!(call_bundle_err(bundle("0xzz")).await.0, -32602);
    }

    #[tokio::test]
    async fn bundle_hash_is_keccak_of_the_tx_hashes() {
        let first = Tx::default().signed(&rich());
        let second = Tx {
            nonce: 1,
            ..Default::default()
        }
        .signed(&rich());
        let response = call_bundle(json!({
            "txs": [raw(&first), raw(&second)], "blockNumber": "0x1", "stateBlockNumber": "latest",
        }))
        .await
        .unwrap();
        let mut hashes = first.hash(&NativeCrypto).as_bytes().to_vec();
        hashes.extend_from_slice(second.hash(&NativeCrypto).as_bytes());
        assert_eq!(
            response["bundleHash"],
            format!("{:#x}", H256(NativeCrypto.keccak256(&hashes)))
        );
        assert_eq!(response["results"][0]["txHash"], hash(&first));
        assert_eq!(response["results"][1]["txHash"], hash(&second));
        assert_eq!(response["bundleGasPrice"], GWEI.to_string());
    }

    async fn mixed_bundle_response() -> Value {
        let success = Tx {
            to: TxKind::Call(PAYER),
            value: ETHER,
            gas: 100_000,
            ..Default::default()
        }
        .signed(&rich());
        let reverting = Tx {
            nonce: 1,
            to: TxKind::Call(REVERTER),
            gas: 100_000,
            ..Default::default()
        }
        .signed(&rich());
        let creation = Tx {
            nonce: 2,
            to: TxKind::Create,
            gas: 100_000,
            data: Bytes::from(hex::decode("600060005360016000f3").unwrap()),
            ..Default::default()
        }
        .signed(&rich());
        call_bundle(json!({
            "txs": [raw(&success), raw(&reverting), raw(&creation)],
            "blockNumber": "0x1", "stateBlockNumber": "latest",
        }))
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn response_matches_alloys_wire_format() {
        let response = mixed_bundle_response().await;
        let keys = |value: &Value| {
            let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        assert_eq!(
            keys(&response),
            [
                "bundleGasPrice",
                "bundleHash",
                "coinbaseDiff",
                "ethSentToCoinbase",
                "gasFees",
                "results",
                "stateBlockNumber",
                "totalGasUsed"
            ]
        );
        let tx_keys = |last: &'static str| {
            let mut keys = vec![
                "coinbaseDiff",
                "ethSentToCoinbase",
                "fromAddress",
                "gasFees",
                "gasPrice",
                "gasUsed",
                "toAddress",
                "txHash",
                last,
            ];
            keys.sort();
            keys
        };
        let results = response["results"].as_array().unwrap();
        assert_eq!(keys(&results[0]), tx_keys("value"));
        assert_eq!(keys(&results[1]), tx_keys("revert"));
        assert_eq!(keys(&results[2]), tx_keys("value"));
        assert!(results[2]["toAddress"].is_null());
        assert_eq!(results[2]["value"], "0x00");

        let is_decimal = |value: &Value| {
            let text = value.as_str().unwrap();
            !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
        };
        for field in [
            "bundleGasPrice",
            "coinbaseDiff",
            "ethSentToCoinbase",
            "gasFees",
        ] {
            assert!(is_decimal(&response[field]), "{field}: {}", response[field]);
        }
        assert!(response["stateBlockNumber"].is_u64());
        assert!(response["totalGasUsed"].is_u64());
        assert_eq!(response["bundleHash"].as_str().unwrap().len(), 66);
        for result in results {
            for field in ["coinbaseDiff", "ethSentToCoinbase", "gasFees", "gasPrice"] {
                assert!(is_decimal(&result[field]), "{field}: {}", result[field]);
            }
            assert!(result["gasUsed"].is_u64());
            assert_eq!(result["txHash"].as_str().unwrap().len(), 66);
            assert_eq!(result["fromAddress"].as_str().unwrap().len(), 42);
        }
    }

    fn alloy_numeric<'de, D: Deserializer<'de>>(d: D) -> Result<U256, D::Error> {
        match Value::deserialize(d)? {
            Value::String(text) => match text.parse::<u128>() {
                Ok(value) => Ok(U256::from(value)),
                Err(_) => U256::from_str(text.trim_start_matches("0x")).map_err(D::Error::custom),
            },
            Value::Number(number) => number
                .as_u64()
                .map(U256::from)
                .ok_or_else(|| D::Error::custom("invalid u256")),
            _ => Err(D::Error::custom("invalid u256")),
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    struct AlloyResponse {
        bundle_hash: H256,
        #[serde(deserialize_with = "alloy_numeric")]
        bundle_gas_price: U256,
        #[serde(deserialize_with = "alloy_numeric")]
        coinbase_diff: U256,
        #[serde(deserialize_with = "alloy_numeric")]
        eth_sent_to_coinbase: U256,
        #[serde(deserialize_with = "alloy_numeric")]
        gas_fees: U256,
        results: Vec<AlloyTxResult>,
        state_block_number: u64,
        total_gas_used: u64,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    struct AlloyTxResult {
        #[serde(deserialize_with = "alloy_numeric")]
        coinbase_diff: U256,
        #[serde(deserialize_with = "alloy_numeric")]
        eth_sent_to_coinbase: U256,
        from_address: Address,
        #[serde(deserialize_with = "alloy_numeric")]
        gas_fees: U256,
        #[serde(deserialize_with = "alloy_numeric")]
        gas_price: U256,
        gas_used: u64,
        to_address: Option<Address>,
        tx_hash: H256,
        #[serde(default, with = "opt_bytes")]
        value: Option<Bytes>,
        #[serde(default, with = "opt_bytes")]
        revert: Option<Bytes>,
    }

    mod opt_bytes {
        use super::*;
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Bytes>, D::Error> {
            Option::<String>::deserialize(d)?
                .map(|text| {
                    hex::decode(text.trim_start_matches("0x"))
                        .map(Bytes::from)
                        .map_err(D::Error::custom)
                })
                .transpose()
        }
        #[allow(dead_code)]
        pub fn serialize<S: Serializer>(_: &Option<Bytes>, _: S) -> Result<S::Ok, S::Error> {
            unreachable!()
        }
    }

    /// A client that takes wei amounts only as strings and gas and block numbers only as
    /// JSON numbers.
    fn numeric_string<'de, D: Deserializer<'de>>(d: D) -> Result<U256, D::Error> {
        let text = String::deserialize(d)?;
        match text.strip_prefix("0x") {
            Some(hex) => U256::from_str_radix(hex, 16).map_err(D::Error::custom),
            None => U256::from_dec_str(&text).map_err(|e| D::Error::custom(format!("{e:?}"))),
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    struct StrictResult {
        #[serde(deserialize_with = "numeric_string")]
        coinbase_diff: U256,
        results: Vec<StrictTx>,
        state_block_number: u64,
        total_gas_used: u64,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    struct StrictTx {
        #[serde(deserialize_with = "numeric_string")]
        coinbase_diff: U256,
        #[serde(deserialize_with = "numeric_string")]
        eth_sent_to_coinbase: U256,
        from_address: Address,
        #[serde(deserialize_with = "numeric_string")]
        gas_fees: U256,
        #[serde(deserialize_with = "numeric_string")]
        gas_price: U256,
        gas_used: u64,
        to_address: Option<Address>,
        tx_hash: H256,
        #[serde(default, with = "opt_bytes")]
        revert: Option<Bytes>,
    }

    #[tokio::test]
    async fn response_parses_as_alloy_types_and_a_strict_client() {
        let response = mixed_bundle_response().await;

        let alloy: AlloyResponse = serde_json::from_value(response.clone()).unwrap();
        assert_eq!(alloy.results.len(), 3);
        assert_eq!(alloy.state_block_number, 0);
        assert_eq!(alloy.eth_sent_to_coinbase, U256::from(ETHER));
        assert_eq!(alloy.coinbase_diff, alloy.gas_fees + U256::from(ETHER));
        assert_eq!(
            alloy.results[1].revert.as_deref(),
            Some(&[0xde, 0xad, 0xbe, 0xef][..])
        );
        assert!(alloy.results[0].value.is_some() && alloy.results[0].revert.is_none());
        assert!(alloy.results[2].to_address.is_none());
        assert_eq!(
            alloy.total_gas_used,
            alloy
                .results
                .iter()
                .map(|result| result.gas_used)
                .sum::<u64>()
        );

        let strict: StrictResult = serde_json::from_value(response).unwrap();
        assert_eq!(strict.results.len(), 3);
        assert_eq!(strict.coinbase_diff, alloy.coinbase_diff);
        assert!(strict.results[0].revert.is_none());
        assert!(strict.results[1].revert.is_some());
        assert_eq!(strict.results[0].eth_sent_to_coinbase, U256::from(ETHER));
    }

    /// Every number a `0x` quantity, `stateBlockNumber` a number, `timeout` set.
    #[tokio::test]
    async fn quantity_request_shape() {
        let store = store().await;
        let genesis = store.get_block_header(0).unwrap().unwrap();
        let tx = Tx::default().signed(&rich());
        let response = call_bundle_on(
            store,
            json!({
                "txs": [raw(&tx)],
                "blockNumber": "0x1",
                "stateBlockNumber": "0x0",
                "timestamp": format!("{:#x}", genesis.timestamp + 12),
                "timeout": "0x1",
                "baseFee": format!("{:#x}", NEXT_BASE_FEE),
            }),
        )
        .await
        .unwrap();
        let parsed: StrictResult = serde_json::from_value(response).unwrap();
        assert_eq!(parsed.state_block_number, 0);
        assert_eq!(parsed.total_gas_used, 21_000);
        assert_eq!(parsed.results[0].gas_price, U256::from(GWEI));
    }

    #[tokio::test]
    async fn alloy_request_shape() {
        let tx = Tx::default().signed(&rich());
        let response = call_bundle(json!({
            "txs": [raw(&tx)],
            "blockNumber": "0x1",
            "stateBlockNumber": "latest",
            "transactionIndex": -1,
            "coinbase": format!("{:#x}", Address::repeat_byte(0xcb)),
            "timestamp": 1_718_040_093u64,
            "timeout": 500,
            "gasLimit": 30_000_000,
            "difficulty": "0x0",
            "baseFee": "0x342770c0",
        }))
        .await
        .unwrap();
        assert_eq!(response["results"][0]["gasPrice"], GWEI.to_string());
    }
}
