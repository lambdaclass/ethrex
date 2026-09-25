# FAQ
## `usize` and `U256`
In Rust, **accessing an index on a specific data structure requires a `usize` type variable.** This can be seen in methods like `get` and `get_mut`.
For more details on how `U256` is used to represent storage keys and other 256-bit values, see the [State diffs](../../l2/fundamentals/state_diffs.md) documentation.
On the other hand, the EVM specification requires all addresses to be in `U256`. Therefore, every opcode treats its arguments as `U256` values.
The problem arises in the opcodes that need to access a specific index on a data structure (e.g. `CALLDATA`, `CODECOPY`, `EXTCODECOPY`, etc).
These operands receive offsets and indexes in `U256`, but the data structure they have to access (e.g. `Memory` or  `Calldata`) **require a `usize`**. Therefore, those parameters need to be cast from `U256` to `usize`.
The problem is, `U256`'s representation ranger is larger than `usize`'s; so not all numbers can be successfully cast. In these cases, special attention is needed.

The main way to deal with theses cases (at least, at the time of writing) is to **cast the value only when you know it can fit**. Before casting to `usize`, we compare the size of the index in `U256` with the length of the data structure it wants to access. Here's an example from the `EXTCODECOPY` opcode (NOTE: the code snippet is a simplified/altered version to demonstrate this pattern. The actual implementation is fairly different):

```rust
///  bytecode: Represents the EVM bytecode array to be executed.
0:   pub fn op_extcodecopy(bytecode_offset: U256, bytecode: Bytes, vector_size: usize) -> Result<(), Err> {

        (...)

1:       let mut data = vec![0u8; vector_size];
2:
3:       let bytecode_length: U256 = bytecode.len().into();
4:       if bytecode_offset < bytecode_length {
5:           let offset: usize = offset
6:               .try_into()
7:               .map_err(|_| InternalError::TypeConversion)?;
8:           // After this the data vector is modified

        (...)

9:      }
10:     memory.store_data(&data);
11: }
```
Some context: It is not important what this operand does. The only thing that matters for this example is that `EXTCODECOPY` stores a `data` vector in memory. The offset it receives will tell `EXTCODECOPY` which parts of the bytecode to skip, and which parts it will copy to memory. Skipped sections will be filled with 0's.

- In line `1` we create the vector which we will return.
- In line `3` we get the `bytecode` array length. Since `.len()` returns a `usize` we need to cast it to `U256`, in order to compare it with `bytecode_offset`. Luckily, `usize` always fits into `U256`, so this will never fail.
- In line `4` we check if the calldata offset is larger than the calldata itself. If this is the case, there's no data to copy. So we do not want to modify the vector.
    -  Do note that, after this check we can safely cast the bytecode to `usize`. This is done in line `5`. This is because there is a limit to the contract's bytecode size. For more information, read [this article](https://ethereum.org/en/developers/docs/smart-contracts/#limitations).
    -  We return an `InternalError` because line 5 should never fail. If it fails, then it means there's a problem with the VM itself.
- Finally in line `10`, we store the resulting data vector in memory.
    - If the bytecode_offset was larger than the actual contents of the bytecode array, we return a vector with only 0's. This is the intended behavior.


This pattern is fairly common and is useful to keep in mind, especially when dealing with operands that deal with offsets and indexes.


## External vs Internal Transactions

- External transactions are initiated by EOAs (Externally Owned Accounts). These are user-triggered and are the only way to start activity on-chain (e.g., sending ETH, calling a contract).
- Internal transactions are not real transactions in the blockchain data. They are contract-to-contract calls triggered during the execution of external transactions, using opcodes like CALL, DELEGATECALL, CREATE, etc. They’re not recorded in the transaction pool.


## CacheDB vs. Cold and Warm Addresses

This topic often causes confusion. The presence of an address in the cache does not mean it is **warm** — these are two separate concepts.

**Cold & Warm:**
- An address is **cold** if it has not been accessed yet during the current transaction.
- An address is **warm** if it has already been accessed in the current transaction, this could be through a call to that account, by being in the [access list](https://eips.ethereum.org/EIPS/eip-2930), etc.
Accessing a **cold** address incurs higher gas costs than accessing a **warm** address.

**CacheDB:**
- The `CacheDB` is a structure that is persisted between transactions and keeps track of changes that are eventually going to be committed to the Database.

So if you want to access an account that's in the `CacheDB` it will be cheap for the EVM (because it won't look up in the `Database`) but if it was accessed in a transaction that never accessed that account the address will still be **cold** and therefore the gas cost will be higher than if it was **warm**.

## Errors

These are the kinds of errors:
- `InternalError`: These are errors that break execution, they shouldn't ever happen. For example, an underflow when substracting two values and we know for sure the first one is greater than the second.
  - `DatabaseError`: Subcategory of `InternalError`, this error is only used within the `Database` trait that the LEVM crate exposes. This should be returned when there's an unexpected error when trying to read from the database.
- `TxValidation`: These are thrown if the transaction doesn't pass the required validations, like the sender having enough value to pay for the transaction gas fees, or that the transaction nonce should match with sender's nonce. These errors **INVALIDATE** the transaction, they shouldn't make any changes to the state.
- `ExceptionalHalt`: Any error that's contemplated in the EVM and is expected to happen, like Out-of-Gas and Stack Overflow. These errors cause the current executing context to **Revert** consuming all context gas left. Some examples include a Stack Overflow and when bytecode contains an Invalid Opcode.
- `RevertOpcode`: Triggered by Revert Opcode, it behaves like an `ExceptionalHalt` except this one doesn't consume the gas left.

## LevmAccount vs Account
Why not use the same Account struct that the L1 uses? Because it's pretty limited and we wanted a little bit more flexibility in LEVM. We wanted to have an `AccountStatus` so that the VM knows the status of an account at any given moment and also we don't need the account to have code, as the code will be stored in the database directly and we can access it via `code_hash`.
Advantages: 
- We'll fetch the code only if we need to, this means less accesses to the database. 
- If there is duplicate code between accounts (which is pretty common) we'll store it in memory only once. 
- We'll be able to make better decisions without relying on external structures, based on the current status of an Account. e.g. If it was untouched we skip processing it when calculating Account Updates, or if the account has been destroyed and re-created with same address we know that the storage on the Database is not valid and we shouldn't access it, etc.

What do we sacrifice? Just having to switch types when interacting with LEVM, but this is straightforward and is worth the benefits.
