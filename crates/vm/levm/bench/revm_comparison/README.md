# Benchmarks
This README explains how to run benchmarks to compare the performance of `levm` and `revm` when running different contracts. The benchmarking tool used to gather performance metrics is [hyperfine](https://github.com/sharkdp/hyperfine), and the obtained results will be included for reference.

To run the benchmarks (from `levm`'s root):

```bash
make revm-comparison
```

Note that first you will need to install the solidity compiler
On mac you can use homebrew:

```bash
brew install solidity
```

For other installation methods check out the [official solidity installation guide](https://docs.soliditylang.org/en/latest/installing-solidity.html)


Additional Notes:
- As it is done now, contracts should have the `Benchmark` public function that expects a `uint256`.
