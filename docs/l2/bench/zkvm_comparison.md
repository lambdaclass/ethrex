# zkVM Comparison

This page provides benchmark comparisons between ethrex and other implementations, along with a feature matrix for supported zkVM backends.

## ethrex vs RSP (SP1)

RSP (RISC Zero/SP1 Prover) is Succinct's implementation for proving Ethereum blocks. Here's how ethrex compares on mainnet blocks:

| Block | Gas Used | ethrex (SP1) | RSP | Difference |
|-------|----------|--------------|-----|------------|
| 23769082 | 7.9M | 2m 23s | 1m 27s | +63% |
| 23769083 | 44.9M | 12m 24s | 7m 49s | +59% |
| 23769084 | 27.1M | 8m 40s | Failed | — |
| 23769085 | 22.2M | 6m 40s | Failed | — |
| 23769086 | 28.4M | 7m 36s | 4m 45s | +60% |
| 23769087 | 17.5M | 6m 02s | Failed | — |
| 23769088 | 17.5M | 4m 50s | 2m 59s | +61% |
| 23769089 | 23.9M | 8m 14s | 4m 44s | +74% |
| 23769090 | 24.2M | 8m 11s | 4m 40s | +75% |
| 23769091 | 24.4M | 7m 02s | Failed | — |
| 23769092 | 21.7M | 6m 35s | 4m 01s | +64% |

> [!NOTE]
> RSP failed on several blocks with `block gas used mismatch` errors. ethrex successfully proved all blocks.

**Hardware:**
- ethrex: AMD EPYC 7713 64-Core, 128GB RAM, RTX 4090
- RSP: AMD EPYC 7F72 24-Core, 64GB RAM, RTX 4090

## zkVM Backend Comparison

ethrex supports multiple zkVM backends with varying features and maturity levels:

| Feature | SP1 | RISC Zero | ZisK | OpenVM |
|---------|-----|-----------|------|--------|
| **Status** | Production | Production | Experimental | Experimental |
| **GPU Acceleration** | ✓ | ✓ | ✓ | ✓ |
| **L2 Prover** | ✓ | ✓ | Planned | Planned |
| **Proof Aggregation** | ✓ | ✓ | ✓ | ✓ |

### Precompile Support

ZK proving of Ethereum precompiles varies by backend:

| Precompile | SP1 | RISC Zero | ZisK |
|------------|-----|-----------|------|
| ecrecover | ✓ | ✓ | ✓ |
| SHA256 | ✓ | ✓ | ✓ |
| RIPEMD160 | ✓ | ✓ | ✓ |
| identity | ✓ | ✓ | ✓ |
| modexp | ✓ | ✓ | ✓ |
| ecAdd | ✓ | ✓ | ✓ |
| ecMul | ✓ | ✓ | ✓ |
| ecPairing | ✓ | ✓ | ✓ |
| blake2f | ✓ | ✓ | ✓ |
| KZG point evaluation | ✓ | ⚠️ | ⚠️ |
| BLS12-381 | ✓ | ⚠️ | ✓ |
| P256 verify | ✓ | ✓ | ⚠️ |

Legend: ✓ = Supported with patches, ⚠️ = Limited or requires unstable features

## Optimization Impact

ethrex has undergone significant optimization for zkVM proving:

| Optimization | Impact | Description |
|--------------|--------|-------------|
| Jumpdest analysis | -15% cycles | Optimized jump destination validation |
| Trie caching | -50% hash calls | Cache initial node hashes during trie construction |
| Trie hashing | -75% trie cycles | Improved traversal and RLP encoding |
| Trie operations | -93% get/insert cycles | Eliminated unnecessary node cloning |
| Serialized tries | -22% total cycles | Pre-serialize resolved tries, skip decoding |
| ecPairing patch | -10% total cycles | 138k → 6k cycles per operation |
| ecMul patch | -10% total cycles | Accelerated scalar multiplication |

See [prover_performance.md](./prover_performance.md) for detailed optimization history.

## Reproduction Instructions

### ethrex Benchmarks

1. Clone [ethrex-replay](https://github.com/lambdaclass/ethrex-replay)
2. Run the prover:
```bash
cargo r -r -F "sp1,gpu" -p ethrex-replay -- blocks \
  --action prove \
  --zkvm sp1 \
  --from 23769082 \
  --to 23769092 \
  --rpc-url <RPC_WITH_DEBUG_EXECUTIONWITNESS>
```

### RSP Benchmarks

1. Clone [rsp](https://github.com/succinctlabs/rsp)
2. Run with CUDA:
```bash
SP1_PROVER=cuda cargo r -r \
  --manifest-path bin/host/Cargo.toml \
  --block-number <BLOCK> \
  --rpc-url <RPC> \
  --prove
```

## Hardware Recommendations

| Use Case | Minimum | Recommended |
|----------|---------|-------------|
| Development | 32GB RAM, 8 cores | 64GB RAM, 16 cores |
| Production (CPU) | 64GB RAM, 32 cores | 128GB RAM, 64 cores |
| Production (GPU) | 64GB RAM, RTX 3090 | 128GB RAM, RTX 4090 |

GPU proving is significantly faster and recommended for production workloads. All modern NVIDIA GPUs with 24GB+ VRAM are supported.
