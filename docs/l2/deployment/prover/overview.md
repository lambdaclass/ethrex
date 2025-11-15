# Run an ethrex prover

Deployar los contratos de ethrex L2 en la L1 y levantar el nodo no lo es todo a la hora de levantar tu ethrex L2 stack.

Si venis siguiendo la guia de deployment, ya tendrias que tener un nodo ethrex L2 corriendo y conectado a la L1. Si este no es el caso, te recomiendo volver a esa guia antes de continuar.

El siguiente paso es correr el prover, que es el componente encargado de generar las pruebas ZK para los bloques de la L2. Pruebas que luego seran enviadas a la L1 para su verificacion y asi dar por finalizado el estado de tu L2.

En esta seccion, vamos a cubrir como correr uno o varios provers de ethrex L2.

> [!NOTE]
> This section focuses solely on the step-by-step process for running an ethrex L2 prover in any of its forms. For a deeper understanding of this works under the hood, refer to the Fundamentals section. To learn more about the architecture of each mode, see the Architecture section.

Before proceeding, note that this guide assumes you have ethrex installed. If you haven't installed it yet, follow one of the methods in the [Installation Guide](../../getting-started/installation/README.md). If you're looking to build from source, don't skip this section—we'll cover that method here, as it is independent of the deployment approach you choose later.

## Building from source (skip if ethrex is already installed)

### Prerequisites

Ensure you have the following installed on your system:

- Rust and Cargo (install via [rustup](https://rustup.rs/))
- Solidity compiler (refer to [Solidity documentation](https://docs.soliditylang.org/en/latest/installing-solidity.html))
- SP1 Toolchain (if you plan to use SP1 proving, refer to [SP1 documentation](https://docs.succinct.xyz/docs/sp1/getting-started/install))
- RISC0 Toolchain (if you plan to use RISC0 proving, refer to [RISC0 documentation](https://dev.risczero.com/api/zkvm/install))
- CUDA Toolkit 12.9 (if you plan to use GPU acceleration for SP1 or RISC0 proving)

1. Clone the official ethrex repository:

    ```shell
    git clone https://github.com/lambdaclass/ethrex
    cd ethrex
    ```

2. Build the binary:

    ```shell
    # For SP1 CPU proving (very slow, not recommended)
    cargo build --release --bin ethrex --features l2,l2-sql,sp1

    # For RISC0 CPU proving (very slow, not recommended)
    cargo build --release --bin ethrex --features l2,l2-sql,risc0

    # For SP1 and RISC0 CPU proving (very slow, not recommended)
    cargo build --release --bin ethrex --features l2,l2-sql,sp1,risc0

    # For SP1 GPU proving
    cargo build --release --bin ethrex --features l2,l2-sql,sp1,gpu

    # For RISC0 GPU proving
    cargo build --release --bin ethrex --features l2,l2-sql,risc0,gpu

    # For SP1 and RISC0 GPU proving
    cargo build --release --bin ethrex --features l2,l2-sql,sp1,risc0,gpu
    ```

> [!WARNING]
> If you want your verifying keys generation to be reproducible, prepend `PROVER_REPRODUCIBLE_BUILD=true` to the above command.
>
> Example:
>
> ```shell
> PROVER_REPRODUCIBLE_BUILD=true COMPILE_CONTRACTS=true cargo b -r --bin ethrex -F l2,l2-sql,sp1,risc0,gpu
> ```
