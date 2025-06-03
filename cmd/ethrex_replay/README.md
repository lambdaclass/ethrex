# L1 block prover

## Usage

1. For now we only support SP1. Install their [toolchain](https://docs.succinct.xyz/docs/sp1/introduction) first (version 5.0.0).
2. Run:
   1. `make sp1 RPC_URL=<json rpc url> BLOCK_NUMBER=<number, optional>` for execution without proving
   2. `make prove-sp1 RPC_URL=<json rpc url> BLOCK_NUMBER=<number, optional>` for generating a proof.
   3. `make prove-sp1-gpu RPC_URL=<json rpc url> BLOCK_NUMBER=<number, optional>` for generating a proof with GPU acceleration.

If `BLOCK_NUMBER` is not defined then the latest block will be selected.
