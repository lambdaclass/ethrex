# ethrex-prover ZisK performance

## Latest benchmarks against ZisK's rsp
- ethrex commit: `-`
- ZisK's rsp: taken from ethproofs

| Block     | Gas Used     | ZisK's rsp | ethrex    |
| --------- | ------------ | ---------- | --------- |
| 23919400  | 41,075,722   |  3m 00s    |  3m 36s   |
| 23919500  | 40,237,085   |  3m 27s    |  4m 07s   |
| 23919600  | 24,064,259   |  2m 19s    |  2m 49s   |
| 23919700  | 20,862,238   |  2m 04s    |  2m 27s   |
| 23919800  | 31,813,109   |  2m 47s    |  3m 18s   |
| 23919900  | 22,917,739   |  1m 57s    |  2m 17s   |
| 23920000  | 37,256,487   |  2m 58s    |  3m 32s   |
| 23920100  | 33,542,307   |  2m 42s    |  3m 16s   |
| 23920200  | 22,994,047   |  1m 55s    |  2m 21s   |
| 23920300  | 53,950,967   |  4m 24s    |  5m 17s   |

**Benchmark server hardware:**

Both tested on RTX 4090.

**How to reproduce for ethrex:**

TODO

## Latest improvements

- [#]
- [#5535](https://github.com/lambdaclass/ethrex/pull/5535): implement ecadd with substrate for ZisK and SP1 (draft)
- [#5529](https://github.com/lambdaclass/ethrex/pull/5529): use ZisK patched substrate-bn for bn254 G1 mul (merged)
- [#5515](https://github.com/lambdaclass/ethrex/pull/5515): add optimal params for release profile of ZisK guest (closed, worsened perf.)
- [#5514](https://github.com/lambdaclass/ethrex/pull/5514): optimize from_bytecode by using a bitmap (draft, too small of an improvement)
- [#5491](https://github.com/lambdaclass/ethrex/pull/5491): use ZisK mulmod syscall for levm op_mulmod (merged)
- [#5484](https://github.com/lambdaclass/ethrex/pull/5484): add kzg-rs patch to ZisK guest (merged)

## How to get ZisK execution and proving statistics for some block execution

1. Clone **ethrex-replay** on the `main` branch.
2. Modify `Cargo.toml` to point to the branch or commit of the **ethrex** dependencies you want to benchmark.
   By default, it uses `main`.
3. Build **replay** with the `zisk` and `gpu` features:
   `cargo b -r -F zisk,gpu`
4. This will check out **ethrex** into Cargo’s cache, under:
   `~/.cargo/git/checkouts/ethrex-<something>/<commit>/`
5. It will also compile the Zisk guest program, which you can find (relative to the cargo git ethrex checkout path) at:
   `crates/l2/prover/src/guest_program/src/zisk/out/riscv64ima-zisk-zkvm-elf`
6. Copy that binary to your current directory.
7. Generate inputs for the blocks you want to benchmark. For this, **ethrex-replay** provides the `generate-input` command:
   `cargo r -r -F zisk,gpu -- generate-input --rpc-url <rpc> --blocks <blocknum1>,<blocknum2>,<etc>`
9. The RPC URL can be any node exposing the `debug_executionWitness` RPC endpoint, for example, an ethrex or reth node synced to mainnet, specifying port 8545
10. The command will generate files named:
    `ethrex_mainnet_<blocknum>_input.bin`
    inside the `generated_inputs/` directory.
11. With both the input and the ELF (and since building the ELF sets up the ROM, which you usually don’t need to do manually, Zisk will warn you if ROM is missing) run **ziskemu** with specific args to obtain statistics:
    `ziskemu -e riscv64ima-zisk-elf -i generated_inputs/ethrex_mainnet_<blocknum>_input.bin -X -D -S -T <num_f>`
12. The statistics will show a “top most expensive functions” report during execution.
    With `-T <num_f>` you choose how many entries to display (top 20 is a good number).
13. Then you can generate a proof (and benchmark proving time) with:
    `cargo-zisk prove -e riscv64ima-zisk-elf --agregation --unlocked-mapped-memory -i generated_inputs/ethrex_mainnet_<blocknum>_input.bin`
14. Or more easily, using **replay** directly, so you don't need to manually generate inputs:
    `cargo r -r -F zisk,gpu -- blocks --zkvm zisk --action prove --rpc-url <rpc> <blocknum1>,<blocknum2>,<etc>`
