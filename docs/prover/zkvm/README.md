# zkVM Ecosystem

This section covers the zero-knowledge virtual machine (zkVM) ecosystem as it relates to ethrex's proving infrastructure.

## What is a zkVM?

A zero-knowledge virtual machine (zkVM) proves correct execution of programs without revealing inputs. You write code in Rust or C++, compile it to RISC-V (or a custom ISA), and generate a cryptographic proof that the execution was correct.

For ethrex, this means we can prove that a batch of Ethereum blocks was executed correctly, enabling trustless verification on L1 without re-executing the transactions.

## Key Terminology

| Term | Definition |
|------|------------|
| **Guest program** | Code that runs inside the zkVM and gets proven. In ethrex, this is the block execution logic. |
| **Host** | Code that runs outside the zkVM, prepares inputs, and verifies proofs. |
| **ELF** | Compiled guest program (RISC-V executable). |
| **Proof** | Cryptographic evidence of correct execution. |
| **Receipt** | Proof + public outputs (RISC Zero terminology). |
| **Precompile** | Accelerated operation built into the zkVM (e.g., SHA-256, Keccak, secp256k1). |
| **Patch** | Modified crate that redirects crypto operations to zkVM precompiles instead of native code. |
| **Cycles** | Unit of computation in a zkVM. More cycles = longer proving time. |

## Why zkVMs for Ethereum?

Traditional optimistic rollups require a challenge period (typically 7 days) because fraud proofs must be generated on-demand. zkVMs enable **validity proofs**: cryptographic evidence that execution was correct, verified immediately on L1.

Benefits:
- **Instant finality**: No challenge period required
- **Lower trust assumptions**: Math, not economic incentives
- **Smaller on-chain footprint**: Only verify the proof, not re-execute

## ethrex's Multi-Backend Approach

ethrex supports multiple zkVM backends, each with different trade-offs:

| Backend | Organization | Status | Primary Use Case |
|---------|--------------|--------|------------------|
| [SP1](./backends.md#sp1) | Succinct | Production | General proving |
| [RISC0](./backends.md#risc0) | RISC Zero | Production | On-chain verification |
| [ZisK](./backends.md#zisk) | Polygon | Active Development | Performance optimization |
| [OpenVM](./backends.md#openvm) | Axiom | Experimental | Modular extensions |

See [Backend Comparison](./backends.md) for detailed analysis.

## Performance Considerations

zkVM proving is computationally intensive. Key factors affecting performance:

1. **Crypto operations**: Hash functions, signature verification, and pairing operations dominate cycle counts
2. **Patch coverage**: Using patched crates with precompile acceleration is critical
3. **Memory usage**: Large state witnesses increase proving time
4. **Parallelization**: Some backends support GPU acceleration

See [Optimization Status](./optimization-status.md) for ethrex's current optimization state.

## Further Reading

- [Backend Comparison](./backends.md) — Detailed comparison of SP1, RISC0, ZisK, and OpenVM
- [Patches & Precompiles](./patches.md) — How crypto acceleration works in zkVMs
- [Optimization Status](./optimization-status.md) — Current ethrex optimization analysis
- [Guest Program](../guest_program.md) — ethrex guest program architecture
- [Prover Overview](../prover.md) — High-level prover design
