# zkVM RISC-V target spec (for no_std CI)

`riscv64im_zicclsm-unknown-none-elf.json` is the [eth-act zkVM standards][std]
target that Ethereum zkVMs are converging on: RV64IM + Zicclsm, soft-float LP64,
single-hart, no syscalls, `panic = abort`. The no_std CI job
(`../workflows/pr_nostd.yaml`) builds the guest crates against it to catch any
`use std::` regression.

Content is the authoritative spec from [`eth-act/ere`][ere]. Reproduce with:

```sh
rustc +nightly -Z unstable-options --print target-spec-json \
  --target riscv64im-unknown-none-elf | jq '.["atomic-cas"] = true'
```

## Two things that look wrong but aren't

**The ISA has no `A` (atomics) extension, yet the spec sets `atomic-cas: true`.**
Two different layers:
- *ISA / circuit* (what the standard defines): no `A` — the emitted guest binary
  has zero atomic instructions. On a single hart they'd be pointless circuit cost.
- *rustc target spec* (`atomic-cas: true` + `+forced-atomics`): a frontend
  setting that only lets the atomic **API** (`core::sync::atomic`, `Arc`,
  `bytes`, …) type-check. Codegen then lowers those ops to plain load/store
  (`-Cpasses=lower-atomic`), sound because single-hart.

So `atomic-cas: true` isn't a contradiction of "no atomics in the circuit" — it's
how you get there for code that uses `Arc`/`bytes`. A target with
`atomic-cas: false` (stock `riscv64im-unknown-none-elf`) refuses the API at the
language level, so `bytes` fails to compile and `lower-atomic` can't help (it runs
after rustc already rejected the call).

**The name says `zicclsm` but `features` doesn't list it.** Zicclsm (misaligned
load/store) is a runtime/VM property checked by the eth-act compliance monitor,
not a rustc feature — it can't appear in `features`.

## Why CI uses nightly + build-std

This is a JSON spec, not a built-in rustc target, so there's no prebuilt
`core`/`alloc`: the job uses nightly with `-Z build-std=core,alloc` and
`-Z json-target-spec`.

[std]: https://github.com/eth-act/zkvm-standards/blob/main/standards/riscv-target/target.md
[ere]: https://github.com/eth-act/ere/blob/master/crates/compiler/sp1/src/rust_rv64ima/riscv64ima-unknown-none-elf.json
