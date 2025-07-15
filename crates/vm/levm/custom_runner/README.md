## LEVM Custom Runner

Stack is represented from bottom to top. So for [1,2,3] 1 will be the element at the bottom and 3 will be the top. In LEVM our stack is implemented the other way around but I had to choose one way of doing it and this is the most conventional.


The program expects 2 inputs:
- One JSON with fields like the Transaction, Fork, etc. These are all specified in `input_example.json`, you can copy that.
- Bytecode (in mnemonic whenever implemented)

By default the files being used are `input.json` and `code.txt`.

If not specified in the transaction, default sender will be `0x000000000000000000000000000000000000dead`, whereas default contract will be `0x000000000000000000000000000000000000beef`.
Default coinbase is `0x7777777777777777777777777777777777777777`.




