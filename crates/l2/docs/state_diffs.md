# State diffs

This architecture was inspired by MatterLabs' ZKsync pubdata architecture (see [here](https://github.com/matter-labs/zksync-era/blob/main/docs/specs/data_availability/pubdata.md)).

To provide data availability for our network, we need to publish enough information on every commit transaction to be able to reconstruct the entire state of the L2 from the beginning by querying the L1.

The data needed is:

- The nonce and balance of every `EOA`.
- The nonce, balance, and storage of every contract account. Note that storage here is a mapping `(U256 → U256)`, so there are a lot of values inside it.
- The bytecode of every contract deployed on the network.
- All withdrawal Logs.

After executing each L2 block, the EVM will return the following data:

- A list of every storage slot modified in the block, with their previous and next values. A storage slot is a mapping `(address, slot) -> value`. Note that, on a block, there could be repeated writes to the same slot. In that case, we keep only the latest write; all the others are discarded since they are not needed for state reconstruction.
- The bytecode of every newly deployed contract. Every contract deployed is then a pair `(address, bytecode)`.
- A list of withdrawal logs (as explained in milestone 1 we already collect these and publish a merkle root of their values as calldata, but we still need to send them as the state diff).
- A list of triples `(address, nonce_increase, balance)` for every modified account. The `nonce_increase` is a value that says by how much the nonce of the account was increased on the block (this could be more than one as there can be multiple transactions for the account on the block). The balance is just the new balance value for the account.

The full state diff sent on every block will then be a sequence of bytes encoded as follows. We use the notation `un` for a sequence of `n` bits, so `u16` is a 16-bit sequence and `u96` a 96-bit one, we don’t really care about signedness here; if we don’t specify it, the value is of variable length and a field before it specifies it.

- The first byte is a `u8`: the version header. For now it should always be zero, but we reserve it for future changes to the encoding/compression format.
- Next come the `ModifiedAccounts` list. The first two bytes (`u16`) are the amount of element it has, followed by its entries. Each entry correspond to an altered address and has the form:
  - The first byte is the `type` of the modification. The value is a `u8`, constrained to the range `[1; 23]`, computed by adding the following values:
    - `1` if the balance of the EOA/contract was modified.
    - `2` if the nonce of the EOA/contract was modified.
    - `4` if the storage of the contract was modified.
    - `8` if the contract was created and the bytecode is previously unknown.
    - `16` if the contract was created and the bytecode is previously known.
  - The next 20 bytes, a `u160`, is the address of the modified account.
  - If the balance was modified (i.e. `type & 0x01 == 1`), the next 32 bytes, a `u256`, is the new balance of the account.
  - If the nonce was modified (i.e. `type & 0x02 == 2`), the next 2 bytes, a `u16`, is the increase in the nonce.
  - If the storage was modified (i.e. `type & 0x04 == 4`), the next 2 bytes, a `u16`, is the number of storage slots modified. Then come the sequence of `(key_u256, new_value_u256)` key value pairs with the modified slots.
  - If the contract was created and the bytecode is previously unknown (i.e. `type & 0x08 == 8`), the next 2 bytes, a `u16`, is the length of the bytecode in bytes. Then come the bytecode itself.
  - If the contract was created and the bytecode is previously known (i.e. `type & 0x10 == 16`), the next 20 bytes, a `u160`, is the hash of the bytecode of the contract.
  - Note that values `8` and `16` are mutually exclusive, and if `type` is greater or equal to `4`, then the address is a contract. Each address can only appear once in the list.
- Next the `WithdrawalLogs` field:
    - First two bytes are the number of entries, then come the tuples `(to_u160, amount_u256, tx_hash_u256)`.
- Next the `DepositLogs` field:
    - First two bytes are the number of entries, then come the last 30 bytes of the `keccak` encoding of the concatenation of deposits with form `keccack256(to_u160 || value_u256)`.
- In case of the only changes on an account are produced by withdrawals, the `ModifiedAccounts` for that address field must be omitted. In this case, the state diff can be computed by incrementing the nonce in one unit and subtracting the amount from the balance.

To recap, using `||` for byte concatenation and `[]` for optional parameters, the full encoding for state diffs is:

```jsx
version_header_u8 ||
// Modified Accounts
number_of_modified_accounts_u16 ||
(
  type_u8 || address_u160 || [balance_u256] || [nonce_increase_u16] ||
  [number_of_modified_storage_slots_u16 || (key_u256 || value_u256)... ] ||
  [bytecode_len_u16 || bytecode ...] ||
  [code_hash_u256]
)...
// Withdraw Logs
number_of_withdraw_logs_u16 ||
(to_u160 || amount_u256 || tx_hash_u256) ...
// Deposit Logs
number_of_deposit_logs_u16 || keccak256(keccack256(to_u160 || value_u256) || ...)[2:32]
```

The sequencer will then make a commitment to this encoded state diff (explained in the EIP 4844 section how this is done) and send on the `commit` transaction:

- Through calldata, the state diff commitment (which is part of the public input to the proof).
- Through the blob, the encoded state diff.
