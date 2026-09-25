### The stateless schema id does not identify the encoding

**Where:** `STATELESS_INPUT_SCHEMA_ID` in `crates/common/types/stateless_ssz.rs`.

Upstream keeps the stateless input schema id at `0x1501`
(`fork_index 0x15 << 8 | revision 0x01`) across incompatible body changes. Three
encodings have now shipped under it: `tests-zkevm@v0.6.2`, then #3248 + #3278,
then #3356, which moved `state`, `codes` and `public_keys` from `SszList` to
`ProgressiveList`. ethrex speaks the last one.

The consequence is that the 2-byte prefix cannot be used to detect a stale or
mismatched bundle. A wrong-dialect input is accepted by the id check and then
fails later — in SSZ decode, or on a root that does not match — rather than being
rejected up front for what it is. `only_amsterdam_schema_id_decodes` therefore
proves less than its name suggests.

Worth raising upstream: a revision field that does not move across a body change
provides no version negotiation at all.

---

### ZisK guest program hash changes with the `unsync_cell` gate

**Where:** `crates/common/types/block.rs`, `transaction.rs`.

The gate on the single-threaded `unsync_cell::OnceCell` moved from
`all(feature = "eip-8025", target_arch = "riscv64")` to
`all(feature = "zisk", target_arch = "riscv64")` when the `eip-8025` feature was removed.

The guest ELFs were previously built `--features "<zkvm>-build-elf,ci"`, which never enabled
`eip-8025`, so they compiled the atomic `once_cell` variant. `bin/zisk/Cargo.toml` does enable
`ethrex-common/zisk`, so **the ZisK guest now compiles the `unsafe impl Sync` cell instead**.
That changes the ELF bytes and therefore the program hash and verification key.

This is intended (the guest is single-threaded, so the unsync cell is sound and cheaper), but it
is a VK change rather than a no-op refactor, and the diffstat presents it as a file rename
(`eip8025_cell.rs` → `unsync_cell.rs`). Anyone pinning a ZisK VK across this change must
re-register it. The `stateless-validator` crate now forwards `ethrex-common/zisk` from its own
`zisk` feature so the two ZisK guests do not disagree on the cell type.

---

### Release signing key is an unprotected repository secret

**Where:** `.github/workflows/tag_release.yaml`.

`MINISIGN_SECRET_KEY` is a plain repository secret. There is no `environment:` on
`finalize-release` or `dry-run-release-assets`, and `gh api repos/lambdaclass/ethrex/rulesets`
shows only branch-targeted rulesets, so the `github.ref_type == 'tag'` condition is a workflow
check rather than an enforced boundary: anyone who can push a tag can reach the signing key.

This is a repository-settings change, not a code change, so it is recorded here rather than
fixed in the tree. Recommended:

1. Move `MINISIGN_SECRET_KEY` / `MINISIGN_PASSWORD` into a GitHub **Environment** with required
   reviewers, and add `environment:` to the two jobs that sign.
2. Add a ruleset targeting `refs/tags/v*` restricting who may create release tags.

Until then, the compromise of that key is silent and durable: signatures would still verify
against the committed `.github/minisign.pub`.
