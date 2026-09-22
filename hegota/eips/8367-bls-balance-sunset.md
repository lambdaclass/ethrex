# EIP-8367: Balance sunset for retired BLS validators
- **Layer:** CL
- **EIP status:** Draft (spec not yet merged; pending [PR #12099](https://github.com/ethereum/EIPs/pull/12099) — content fetched from the PR)
- **Hegota status:** Proposed (CFI) — acdc/184, 2026-08-06 (champion: NC)
- **Authors:** NC
- **Links:** [spec (PR)](https://github.com/ethereum/EIPs/pull/12099) · [discussion](https://ethereum-magicians.org/t/eip-8367-balance-sunset-for-retired-bls-validators/29299)
- **Prior fork history:** none

## TL;DR
Second stage of the 0x00-credential retirement arc: after EIP-8365 freezes the remaining 0x00 validators, this EIP clamps each such balance to a ceiling that declines linearly from 64 ETH to zero over a published multi-year window (indicatively 2–3 years). Rotating credentials via `BLSToExecutionChange` at any point releases the full remaining balance. The goal: by the post-quantum transition every 0x00 entry holds zero, so deleting them is cleanup, not confiscation.

## What it changes
- Constants: `SUNSET_START_EPOCH`, `SUNSET_END_EPOCH` (absolute epochs, TBD, immune to fork-timing slippage), `SUNSET_INITIAL_CEILING = 64 ETH` (above the highest observed 0x00 balance, so nothing drops abruptly at start).
- New epoch step `process_balance_sunset`: `balance = min(balance, ceiling(epoch))` via `decrease_balance` for every validator with 0x00 credentials. The clamp is idempotent, deterministic, pure function of epoch; top-ups are absorbed by the ceiling next epoch (so they can't defeat the schedule).
- The reduction is a burn — nobody receives the funds; upper bound ~343k ETH, in practice far less as live holders rotate.
- `BLSToExecutionChange` stays available throughout and beyond the window; from the epoch of rotation the clamp no longer applies and the sweep pays out the remainder.
- Schedule constants are ordinary spec parameters, intended to be re-tuned at intermediate forks if the post-quantum timeline moves; zeroing residuals at the post-quantum fork remains a last-resort backstop.
- No EL changes.

## Motivation
Freezing alone (EIP-8365) leaves thousands of non-zero registry entries the CL must carry forever, and the eventual removal would itself be a confiscation decision taken at the worst possible moment (the post-quantum fork). A gradual decline delivers the deprecation notice through dashboards/balance alerts (announcement channels have saturated — conversions decayed to double digits per month), keeps a salvage path open until the end (missing a year costs a fraction, not everything), and leaves only zero-balance entries to delete. Cites the inactivity leak as the existing precedent for rule-based, class-conditional balance reduction.

## Dependencies & related EIPs
- Requires EIP-8365 (clamping active validators' balances would break rewards/penalties/effective-balance accounting — only safe once all 0x00 validators are exited). The two stages may activate in the same or consecutive forks.
- Final stage (removal of `BLSToExecutionChange`, gossip topic, op pool, registry entries) is a separate future EIP at the post-quantum fork.
- Follows the SELFDESTRUCT staged-deprecation precedent (EIP-6049 → EIP-6780 → EIP-4758, the latter in this cluster).
- Mirrors post-quantum registry proposals that apply inactivity-leak-style balance drains to non-registrants.

## Impact on ethrex / client teams
- None/minimal for ethrex (CL-only; explicitly no execution-layer changes). The burn affects CL accounting and total supply, not EL state.
- For CL teams: small-to-medium — one pure epoch-processing function plus fork tests; clients may keep an index of 0x00 validators (bounded, ~9,290, shrinking) to make the per-epoch scan negligible.

## Open questions & controversies
- This is the controversial stage: it burns balances of anyone who doesn't act — chiefly lost-key holders whose funds are unrecoverable anyway, but it sets the precedent of the protocol reducing balances of a class defined by credential type.
- `SUNSET_START_EPOCH`/`SUNSET_END_EPOCH` are TBD and must track an uncertain post-quantum timeline, requiring re-tuning at intermediate forks — governance overhead.
- Requires EIP-8365's forced retirement to already be accepted, compounding the governance precedent.
- Spec is in PR (not merged), no test vectors yet — early maturity.
