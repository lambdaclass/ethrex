# Hegotá devnet faucet, design notes

Status: **deployed** on the Hegota devnet, replacing `chainflag/eth-faucet`.
The funding key is dedicated to the faucet and must be rotated when the chain is
reset or made public.

## Why not the off-the-shelf one

The devnet ran `chainflag/eth-faucet:latest` (v1.2.1). It is broken on this fork
and cannot be fixed by configuration:

- `internal/chain/transaction.go:76` hardcodes `gasLimit := uint64(21000)` for
  both the dynamic-fee and legacy paths. There is no flag or env override.
- Under EIP-8038 state-growth pricing a transfer that **creates** an account
  costs far more than the historical 21000. Measured on this devnet:
  21000 to an existing account succeeds; a fresh account needs ~207391. So the
  faucet's transaction is included, runs out of gas, and burns its whole limit,
  failing for exactly the new addresses a faucet exists to serve.
- Two further flaws independent of gas: it rejects non-EIP-55 addresses
  (`{"msg":"invalid address"}` for the all-lowercase form most people paste), and
  it answers `HTTP 200` with a txhash for a transaction that then fails on
  chain, so a caller cannot distinguish success from failure.

Rebuilding a patched fork is awkward because the frontend is `go:embed`-ed from
`web/dist`, which is produced by a yarn stage.

## Rate limiting, the cases that matter

Ordered by how easily each one is missed.

1. **Proxy-aware client IP.** Behind a reverse proxy, `RemoteAddr` is always the
   proxy: one shared bucket, so the limit is either useless or a global lockout.
   Take the client from `X-Forwarded-For` counting **from the right**, trusting
   exactly `PROXY_COUNT` hops. Never trust the leftmost entry, a client can put
   anything there, which is a free bypass.
2. **Bucket IPv6 by /64, not /128.** A single user usually controls a whole /64.
   Per-/128 limiting lets them rotate through effectively unlimited addresses.
   This is the most common faucet bypass. IPv4 stays per-address (CGNAT already
   forces sharing).
3. **Per-recipient limit, keyed on the lowercased address.** Without it, IP
   rotation refills one address. Keying on the raw string makes `0xabc…` and
   `0xABC…` two buckets for one account, a one-character bypass.
4. **Serialize sending, and own the nonce.** Concurrent claims must not build the
   same nonce. One lock around send, a local nonce, resync from chain and retry
   once on `nonce too low` / `already known`. This is not hypothetical: the
   deployed faucet wedged exactly this way when another sender used its account.
5. **In-flight dedupe per address.** A double-click must not send twice; the
   per-address window catches it only after both are already in the mempool.
6. **Global budget and a reserve floor.** Cap claims per hour, and refuse below a
   balance floor so the faucet cannot be drained and always retains gas money.
7. **Recipient balance cap.** Refuse addresses that already hold plenty. Kills
   top-up spam.
8. **Bounded limiter state.** Unpruned maps are a memory DoS. Prune on access and
   hard-cap the entry count.
9. **Request hygiene.** Body size cap, JSON only, read/write timeouts.
10. **Honest results.** Wait for the receipt and report a failed status as a
    failure.

## Deliberate non-goals

- No persistence: limiter state is in memory, so a restart forgives outstanding
  windows. Acceptable for a devnet; a restart is not attacker-triggerable.
- No captcha or auth. If abuse becomes real, put auth at the proxy rather than
  growing this service.

## Operational notes

- **The funding key must be dedicated.** The deployed faucet funded itself from one
  of the well-known kurtosis genesis accounts, which is how its nonce got
  clobbered by ordinary test traffic: anyone running standard tooling against the
  devnet spends from those same accounts, accidentally.
- `FAUCET_AMOUNT=10` on the old deployment dispensed 10 ETH while the USER-GUIDE
  advertised 1 ETH. Pick one.
- Keep the `/api/claim` path and `{"address": …}` body so the USER-GUIDE stays
  correct.
