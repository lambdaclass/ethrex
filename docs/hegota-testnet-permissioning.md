# Hegotá testnet — validator permissioning

Working document. The Hegotá testnet is permissioned the way Sepolia is: **anyone may
sync and peer, but becoming a validator requires an access token.** That asymmetry is
what makes the chain both open to join and stable to run, and it is worth stating
plainly because the two halves are often confused. Nothing here restricts RPC access,
block propagation, or running a node.

Permissioning is entirely a genesis-and-contract concern. The execution layer needs no
changes: ethrex recognises deposits by matching a log's address against
`ChainConfig::deposit_contract_address` and its first topic against the deposit event
hash, so a gated deposit contract that emits the standard event is transparent to it.

## The contracts

`pk910/gated-deposit-contract`, installed by `ethereum-genesis-generator` when
`DEPOSIT_CONTRACT_GATED=true`. Two contracts, deliberately separated:

**`GatedDepositContract`**, at the chain's deposit-contract address. It is the mainnet
deposit contract plus a single `depositGater.check_deposit()` call before a deposit is
processed. Same Merkle tree, same hashing, same `DepositEvent` signature and data, and
critically **no additional events** — custom events on a deposit contract are what broke
Sepolia at the Electra fork. Its storage slot `0x41` holds the gater's address.

**`TokenDepositGater`**, at `0x00000000a11acc355c0de0000a11acc355c0de00`. An ERC-20
named "Deposit Token" with symbol "Deposit" and `decimals() == 0`, so one token is one
validator slot and tokens cannot be subdivided. It burns one token per gated deposit.

## How a deposit is judged

`check_deposit` runs in this order:

1. **Caller check.** `require(hasRole(DEPOSIT_CONTRACT_ROLE, _msgSender()))`. Only the
   deposit contract may call the gater.
2. **Optional chained gater.** If a custom gater address is configured, it is consulted
   first and may approve unilaterally. Unset on this chain.
3. **Classification.** A deposit is a **top-up** when the signature is 96 zero bytes
   *and* the withdrawal credentials are 32 zero bytes; top-ups use the pseudo-prefix
   `0xffff`. Otherwise the deposit type is the **first byte of the withdrawal
   credentials**.
4. **Per-type gate.** Two bits: `0x01` = blocked, `0x02` = no token required. Blocked
   reverts. Otherwise, unless the type is token-free, the gater requires
   `balanceOf(sender) > 0` and burns one token.

**The token is checked and burned against the caller of `deposit()`, not against the
validator.** Whoever *sends the transaction* must hold the token. This is the single most
common misunderstanding: handing someone a token means funding the address they will
submit from, not their withdrawal address and not their validator pubkey.

## The policy on this chain

| Prefix | Meaning | Setting | Effect |
| --- | --- | --- | --- |
| `0x00` | BLS withdrawal credentials | `0x01` | **Blocked** |
| `0x01` | Execution-address credentials | `0x00` | Allowed, one token |
| `0x02` | Compounding credentials (EIP-7251) | `0x00` | Allowed, one token |
| `0x03` | Builder credentials | `0x00` | Allowed, one token |
| `0xffff` | Top-up | `0x02` | Allowed, **no token** |

Reasons, since each is a deliberate choice:

- **`0x00` blocked.** BLS credentials cannot withdraw to an execution address, so they
  only create validators nobody can exit cleanly. Blocking the prefix at the gate is
  cheaper than discovering it after activation.
- **`0x01`, `0x02`, `0x03` allowed with a token.** These are the credential types a
  validator on this chain should use. `0x03` is included because the chain runs Gloas,
  which makes builder credentials reachable.
- **`0xffff` token-free.** A top-up raises an existing validator's balance rather than
  creating a new one, so gating it adds no permissioning value while breaking EIP-7251
  consolidation for operators who already hold a slot.

An absent settings key reads as storage zero, which is identical to an explicit `0x00`.
Only `0x00` and `0xffff` change behaviour; the rest are recorded for clarity.

## Admin roles, and the one that cannot be undone

`SimpleAccessControl` stores a role as a single storage word at
`keccak`-free key `bytes12(role) ‖ address`. Its semantics are:

- `hasRole` is `value >= 1`
- `isStickyRole` is `value == 2`
- `grantRole` writes `1`
- `revokeRole` refuses when `isStickyRole` is true

The genesis generator injects each `DEPOSIT_CONTRACT_ADMINS` entry with value **`2`**.

**A genesis admin is therefore permanent for the life of the chain.** It cannot be
revoked by any transaction. If that key is lost or leaked, the remedies are the
per-prefix kill switch below, or a new genesis. Admins granted later at runtime get
value `1` and revoke normally, so prefer delegating to runtime admins and keeping the
genesis key offline and unused.

This is also why the genesis admin must not be derived from the kurtosis default
mnemonic, which is public and identical on every deployment: a default-mnemonic admin
would be an unrevokable, publicly-controlled token mint, and the permissioning would be
decorative while appearing to work.

## Runbook: granting a third party validator slots

The gater ships an admin CLI, published as a container image. `$ADMIN_KEY` is an admin
private key; `$RPC` is any of the chain's RPC endpoints.

Confirm the deployment before touching it. This prints the gater address, the token
supply, admin stickiness, and each per-prefix configuration; check them against the
table above:

```
docker run --rm -it pk910/gated-deposit-contract-cli -k $ADMIN_KEY -r $RPC status
```

Grant `N` validator slots. `<DEPOSITOR_EOA>` is the address that will **send** the
deposit transaction:

```
docker run --rm -it pk910/gated-deposit-contract-cli -k $ADMIN_KEY -r $RPC \
  mint --to <DEPOSITOR_EOA> --amount <N>
```

Delegate or withdraw admin rights. A runtime grant is revocable; a genesis one is not:

```
docker run --rm -it pk910/gated-deposit-contract-cli -k $ADMIN_KEY -r $RPC grantAdmin  --account <ADDR>
docker run --rm -it pk910/gated-deposit-contract-cli -k $ADMIN_KEY -r $RPC revokeAdmin --account <ADDR>
```

Kill switch. Blocking a prefix stops new validators of that type immediately, without
touching anyone's tokens, and is the fastest response to an abused or leaked admin key:

```
docker run --rm -it pk910/gated-deposit-contract-cli -k $ADMIN_KEY -r $RPC \
  setConfig --prefix 0x01 --blocked true
```

## Verifying the permissioning actually works

A chain whose deposits all revert looks healthy until someone tries to join, so test
both directions explicitly:

1. A deposit from a token-holding address **succeeds**, and the beacon chain observes
   the deposit request.
2. A deposit from an address holding no token **reverts** with "Not enough tokens".
3. A deposit with `0x00` withdrawal credentials **reverts** with "Deposit type is
   blocked", even from a token holder.
4. A top-up from an address holding no token **succeeds**.
5. `eth_getCode` at the gater address is non-empty, and the deposit contract's storage
   slot `0x41` holds the gater address.
6. No address derived from a public mnemonic holds a token balance or an admin role.

If every deposit reverts with "Only deposit contract can call this function", the
deposit-contract address was overridden. The gater template grants
`DEPOSIT_CONTRACT_ROLE` to one hard-coded address and the generator never patches that
key, so the chain's deposit contract must stay at the default address.
