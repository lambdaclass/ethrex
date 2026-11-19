# daisugi-devnet-0 fixtures

Vectors for the EIP-8288 verifier, stored as hex text.

| File | What it is |
|---|---|
| `block-673-stark-proof.hex` | The 253,584-byte proof in the body of daisugi-devnet-0's block 673, read from the network's public RPC (`starkProof` of `eth_getBlockByNumber` with full transactions). Its keccak is the header's `starkProofHash` `0x455907bb…817e`, and it proves that block's `blockDepsHash` `0xe79b80cd…3f72` under the network's `AGGREGATED_VK`. |
| `lean-vectors/` | Test vectors that the network's pinned native proving package ships for its own tests (version 0.21.0.110, package sha256 `daecd150e646cdbc1ecaffb0fa118dff780c8a928a2ff705a7c3362fddd51ac2`, built from NethermindEth/lean-bindings `8d54d3165d`). |

`lean-vectors/`:

| File | What it is |
|---|---|
| `aggregated-key.hex` | The package's `AGGREGATED_VK`, `e09e5acf…cb81` |
| `sphincs-key.hex`, `sphincs-message.hex`, `sphincs-signature.hex` | A leanSPHINCS dependency: the 32-byte public key `pkSeed \|\| pkRoot`, the signed message, and the 6,208-byte witness (the key, then the 6,176-byte signature) |
| `stark-key.hex`, `stark-message.hex`, `stark-proof.hex` | A leanSTARK dependency: the program's verification key, the proven message, and the 167,785-byte proof |

The tests that read them are in `test/tests/common/leanvm_aggregator_tests.rs` and
run with `cargo test --release -p ethrex-test --features leanvm`.
