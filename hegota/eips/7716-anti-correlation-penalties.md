# EIP-7716: Anti-correlation attestation penalties
- **Layer:** CL
- **EIP status:** Stagnant
- **Hegota status:** Proposed (CFI) (acdc/177, 2026-04-16; presented again acdc/184, 2026-08-06)
- **Authors:** dapplion, Toni Wahrstätter, Vitalik Buterin
- **Links:** [spec](https://eips.ethereum.org/EIPS/eip-7716) · [discussion](https://ethereum-magicians.org/t/eip-7716-anti-correlation-attestation-penalties/20137)
- **Prior fork history:** none (proposed for the first time for Hegota)

## TL;DR
Scales missed-attestation penalties by how correlated the misses are in the same slot: an isolated failure stays cheap, a failure simultaneous with thousands of others costs up to 4x more. The goal is a direct economic incentive to diversify clients, hosting, geography, and ISPs. The EIP is marked Stagnant and its spec is extremely thin — one formula and two constants.

## What it changes
- **New beacon state variable:** `NET_EXCESS_PENALTIES`, a running EWMA-like tracker of excess missed attestations.
- **Penalty scaling:** per slot, compute
  `penalty_factor = min( (non_attesting_balance * PENALTY_ADJUSTMENT_FACTOR) // (NET_EXCESS_PENALTIES * total_active_balance + 1), MAX_PENALTY_FACTOR )`
  with `PENALTY_ADJUSTMENT_FACTOR = 4096` and `MAX_PENALTY_FACTOR = 4`. Missed-attestation penalties are multiplied by this factor.
- **State update:** `NET_EXCESS_PENALTIES = max(1, NET_EXCESS_PENALTIES + penalty_factor) - 1` — decays by 1 per slot toward 1.
- Behavior: under stable participation `penalty_factor` is 1 (status quo); a sudden drop in participation pushes the factor above 1 until `NET_EXCESS_PENALTIES` catches up; unusually high participation temporarily zeroes it.
- Hard-fork required (changes reward/penalty accounting).

## Motivation
Today a validator that misses an attestation pays the same whether it failed alone or as part of a mass outage — so there's no economic reason to avoid running the same client, cloud provider, or region as everyone else (beyond all-or-nothing slashing risk). Correlated failures are the systemic risk to finality; this makes them proportionally expensive and makes diversification (multi-client, multi-host, multi-region) directly profitable. It complements the existing inactivity leak, which only triggers after ~4 epochs of non-finality, by pricing correlation into ordinary per-slot misses.

## Dependencies & related EIPs
- **Standalone:** no hard dependencies; touches only CL penalty accounting.
- **Related:** the inactivity leak (existing mechanism, handles liveness failures at finality scale); EIP-7251 (MaxEB) changes how much stake sits behind single operators, amplifying the effect; client-diversity initiatives are the social counterpart.
- **Interaction with this cluster:** shorter slots (EIP-8198) would increase slot frequency and thus how fast `NET_EXCESS_PENALTIES` reacts — constants would need re-tuning; more attestations per second from 8243/8334 doesn't change the accounting.

## Impact on ethrex / client teams
- **None for ethrex** — CL-only penalty accounting (beacon state field + per-slot processing). No EL, Engine API, or RPC surface.
- For CL teams: small in code (one state field, one formula) but needs careful tests around penalty math; the bigger cost is analysis/simulation, not implementation.
- Effort for ethrex: **none**.

## Open questions & controversies
- **Spec is Stagnant and skeletal:** the security section literally ends with "TBD". The acknowledged open attack — splitting validator views so a proposer can inflate the `penalty_factor` for consecutive slots at little risk — is unresolved.
- Penalty now depends on other participants' behavior, not just your own — philosophically contentious (you can be punished more because a big operator went down in your slot).
- Disagreement about whether attestation-level penalties move operator behavior vs. existing slashing/inactivity-leak incentives.
- Championed for Hegota by Oisin Kyne (not an author) — presented twice (acdc/177, acdc/184), suggesting renewed interest despite Stagnant status.
