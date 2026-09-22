# EIP-8365: BLS withdrawal credential retirement
- **Layer:** CL
- **EIP status:** Draft (spec not yet merged; pending [PR #12097](https://github.com/ethereum/EIPs/pull/12097) — content fetched from the PR)
- **Hegota status:** Proposed (CFI) — acdc/184, 2026-08-06 (champion: NC)
- **Authors:** NC
- **Links:** [spec (PR)](https://github.com/ethereum/EIPs/pull/12097) · [discussion](https://ethereum-magicians.org/t/eip-8365-bls-withdrawal-credential-retirement/29284)
- **Prior fork history:** none

## TL;DR
First stage of a retire → drain → remove plan for the ~9,100 remaining mainnet validators with genesis-era 0x00 BLS withdrawal credentials. A standing epoch rule force-exits all active 0x00 validators (capped per epoch), and deposits creating new 0x00 validators are silently skipped. Balances are untouched: rotation to 0x01 via `BLSToExecutionChange` stays open and pays out in full via the sweep.

## What it changes
- New epoch-processing step `process_bls_credential_retirement` (after `process_registry_updates`): each epoch, up to `MAX_RETIREMENTS_PER_EPOCH` (TBD; indicative 8 ≈ churn-matched, draining in ~5 days; 1 ≈ six weeks) active 0x00 validators are exited via standard `initiate_validator_exit`, in index order. Standing rule, not a one-shot — also catches stragglers surfacing from the activation queue.
- `apply_pending_deposit` modified: deposits that would create a new validator with 0x00 credentials are silently skipped and the ETH is burned in the deposit contract (same semantics as invalid-signature deposits under EIP-6110; same permanent-rejection shape as EIP-8205). Top-ups to existing validators are unaffected.
- Retired validators are frozen: no rewards, no penalties, balance intact, out of all duty selection.
- Unchanged: `BLSToExecutionChange` (credential rotation to 0x01/0x02 keeps working — after exit, rotation makes the validator fully withdrawable and the sweep pays the entire balance); voluntary exits.
- No EL changes.

## Motivation
The 0x00 population (down from ~600k to ~9,290) has stalled: conversions are in double digits per month and hundreds of validators appear to have lost keys. While any 0x00 validator exists, every fork must carry `process_bls_to_execution_change`, its gossip topic and op pool. Worse, BLS is not post-quantum secure: `BLSToExecutionChange` reveals the withdrawal pubkey, so a quantum adversary could derive the key and race credential changes — post-quantum key registries cannot accommodate validators with no execution address. Natural ejection of offline lost-key validators would take ~28 years.

## Dependencies & related EIPs
- Requires EIP-6110 (deposit pipeline, Pectra), EIP-7251 (Pectra), EIP-7732 (ePBS, Glamsterdam — builder credential routing interplay).
- Stage 2 is EIP-8367 (balance sunset, requires 8365); a final post-quantum-stage EIP would remove the remaining 0x00 machinery entirely.
- Explicitly follows the SELFDESTRUCT staged-deprecation precedent: EIP-6049 → EIP-6780 → EIP-4758 (in this cluster).
- Deposit-rejection semantics deliberately mirror EIP-8205's credential-mismatch rejection.

## Impact on ethrex / client teams
- None/minimal for ethrex (CL-only). The EIP states explicitly: no execution-layer changes required.
- Only indirect touchpoint: EL deposit-contract behavior is unchanged; the burn happens by the CL skipping the deposit. Deposit tooling is unaffected unless it creates 0x00 deposits (no maintained tooling does).
- For CL teams: medium — new epoch step, deposit guard, fork tests.

## Open questions & controversies
- Force-exiting a class of validators is a notable governance step; justified as a uniform rule over an objective on-chain property with a permissionless remedy (rotation), citing repricings/SELFDESTRUCT/inactivity-leak precedent — but it sets a precedent of the protocol retiring a user class.
- `MAX_RETIREMENTS_PER_EPOCH` is TBD; trades drain time vs. exit-churn share. Retirement shares the exit queue with ordinary voluntary exits during the window.
- Holders of only the signing key (not withdrawal key) can exit but not rotate — they stay stranded (already true today; this EIP doesn't worsen it, but 8367's sunset would burn those balances).
- Spec is in PR (not merged), constants TBD, tests TBD — early maturity.
