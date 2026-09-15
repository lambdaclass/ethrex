# Dogfooding the FOCIL enforcement spec

Two implementations of `docs/eip-focil-frametx.md`, each started from a client that has
only one half of what the spec assumes, each run as a joiner against the live Hegotá
testnet. This file collects what the spec text failed to say, said ambiguously, or said
wrongly, as found by implementing from it. Entries name the section they are about and
what an implementer needed that was not there. Another agent may review and apply them.

## Setup

The two starting points, and what each already has:

| Base | Head | Has | Lacks |
| --- | --- | --- | --- |
| `focil-devnet-0` | `c28af3278` | EIP-7805 engine surface, inclusion-list builder and Profile 1 validator, the newest glamsterdam-devnet-8 fixes | every frame EIP: the branch's head commit removed EIP-8141 outright, 49 files and 12,988 deletions |
| `frames-devnet-0` | `d587cf9ff` | EIP-8141 at `b75cbe6115`, the mempool prefix simulation and validation observer | EIP-7805 entirely, EIP-8250, EIP-8272 |

`frames-devnet-0` is an ancestor of `hegota-testnet`: the merge that produced the running
chain took it in whole. `focil-devnet-0` is not; it diverged and then deleted frames.

The spec delegates all four prerequisite EIPs by reference and specifies none of them.
Implementing those from their own texts is not a test of this spec, so each leg brings
them in from the branches that already implement them, and implements only what this spec
specifies, the Profile 2 enforcement layer, fresh from the text. That layer is removed from
the prerequisite base first, so that neither leg can read the existing implementation.

Sizing, from merge dry runs:

| Step | Result |
| --- | --- |
| `hegota-testnet` into `focil-devnet-0` at its head | 49 conflicts, 27 of them in `crates/`, most where the head commit deleted a frames file that `hegota-testnet` modified |
| `hegota-testnet` into `frames-devnet-0` | fast-forward, no merge needed |
| delta `frames-devnet-0` to `hegota-testnet` | 62 files in `crates/`, +7,806 / -1,221 |
| delta `focil-devnet-0` to `hegota-testnet` | 90 files in `crates/`, +14,188 / -2,158 |

## Leg 1: FOCIL base

### Prerequisite merge

(inventory and resolutions recorded below as they are made)

### Spec feedback

(entries added as implementation proceeds)

## Leg 2: frames base

### Spec feedback

(entries added as implementation proceeds)

## Cross-implementation agreement

(recorded once both nodes follow the chain)
