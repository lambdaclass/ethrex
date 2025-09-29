## LEVM Runner

You can send a transaction directly to the EVM specifying things like `fork`, `bytecode`, `pre-state` and setting an initial `stack` and `memory` for testing/benchmarking purposes. After execution it will print the final stack and memory, state diff and execution result.

Example Run: `cargo run -- --input input_example.json --code mnemonics_example.txt`

The main runner typically expects 2 inputs:
- One JSON with fields like the Transaction, Fork, etc. These are all specified in `input_example.json`, you can copy that for you input and make changes.
- Bytecode, either raw or mnemonics, the latter separated by spaces or newlines. Examples in `code_example.txt` and `mnemonics_example.txt`

You can provide input, bytecode or both. If input is not provided default values will be used. If bytecode file is sent as argument and the transaction is a `CALL` then it will be the bytecode of the contract being called; but if the transaction is `CREATE` then it will be the initcode.
`CREATE` transactions can be sent if the `to` field of the json is `null`, remember that the initcode has to contain a return instruction with the bytecode at the end. Example of this in `initcode_example.txt`.

You can also use the subcommand `--emit-bytes` to convert a mnemonic `.txt` file into a bytecode file without executing it. This is useful for profiling the EVM with tools like `flamegraph` or `samply` , as it avoids parsing the mnemonics during the profiling run — which can introduce noise.

Additional Notes:
- In mnemonics file, numbers in `PUSH` opcodes can be written both in hex and decimal. Hex values must have `0x` as a prefix. Also, numbers will be automatically padded, so you can do for example `PUSH3 0x1f` and it will be equivalent to `PUSH3 0x00001f`. You can't push a value greater than the number of bytes in the PUSH, for example, `PUSH2 0x10000` or `PUSH1 256` will panic.

- Input Stack is represented from bottom to top. So for [1,2,3] 1 will be the element at the bottom and 3 will be the top. This is the most intuitive way of implementing a stack using a vec, that's why it's done this way.
In LEVM our stack actually grows downwards because it has fixed size but for a json this wasn't the nicest approach I believe.

- The input is restricted to few things that are crucial for a transaction, more things could be added for customization but didn't want to add noise to the code nor the input file.

- The input file can contain partial values, for example, you don't need to specify all values for the Transaction field, you can just specify those you want and for the rest default values will be used. These try to be coherent generic values but feel free to check them out in the code.

- If not specified in the transaction, default **sender** will be `0x000000000000000000000000000000000000dead`, whereas default **recipient** will be `0x000000000000000000000000000000000000beef`. Default **coinbase** is `0x7777777777777777777777777777777777777777`.
