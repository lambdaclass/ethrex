# Withdrawal specs

This document contains a detailed explanation of the changes needed to handled withdrawals and the withdrawal flow.

A new `Withdraw` type of transaction on L2 is introduced, where users send a certain amount of `eth` (or the native token in a custom token setup) with it. This money is then burned on the L2 and the operator sends a `WithdrawLog` to L1, so the user can then send a transaction to claim the withdrawal associated to that log and receive their funds from the Common bridge.

In more detail, the full changes/additions are:

- A `Withdraw` transaction type is introduced, comprised of the regular fields in an EIP-1559 transaction.
- On every block, each `Withdraw` transaction will burn (i.e. deduct from the sender) the value attached to it.
- After executing the block, the sequencer will collect all `Withdraw` transactions, will generate a `WithdrawLog` for each, will build a merkle tree from them and calculate the corresponding root, which we call `WithdrawLogsRoot`. The `WithdrawLog` contains the following fields:
    - `to`: the address in L1 that is allowed to claim the funds (this is decided by the user as part of a withdraw transaction. This comes from the regular `to` field on the Withdraw transaction (i.e. we are reusing that field with a slightly different meaning; what it means here is “the address that can claim the funds on L1”).
    - `amount`: the amount of money withdrawn (i.e. the `msg.value` of the transaction).
    - `tx_hash`: the transaction hash in the L2 block it was included in. This will be important for claiming the withdrawal as it will require a merkle proof to be provided along with the index on the tree.
- As part of the L1 `commit` transaction, the sequencer will send the list of all `WithdrawLog`s on the EIP 4844 blob (i.e. as a section of the state diffs) and the `WithdrawLogsRoot` as calldata as part of the public input to the proof. The contract will then:
    - Verify that the withdraw logs passed on the blob are the correct ones (this is done as part of the proof of equivalence protocol explained below).
    - Store the `WithdrawLogsRoot` on a mapping `(blockNumber -> LogsRoot)`
- For users to complete their withdraw process and receive funds on the L1, they need to call a `claimWithdraw(withdrawLog, merkleProof, blockNumber)` function on the common bridge, where `merkleProof` is an inclusion proof of the withdraw log to the root of the merkle tree the contract has stored. The contract will then do the following:
    - Check that the `blockNumber` corresponds to a committed and verified block.
    - Check that this withdrawal has not been already claimed.
    - Retrieve the `withdrawLogsRoot` from the given `blockNumber`.
    - Verify the merkle proof given by the user, passing the proof, the root, and the `tx_hash`.
    - If any check above failed, revert. If all checks passed, send the appropriate funds to the user, then set the `withdrawLog` as claimed.
    - After the withdrawal is sent, we mark it as claimed so it cannot be claimed twice.
