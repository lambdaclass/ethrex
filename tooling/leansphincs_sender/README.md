# leansphincs_sender

Sends frame transactions that a leanSPHINCS key authorizes through a
[declarative claim](../../docs/eip-8141-declarative-claims.md): the transaction declares that the
key signed it, and the signature travels beside the transaction instead of inside it.

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
   Hegotá, which claims are part of:

   ```sh
   cargo run --release --manifest-path tooling/Cargo.toml -p leansphincs_sender -- \
     alloc --accounts 4 > alloc.json
   ```

   ```json
   {
     "0xbd6fea88015ad67cc46f39e2bbeb693df935ae50": {
       "balance": "0x3635c9adc5dea00000",
       "code": "0x5f7feb7cfb04946e4e36765467a24939f9ff4cc1d3a28d47d6f24e7df2dd989adfc37300000000000000000000000000000000000000adb4603e575f5ffd5b60035f5faa",
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
   sent 0xd58ef90d46e1e71179e40bfbf2ab8b0723153fd824f407cbd54049d435cda661 from 0xbd6fea88015ad67cc46f39e2bbeb693df935ae50 nonce 0 (7918 bytes)
   sent 0x97fdc1b6768ee4ff6258de1c2815fb125642f2ca91be7bf8fbbd44544ce1e75a from 0x008292b820008d9d7218710fa962c6d9610dd970 nonce 0 (7918 bytes)
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
  bytes of `keccak256(public_key)`. Each account runs 68 bytes of validation code that approves
  execution and payment when `HASCLAIM(LEANSPHINCS_VERIFIER, public_key, 0)` is 1: the
  transaction has a claim that the account's key signed the transaction's own signature hash.
- **Transactions.** Each one has one claim, `[LEANSPHINCS_VERIFIER, public_key, "", 0, 0]`, and
  two frames: `VERIFY` runs the account's validation code and `SENDER` calls the recipient.
- **Submission.** The tool signs the signature hash and submits the pooled representation,
  `0x06 || rlp([tx_body, 0, [signature], []])`, through `eth_sendRawTransaction`. The signature
  is 7,740 bytes, so a submission is about 8 KB.
- **Pacing.** A node keeps one pending frame transaction per sender. The tool therefore sends an
  account's next transaction only after its nonce shows the previous one was included, and the
  achievable rate is at most the number of accounts per block.

## References

- [Declarative claims in ethrex](../../docs/eip-8141-declarative-claims.md)
- The declarative-claims proposal for EIP-8141
- leanEthereum/leanVM, the `lean_sphincs` crate the tool signs with
