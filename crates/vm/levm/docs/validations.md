## Transaction Validation

1. **GASLIMIT_PRICE_PRODUCT_OVERFLOW** -> The product of gas limit and gas price is too high.
2. **INSUFFICIENT_ACCOUNT_FUNDS** -> Sender does not have enough funds to pay for the gas.
3. **INSUFFICIENT_MAX_FEE_PER_GAS** -> The max fee per gas is lower than the base fee per gas.
4. **INITCODE_SIZE_EXCEEDED** -> The size of the initcode is too big.
5. **INTRINSIC_GAS_TOO_LOW** -> The gas limit is lower than the intrinsic gas.
6. **NONCE_IS_MAX** -> The nonce of the sender is at its maximum value.
7. **PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS** -> The priority fee is greater than the max fee per gas.
8. **SENDER_NOT_EOA** -> The sender is not an EOA (it has code).
9. **GAS_ALLOWANCE_EXCEEDED** -> The gas limit is higher than the block gas limit.
10. **INSUFFICIENT_MAX_FEE_PER_BLOB_GAS** -> The max fee per blob gas is lower than the base fee per gas.
11. **TYPE_3_TX_ZERO_BLOBS** -> The transaction has zero blobs.
12. **TYPE_3_TX_INVALID_BLOB_VERSIONED_HASH** -> The blob versioned hash is invalid.
13. **TYPE_3_TX_PRE_FORK** -> The transaction is a pre-cancun transaction.
14. **TYPE_3_TX_BLOB_COUNT_EXCEEDED** -> The blob count is higher than the max allowed.
15. **TYPE_3_TX_CONTRACT_CREATION** -> The type 3 transaction is a contract creation.
