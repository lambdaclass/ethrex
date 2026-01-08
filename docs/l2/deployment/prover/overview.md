# Run an ethrex prover

Deploying the ethrex L2 contracts on L1 and starting the node isn't everything when it comes to setting up your full ethrex L2 stack.

If you've been following the deployment guide, you should already have an ethrex L2 node running and connected to L1. If that's not the case, I recommend reviewing that guide before proceeding.

The next step is to run the prover—the component responsible for generating ZK proofs for the L2 blocks. These proofs will then be sent to L1 for verification, finalizing the state of your L2.

In this section, we'll cover how to run one or more ethrex L2 provers.

> [!NOTE]
> This section focuses solely on the step-by-step process for running an ethrex L2 prover in any of its forms. For a deeper understanding of how this works under the hood, refer to the [Fundamentals](../../fundamentals/README.md) section. To learn more about the architecture of each mode, see the [Architecture](../../architecture/README.md) section.

Before proceeding, note that this guide assumes you have ethrex installed. If you haven't installed it yet, follow one of the methods in the [Installation Guide](../../../getting-started/installation/README.md). If you're looking to build from source, don't skip this section—we'll cover that method here, as it is independent of the deployment approach you choose later.

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
    # For SP1 CPU proving (very slow, not recommended)
    cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,sp1

    # For RISC0 CPU proving (very slow, not recommended)
    cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,risc0

    # For SP1 and RISC0 CPU proving (very slow, not recommended)
    cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,sp1,risc0

    # For SP1 GPU proving
    cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,sp1,gpu

    # For RISC0 GPU proving
    cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,risc0,gpu

    # For SP1 and RISC0 GPU proving
    cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,sp1,risc0,gpu
    ```

    `cargo install` places the binary at `~/.cargo/bin/ethrex`; ensure that directory is on your `$PATH`. Add `--force` if you need to reinstall.

> [!WARNING]
> If you want your verifying keys generation to be reproducible, prepend `PROVER_REPRODUCIBLE_BUILD=true` to the above command.
>
> Example:
>
> ```shell
> PROVER_REPRODUCIBLE_BUILD=true COMPILE_CONTRACTS=true cargo install --locked --path cmd/ethrex --bin ethrex --features l2,l2-sql,sp1,risc0,gpu
> ```

> [!IMPORTANT]
> Building with both `sp1` and `risc0` features enabled only enables both backends. Settlement will require every proof you mark as required at deploy time (e.g., passing both `--sp1 true` and `--risc0 true` in `ethrex l2 deploy` requires both proofs).
