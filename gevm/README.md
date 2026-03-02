# GEVM

A behavior-identical Go implementation of the Ethereum Virtual Machine.

GEVM is a from-scratch port of the Rust reference EVM, preserving identical semantics: gas accounting, memory expansion, call/create frames, journal reverts, precompile behavior, and instruction decoding.

## Status

- 44,035 / 44,035 EEST GeneralStateTests passing (100%)
- 882 / 882 BlockchainTests passing (100%)
- 260 / 260 InvalidBlocks passing (100%)
- 2,753 / 2,753 TransactionTests passing (100%)
- Osaka-ready: EIP-7823, EIP-7825, EIP-7883, EIP-7939, EIP-7951

## Usage

```go
import (
    "github.com/Giulio2002/gevm/host"
    "github.com/Giulio2002/gevm/spec"
    "github.com/Giulio2002/gevm/state"
    "github.com/Giulio2002/gevm/types"
)

evm := host.NewEvm(db, spec.Osaka, blockEnv, cfgEnv)
defer evm.ReleaseEvm()

result := evm.Transact(&host.Transaction{
    Kind:     host.TxKindCall,
    Caller:   caller,
    To:       target,
    GasLimit: 1_000_000,
    Input:    calldata,
})
fmt.Printf("gas used: %d\n", result.GasUsed)
```

## Benchmarks

### evm-bench

[evm-bench](https://github.com/pashakondratyev/evm-bench) comparison on Apple M4 (median of stable runs, ms, lower is better):

```
Benchmark                GEVM       geth       revm       vs geth
Snailtracer              31.5       62.9       ---(*)     2.0x
ERC20 Transfer            5.2       11.0       4.8        2.1x
ERC20 Mint                3.1        7.0       2.8        2.3x
ERC20 Approval+Transfer   3.7        8.7       3.9        2.4x
TenThousandHashes         2.3        3.8       1.7        1.7x
```

(*) revm snailtracer exhibits state accumulation across runs, making times unreliable.

GEVM is ~2x faster than geth and within 10% of revm (Rust) on ERC20 workloads.

### Go Benchmarks

`go test ./tests/bench/ -bench=. -benchtime=3s` on Apple M4:

```
Benchmark                          Time          Allocs
Snailtracer                        29.9 ms       1
SimpleLoop/loop-100M               53.6 ms       8
SimpleLoop/call-identity-100M      108  ms       8
ERC20 Transfer                     1.85 us       3
Analysis                           2.28 us       1
TenThousandHashes                  526  ns       0
Transfer                           325  ns       0
CREATE 500                         2.51 ms       757
RETURN/1M                          67.8 us       2
```

### Precompiles

```
Precompile          Time          Allocs
ECRECOVER           15.6 us       4
SHA256              87.6 ns       1
RIPEMD160           456  ns       2
IDENTITY/128B       15.8 ns       1
MODEXP              404  ns       11
BN254 Add           1.08 us       1
BN254 Mul           2.15 us       3
BN254 Pairing       327  us       7
BLAKE2F             123  ns       1
BLS G1Add           2.19 us       1
BLS G1Msm           88.3 us       139
BLS G2Add           3.11 us       1
BLS G2Msm           181  us       139
BLS Pairing         439  us       26
P256VERIFY          36.3 us       28
```

## Testing

```bash
# Unit tests
go test ./...

# EEST spec tests (downloads fixtures automatically)
make test-spec

# Blockchain tests
GEVM_BLOCKCHAIN_TESTS_DIR=./tests/fixtures/ethereum-tests/BlockchainTests \
  go test ./tests/spec/ -run=TestBlockchainFixtureDir -timeout=600s

# Differential tests
go test ./tests/differential/
```
