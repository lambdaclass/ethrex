# ziskemu AIR-cost output format (v1.0.0-alpha)

This documents the confirmed ziskemu invocation and output format the
benchmark parser targets. `ziskemu` is still shipped by the v1.0.0-alpha
toolchain (`emulator/src/bin/ziskemu.rs`) and remains the only tool that
emits the AIR-cost breakdown — `cargo-zisk execute` does not. The exact
byte-level capture lives in `fixtures/ziskemu_sample.txt` (the
`mainnet_25087668_light` benchmark block, ZisK v1.0.0-alpha); the same block
is quoted below.

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
`ci` feature skips `cargo-zisk setup` (a proving-only step that otherwise
needs the proving key). See `crates/guest-program/build.rs` (setup is
`#[cfg(not(feature = "ci"))]`).

## Output format (the parser target)

`-X` prints, in order: progress lines (`start/finish reading input`,
`start/finish execution`, `start/finish revealing output`), then a `REPORT`
block, a `COST DISTRIBUTION` block, and detailed per-opcode tables.

The parser only needs two blocks:

```
REPORT
----------------------------------------
STEPS                          8,573,814

COST DISTRIBUTION                   COST       %
------------------------------------------------
MAIN                         583,019,352  44.53%
OPCODES                      124,468,967   9.51%
PRECOMPILES                  227,527,215  17.38%
MEMORY                        80,638,696   6.16%
                        ------------------------
VARIABLE                   1,015,654,230  77.57%
BASE                         293,601,280  22.43%
                        ------------------------
TOTAL                      1,309,255,510 100.00%

FROPS                        103,611,330  10.20%
RAM USAGE                      3,697,848   0.69%
```

### What changed from v0.16.1

- A **`VARIABLE`** line (= `TOTAL - BASE`) was added, and the component order
  changed: v1.0.0-alpha lists `MAIN`, `OPCODES`, `PRECOMPILES`, `MEMORY`, then
  `VARIABLE`/`BASE`, then `TOTAL` (v0.16.1 led with `BASE` and had no
  `VARIABLE` line). Short dashed separators (`add_separator_from(24)`) sit
  around the `VARIABLE`/`BASE` group and before `TOTAL`.
- The `FROPS` percentage is now relative to the *variable* cost (`TOTAL - BASE`),
  not `TOTAL` — the emulator resets its cost divisor before printing it.

The parser is unaffected: it matches on the first whitespace token of each
line and is order-independent, so the added `VARIABLE` line and the
separators are ignored, and every label it needs is still present.

Extraction rules:
- `STEPS`: the integer on the `STEPS` line (REPORT block).
- Cost components under `COST DISTRIBUTION`, one per line, format
  `<LABEL><spaces><COST with comma thousands-separators><spaces><pct>%`:
  `BASE`, `MAIN`, `OPCODES`, `PRECOMPILES`, `MEMORY`.
- `TOTAL`: the `TOTAL` line's cost. It still equals
  BASE+MAIN+OPCODES+PRECOMPILES+MEMORY (`total_cost = base_cost + mem_cost +
  main_cost + ops_cost + precompiled_cost`).
- `VARIABLE` and `FROPS` are intentionally not extracted (their first token
  matches no tracked label).
- All numbers use `,` thousands separators — strip commas before parsing.
- `RAM USAGE` follows TOTAL; its first token `RAM` maps to `ram_usage` (the
  second token `USAGE` fails to parse and is skipped).

The full captured sample is committed at `fixtures/ziskemu_sample.txt` and is
the unit-test fixture for the parser.

## Determinism

For a fixed guest ELF + fixed input, the cost distribution is identical across
runs (no wall-clock, no randomness). Cost numbers are specific to the ZisK
toolchain/ELF version, so the guest ELF sha256 is recorded in every report's
`meta`.
