# zkVM Integrations

ethrex integrates with multiple zero-knowledge virtual machines (zkVMs), giving you flexibility in how you prove Ethereum execution. This page provides an overview of each integration, its status, and links to deployment documentation.

## Integration Overview

| zkVM | Organization | Status | L1 Proving | L2 Prover | Documentation |
|------|--------------|--------|------------|-----------|---------------|
| **SP1** | Succinct | Production | ✓ | ✓ | [SP1 Prover Guide](./l2/deployment/prover/sp1.md) |
| **RISC Zero** | RISC Zero | Production | ✓ | ✓ | [RISC0 Prover Guide](./l2/deployment/prover/risc0.md) |
| **ZisK** | Polygon | Experimental | ✓ | Planned | Coming soon |
| **OpenVM** | Axiom | Experimental | ✓ | Planned | Coming soon |
| **TEE (TDX)** | Intel | Production | — | ✓ | [TDX Prover Guide](./l2/deployment/prover/tee.md) |

## SP1 (Succinct)

[SP1](https://docs.succinct.xyz/) is a zkVM developed by Succinct Labs that enables efficient proving of arbitrary Rust programs.

**Status:** Production-ready for both L1 proving and L2 prover deployments.

**Key Features:**
- GPU acceleration via CUDA
- Proof aggregation support
- Extensive precompile patches for Ethereum operations
- Active development and community support

**Integration Details:**
- ethrex uses SP1's precompile patches for optimized cryptographic operations
- Supports both CPU and GPU proving modes
- Compatible with Aligned Layer for proof aggregation

**Get Started:**
- [Run an ethrex SP1 prover](./l2/deployment/prover/sp1.md)
- [SP1 Documentation](https://docs.succinct.xyz/)

## RISC Zero

[RISC Zero](https://dev.risczero.com/) is a zkVM built on the RISC-V architecture, providing a general-purpose proving environment.

**Status:** Production-ready for both L1 proving and L2 prover deployments.

**Key Features:**
- GPU acceleration support
- Bonsai proving network for distributed proving
- Strong developer tooling and documentation

**Integration Details:**
- ethrex integrates with risc0-ethereum for optimized trie operations
- Supports CUDA acceleration for faster proving
- Some precompiles (Keccak, BLS12-381) require the "unstable" feature flag

**Get Started:**
- [Run an ethrex RISC0 prover](./l2/deployment/prover/risc0.md)
- [RISC Zero Documentation](https://dev.risczero.com/)

## ZisK (Polygon)

[ZisK](https://0xpolygonhermez.github.io/zisk/) is Polygon's zkVM designed for high-performance proving with GPU acceleration.

**Status:** Experimental. L1 proving is functional; L2 integration is planned.

**Key Features:**
- Native GPU support with custom CUDA kernels
- Unique MODEXP precompile implementation
- Optimized for high-throughput proving

**Integration Details:**
- ethrex supports ZisK for L1 block proving via ethrex-replay
- Most Ethereum precompiles are supported with patches
- P256 verification is not yet available (no patch exists)

**Current Limitations:**
- L2 prover integration is not yet complete
- Requires manual installation from source for GPU support

## OpenVM (Axiom)

[OpenVM](https://book.openvm.dev/) is Axiom's modular zkVM framework designed for flexibility and extensibility.

**Status:** Experimental. Initial integration for L1 proving.

**Key Features:**
- Modular architecture for custom extensions
- Support for multiple proving backends
- Designed for composability

**Integration Details:**
- Basic integration for L1 block proving
- Precompile support is being expanded
- L2 prover integration is planned

## TEE (Intel TDX)

Intel Trust Domain Extensions (TDX) provides hardware-based trusted execution for block proving.

**Status:** Production-ready for L2 prover deployments.

**Key Features:**
- Hardware-based security guarantees
- No cryptographic proving overhead
- Fast execution within trusted enclave

**Integration Details:**
- Supported as an L2 prover option
- Can run alongside zkVM provers for redundancy
- Requires TDX-capable hardware

**Get Started:**
- [Run an ethrex TDX prover](./l2/deployment/prover/tee.md)

## Multi-Prover Deployments

ethrex supports running multiple provers simultaneously, providing redundancy and flexibility:

```
                    ┌─────────────┐
                    │   ethrex    │
                    │  Sequencer  │
                    └──────┬──────┘
                           │
           ┌───────────────┼───────────────┐
           │               │               │
      ┌────┴────┐    ┌────┴────┐    ┌────┴────┐
      │   SP1   │    │  RISC0  │    │   TDX   │
      │ Prover  │    │ Prover  │    │ Prover  │
      └────┬────┘    └────┬────┘    └────┬────┘
           │               │               │
           └───────────────┼───────────────┘
                           │
                    ┌──────┴──────┐
                    │   Aligned   │
                    │    Layer    │
                    └─────────────┘
```

See [Multi-prover deployment guide](./l2/deployment/prover/multi-prover.md) for configuration details.

## Ecosystem Integrations

### Aligned Layer

[Aligned Layer](https://alignedlayer.com/) provides proof aggregation and verification services for ethrex L2 deployments.

**Features:**
- Aggregates proofs from multiple zkVM backends
- Reduces L1 verification costs
- Supports SP1, RISC Zero, and other proof systems

**Documentation:** [ethrex <> Aligned integration](./l2/deployment/aligned.md)

## Choosing a zkVM

| Consideration | Recommendation |
|---------------|----------------|
| **Production L2** | SP1 or RISC Zero (most mature) |
| **Maximum performance** | SP1 with GPU acceleration |
| **Hardware security** | TEE (TDX) |
| **Experimentation** | ZisK or OpenVM |
| **Redundancy** | Multi-prover with SP1 + RISC Zero + TEE |

## Performance Comparison

See [zkVM Comparison](./l2/bench/zkvm_comparison.md) for detailed benchmark data comparing proving times across backends.
