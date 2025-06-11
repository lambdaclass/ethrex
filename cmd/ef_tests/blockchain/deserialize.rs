use crate::types::{BlockChainExpectedException, BlockExpectedException};
use ef_tests_state::types::TransactionExpectedException;
use serde::{Deserialize, Deserializer};

pub fn deserialize_block_expected_exception<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<BlockChainExpectedException>>, D::Error>
where
    D: Deserializer<'de>,
{
    let option: Option<String> = Option::deserialize(deserializer)?;

    if let Some(value) = option {
        let exceptions = value
            .split('|')
            .map(|s| match s.trim() {
                "TransactionException.INITCODE_SIZE_EXCEEDED" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::InitcodeSizeExceeded,
                    )
                }
                "TransactionException.NONCE_IS_MAX" => BlockChainExpectedException::TxtException(
                    TransactionExpectedException::NonceIsMax,
                ),
                "TransactionException.TYPE_3_TX_BLOB_COUNT_EXCEEDED" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::Type3TxBlobCountExceeded,
                    )
                }
                "TransactionException.TYPE_3_TX_ZERO_BLOBS" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::Type3TxZeroBlobs,
                    )
                }
                "TransactionException.TYPE_3_TX_CONTRACT_CREATION" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::Type3TxContractCreation,
                    )
                }
                "TransactionException.TYPE_3_TX_INVALID_BLOB_VERSIONED_HASH" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::Type3TxInvalidBlobVersionedHash,
                    )
                }
                "TransactionException.INTRINSIC_GAS_TOO_LOW" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::IntrinsicGasTooLow,
                    )
                }
                "TransactionException.INSUFFICIENT_ACCOUNT_FUNDS" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::InsufficientAccountFunds,
                    )
                }
                "TransactionException.SENDER_NOT_EOA" => BlockChainExpectedException::TxtException(
                    TransactionExpectedException::SenderNotEoa,
                ),
                "TransactionException.PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::PriorityGreaterThanMaxFeePerGas,
                    )
                }
                "TransactionException.GAS_ALLOWANCE_EXCEEDED" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::GasAllowanceExceeded,
                    )
                }
                "TransactionException.INSUFFICIENT_MAX_FEE_PER_GAS" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::InsufficientMaxFeePerGas,
                    )
                }
                "TransactionException.RLP_INVALID_VALUE" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::RlpInvalidValue,
                    )
                }
                "TransactionException.GASLIMIT_PRICE_PRODUCT_OVERFLOW" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::GasLimitPriceProductOverflow,
                    )
                }
                "TransactionException.TYPE_3_TX_PRE_FORK" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::Type3TxPreFork,
                    )
                }
                "TransactionException.TYPE_4_TX_CONTRACT_CREATION" => {
                    BlockChainExpectedException::RLPException
                }
                "TransactionException.INSUFFICIENT_MAX_FEE_PER_BLOB_GAS" => {
                    BlockChainExpectedException::TxtException(
                        TransactionExpectedException::InsufficientMaxFeePerBlobGas,
                    )
                }
                "BlockException.RLP_STRUCTURES_ENCODING" => {
                    BlockChainExpectedException::RLPException
                }
                "BlockException.INCORRECT_BLOB_GAS_USED" => {
                    BlockChainExpectedException::BlockException(
                        BlockExpectedException::IncorrectBlobGasUsed,
                    )
                }
                "BlockException.BLOB_GAS_USED_ABOVE_LIMIT" => {
                    BlockChainExpectedException::BlockException(
                        BlockExpectedException::BlobGasUsedAboveLimit,
                    )
                }
                "BlockException.INCORRECT_EXCESS_BLOB_GAS" => {
                    BlockChainExpectedException::BlockException(
                        BlockExpectedException::IncorrectExcessBlobGas,
                    )
                }
                "BlockException.INCORRECT_BLOCK_FORMAT" => {
                    BlockChainExpectedException::BlockException(
                        BlockExpectedException::IncorrectBlockFormat,
                    )
                }
                "BlockException.INVALID_REQUESTS" => BlockChainExpectedException::BlockException(
                    BlockExpectedException::InvalidRequest,
                ),
                "BlockException.SYSTEM_CONTRACT_EMPTY" => {
                    BlockChainExpectedException::BlockException(
                        BlockExpectedException::SystemContractEmpty,
                    )
                }
                "BlockException.SYSTEM_CONTRACT_CALL_FAILED" => {
                    BlockChainExpectedException::BlockException(
                        BlockExpectedException::SystemContractCallFailed,
                    )
                }
                _ => BlockChainExpectedException::Other,
            })
            .collect();

        Ok(Some(exceptions))
    } else {
        Ok(None)
    }
}
