# ziskemu AIR-cost output format (v0.16.1)

This documents the confirmed ziskemu invocation and output format the
benchmark parser targets. Established by the Task 6 spike on a real Hoodi
block (`fixtures/cache/rpc_prover/cache_hoodi_1265656.json`), ZisK v0.16.1.

## Invocation

```
ziskemu -e <guest.elf> -i <zisk_stdin_input.bin> -X
```

- `-e/--elf`: the zisk guest ELF (`riscv64ima-zisk-zkvm-elf`).
- `-i/--inputs`: the input file in ZisK stdin framing
  (`[8-byte LE len][rkyv ProgramInput][zero-pad to 8]`, produced by
  `ZiskBackend::serialize_input`).
- `-X/--stats`: **the flag that emits the AIR-cost breakdown** ("statistics
  about opcodes and memory usage"). This is the deterministic cost signal.

Do NOT use `-x/--legacy-stats`: it prints a different, less complete block
(`STEPS: 0`, most component costs zero) and is not the AIR-cost distribution.

No proving key is required — `ziskemu` emulates. The guest ELF is built with
`cargo build -p ethrex-guest-program --features zisk,zisk-build-elf,ci`; the
`ci` feature skips `cargo-zisk rom-setup` (a proving-only step that otherwise
needs the proving key). See `crates/guest-program/build.rs` (rom-setup is
`#[cfg(not(feature = "ci"))]`).

## Output format (the parser target)

`-X` prints, in order: progress lines (`start/finish reading input`,
`start/finish execution`, `start/finish revealing output`), then a `REPORT`
block, a `COST DISTRIBUTION` block, and detailed per-opcode tables.

The parser only needs two blocks:

```
REPORT
----------------------------------------
STEPS                         40,007,528

COST DISTRIBUTION                   COST       %
------------------------------------------------
BASE                         293,601,280   5.96%
MAIN                       2,720,511,904  55.18%
OPCODES                      482,648,015   9.79%
PRECOMPILES                  937,548,926  19.02%
MEMORY                       495,887,679  10.06%

TOTAL                      4,930,197,804 100.00%
```

Extraction rules:
- `STEPS`: the integer on the `STEPS` line (REPORT block).
- Cost components under `COST DISTRIBUTION`, one per line, format
  `<LABEL><spaces><COST with comma thousands-separators><spaces><pct>%`:
  `BASE`, `MAIN`, `OPCODES`, `PRECOMPILES`, `MEMORY`.
- `TOTAL`: the `TOTAL` line's cost. It equals BASE+MAIN+OPCODES+PRECOMPILES+MEMORY.
- All numbers use `,` thousands separators — strip commas before parsing.
- `FROPS` and `RAM USAGE` lines follow TOTAL; the parser ignores them.

The full captured sample is committed at `fixtures/ziskemu_sample.txt` and is
the unit-test fixture for the parser (Task 7).

## Determinism

For a fixed guest ELF + fixed input, the cost distribution is identical across
runs (no wall-clock, no randomness). Cost numbers are specific to the ZisK
toolchain/ELF version, so the guest ELF sha256 is recorded in every report's
`meta`.
