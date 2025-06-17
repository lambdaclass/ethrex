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

## Factorial
This program computes the nth factorial number, with n passed via calldata. We chose 1000 as n and ran the program on a loop 100,000 times.

These are the obtained results:

### MacBook Air M1 (16 GB RAM)
|        |    Mean [s]   | Min [s] | Max [s] |  Relative   |
|--------|---------------|---------|---------|-------------|
| `revm` | 6.719 ± 0.047 |  6.677  |  6.843  |    1.00     |
| `levm` | 8.283 ± 0.031 |  8.244  |  8.349  | 1.23 ± 0.01 |

## Fibonacci
This program computed the nth Fibonacci number, with n passed via calldata. Again, we chose 1000 as n and ran the program on a loop 100,000 times.

These are the obtained results:

### MacBook Air M1 (16 GB RAM)
|        |    Mean [s]   | Min [s] | Max [s] |  Relative   |
|--------|---------------|---------|---------|-------------|
| `revm` | 6.213 ± 0.029 |  6.169  |  6.253  |    1.00     |
| `levm` | 8.303 ± 0.094 |  8.204  |  8.498  | 1.33 ± 0.02 |
