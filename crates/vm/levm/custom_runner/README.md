## LEVM Custom Runner

Input Stack is represented from bottom to top. So for [1,2,3] 1 will be the element at the bottom and 3 will be the top. This is the most intuitive way of implementing a stack using a vec, that's why it's done this way.
In LEVM our stack actually grows downwards because it has fixed size but for a json this wasn't the nicest approach I believe.


The program expects 2 inputs:
- One JSON with fields like the Transaction, Fork, etc. These are all specified in `input_example.json`, you can copy that.
- Bytecode (in mnemonic whenever implemented)

By default the files being used are `input.json` and `code.txt`.

If not specified in the transaction, default sender will be `0x000000000000000000000000000000000000dead`, whereas default contract will be `0x000000000000000000000000000000000000beef`.
Default coinbase is `0x7777777777777777777777777777777777777777`.




