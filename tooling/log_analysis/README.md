# Log Analysis Notebook

This tool is a Jupyter notebook to perform bottleneck analysis based on logs from a block import.

## Instructions

0. Make sure you have the [uv](https://docs.astral.sh/uv/) tool installed. We use it to manage dependencies.
1. Run an import with the execution client, saving the output to a file:
```shell
cargo run --release --bin ethrex -- --network ./cmd/ethrex/networks/hoodi/genesis.json import ./hoodi-100k.rlp > import-100k.log
```
2. Move the file to the `tooling/log_analysis` directory:
```shell
mv import-100k.log tooling/log_analysis/import-100k.log
```
3. Start the notebook:
```shell
make notebook
```
4. Go to the `kernel` menu and select `Restart Kernel and Run All Cells`;
5. Go to the bottom of the page, where you'll see the graphs showing participation of each step of the block import process per block.
