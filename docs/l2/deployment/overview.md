# Deploying an ethrex L2

As outlined in the introduction, ethrex L2 offers a wide range of features to its users. The most common is a classic centralized L2 managed by an operator, which can be an individual or a DAO. ethrex L2 can also function as a Validium, which is similarly centralized and operator-managed, with the key difference that network data is not posted to L1 during settlement.

In addition to these classic functionalities, ethrex L2 provides a novel and continually evolving feature in the industry: ethrex L2 as a based rollup. Unlike the previous options, this is a permissionless and decentralized L2 sequencer—anyone can run a node and participate in the network.

In this section, we will cover how to deploy any of these options.

> [!NOTE]
> This section focuses solely on the step-by-step process for deploying ethrex L2 in any of its forms. For a deeper understanding of how each mode works under the hood, refer to the [Fundamentals](../fundamentals/README.md) section. To learn more about the architecture of each mode, see the [Architecture](../architecture/README.md) section.

Before proceeding, note that this guide assumes you have ethrex installed. If you haven't installed it yet, follow one of the methods in the [Installation Guide](../../getting-started/installation/README.md). If you're looking to build from source, don't skip this section—we'll cover that method here, as it is independent of the deployment approach you choose later.

## Building from source (skip if ethrex is already installed)

### Prerequisites

Ensure you have the following installed on your system:

- Rust and Cargo (install via [rustup](https://rustup.rs/))
- Solidity compiler v0.8.31 (refer to [Solidity documentation](https://docs.soliditylang.org/en/latest/installing-solidity.html))
- SP1 Toolchain (if you plan to use SP1 proving, refer to [SP1 documentation](https://docs.succinct.xyz/docs/sp1/getting-started/install))
- RISC0 Toolchain (if you plan to use RISC0 proving, refer to [RISC0 documentation](https://dev.risczero.com/api/zkvm/install))
- CUDA Toolkit 12.9 (if you plan to use GPU acceleration for SP1 or RISC0 proving)

1. Clone the official ethrex repository:

    ```shell
    git clone https://github.com/lambdaclass/ethrex
    cd ethrex
    ```

2. Install the binary to your `$PATH`:

    ```shell
    # For dummy proving
    COMPILE_CONTRACTS=true cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql

    # For SP1 CPU proving (very slow, not recommended)
    COMPILE_CONTRACTS=true cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,sp1

    # For RISC0 CPU proving (very slow, not recommended)
    COMPILE_CONTRACTS=true cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,risc0

    # For SP1 and RISC0 CPU proving (very slow, not recommended)
    COMPILE_CONTRACTS=true cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,sp1,risc0

    # For SP1 GPU proving
    COMPILE_CONTRACTS=true cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,sp1,gpu

    # For RISC0 GPU proving
    COMPILE_CONTRACTS=true cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,risc0,gpu

    # For SP1 and RISC0 GPU proving
    COMPILE_CONTRACTS=true cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,sp1,risc0,gpu
    ```

    By default `cargo install` places the binary at `~/.cargo/bin/ethrex` (make sure that directory is on your `$PATH`). Add `--force` to the commands above if you need to overwrite a previous installation.

> [!WARNING]
> If you want your verifying keys generation to be reproducible, prepend `PROVER_REPRODUCIBLE_BUILD=true` to the above command:
>
> ```shell
> PROVER_REPRODUCIBLE_BUILD=true COMPILE_CONTRACTS=true cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,sp1,risc0,gpu
> ```

> [!IMPORTANT]
> Compiling with both `sp1` and `risc0` features only makes the binary capable of both. Settlement requires every proof you mark as required at deploy time (e.g., passing both `--sp1 true` and `--risc0 true` in `ethrex l2 deploy` will require both proofs).
