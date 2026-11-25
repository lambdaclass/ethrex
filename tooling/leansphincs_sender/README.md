# leansphincs_sender

Sends [EIP-8504](../../docs/eip-8504.md) dependency transactions: frame transactions whose
authorization is a leanSPHINCS signature that travels beside the transaction instead of inside it.

## Usage

The tool has two commands that share the accounts they work on. Both derive the same accounts from
`--seed` and `--accounts`, so they must be given the same values.

```
Usage: leansphincs_sender <COMMAND>

Commands:
  alloc  Prints the genesis `alloc` entries of the accounts, as a JSON object
  send   Sends transactions from the accounts at a fixed rate
```

1. Print the genesis entries and merge them into the `alloc` of a genesis that schedules
   Hegotá, which EIP-8504 is part of:

   ```sh
   cargo run --release --manifest-path tooling/Cargo.toml -p leansphincs_sender -- \
     alloc --accounts 4 > alloc.json
   ```

   ```json
   {
     "0x008292b820008d9d7218710fa962c6d9610dd970": {
       "balance": "0x3635c9adc5dea00000",
       "code": "0x60025f35b36004145f35…",
       "nonce": "0x0",
       "storage": {}
     }
   }
   ```

2. Start a node on that genesis, for example in dev mode:

   ```sh
   cargo run --release --bin ethrex -- --network genesis.json --dev
   ```

3. Send transactions:

   ```sh
   cargo run --release --manifest-path tooling/Cargo.toml -p leansphincs_sender -- \
     send --accounts 4 --rpc-url http://localhost:8545 --rate 2 --count 8
   ```

   ```text
   Sending from 4 accounts on chain 9 at 2 tx/s
   sent 0x9e97…aa7e from 0xbd6f…ae50 nonce 0 (8032 bytes)
   sent 0xc1ac…3d01 from 0x0082…d970 nonce 0 (8032 bytes)
   ...
   Done in 3.6s: 8 sent, 0 sent again, 4 seen included, 0 rejected
   ```

`send` options:

| Option | Default | Meaning |
|---|---|---|
| `--rpc-url` | `http://localhost:8545` | The node's JSON-RPC endpoint |
| `--accounts` | `16` | Number of accounts |
| `--seed` | `pq-el-devnet-0` | Seed the keys are derived from |
| `--rate` | `1` | Transactions per second |
| `--count` | unlimited | Submissions to make, counting the rejected ones and the transactions sent again |
| `--recipient` | `0x…dEaD` | Address the transactions call |
| `--max-fee-per-gas-gwei` | `10` | Fee per gas the transactions are willing to pay |
| `--max-priority-fee-per-gas-gwei` | `1` | Priority fee per gas |
| `--resend-after` | `120` | Seconds after which a transaction that was not included is sent again |

## How it works

- **Accounts.** The key of account `i` is `keccak256(seed || i)`, and its address is the last 20
  bytes of `keccak256(public_key)`. Each account runs validation code that approves execution and
  payment when the transaction's signature list has the entry `[0x10, public_key, "", ""]`: a
  leanSPHINCS signature of the transaction's own signature hash under the account's key.
- **Transactions.** Each one has two frames: `VERIFY` runs the account's validation code and
  `SENDER` calls the recipient. The signature list declares the dependency.
- **Submission.** The tool signs the signature hash and submits
  `0x06 || rlp([tx_payload_body, [signature]])` through `eth_sendRawTransaction`. The signature is
  7,740 bytes, so a submission is about 8 KB.
- **Pacing.** A node keeps one pending frame transaction per sender. The tool therefore sends an
  account's next transaction only after its nonce shows the previous one was included, and the
  achievable rate is at most the number of accounts per block.

## References

- [EIP-8504 in ethrex](../../docs/eip-8504.md)
- Draft EIP-8504, ethereum/EIPs#12672
- leanEthereum/leanVM, the `lean_sphincs` crate the tool signs with
