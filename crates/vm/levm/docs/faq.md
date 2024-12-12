# FAQ
## `usize` and `U256`
In Rust, **accessing an index on a specific data structure requires a `usize` type variable.** This can be seen in methods like `get` and `get_mut`.

<!-- TODO: Link in the documentation where the `U256` addresses are described -->
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
7:               .map_err(|_| InternalError::ConversionError)?;
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
