#!/usr/bin/env python3
"""P0 — guard provenance: the assertion cannot be stripped.

Every published EIP-7906 proof-of-concept assumes the POST_TX assertion is present on the
victim's transaction. But under EIP-8141 whoever composes the transaction composes the
frame list — so in exactly the threat models that matter (a phishing frontend, compromised
signing infrastructure) the adversary omits the guard and the honest wallet signs a
guardless transaction. If that hole is open, every intent-integrity defense is voluntary.

This scenario closes it. The victim transacts through an account whose VERIFY frame
requires a correctly parameterized POST_TX assertion, so the hostile composer cannot omit
it, point it at a harmless contract, or relax its parameters.

Six attempts at the same malicious intent (grant the attacker an unlimited allowance):

  A honest body + correct guard             -> mines
  B malicious body, guard omitted           -> invalid (account refuses)
  C malicious body, guard replaced by no-op -> invalid (account refuses)
  D malicious body, guard weakened          -> invalid (account refuses)
  E honest body, non-owner signature        -> invalid (account refuses)
  F malicious body + correct guard          -> invalid (the assertion itself fires)

B-E prove the guard is unstrippable; F proves the guard works when present. Together they
close the gap between "an assertion exists" and "an assertion is enforced".
"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *  # noqa: F403


def main():
    b = bank()

    print("\n  [setup] deploying")
    shim = b.deploy(compile_yul("TxIntrospection.yul"), "TxIntrospection")
    guard = b.deploy(compile_sol("guards/Guards.sol", "AssertNoApproval"), "AssertNoApproval")
    token = b.deploy(compile_sol("targets/MockERC20.sol", "MockERC20"), "MockERC20")
    noop = b.deploy(compile_sol("guards/Guards.sol", "AssertCounterparties"), "no-op stand-in")

    # The victim's smart account, funded at construction: empty calldata is the VERIFY
    # entry point, so a plain transfer into it would run the verification path.
    account = b.deploy(compile_yul("accounts/GuardMandatingAccount.yul"),
                       "GuardMandatingAccount", value=2 * 10**18)
    b.send_tx(token, 0, encode_call("mint(address,uint256)", account, 1000), gas=GAS_WRITE_CALL)

    attacker = "0x" + "ba" * 20

    # The policy: no approval may be granted on this account's behalf, to anyone.
    good_guard_data = encode_call(
        "assertNoApprovalOutside(address,address,address[])", shim, account, [])
    commitment = keccak(good_guard_data)
    b.send_tx(account, 0, encode_call("setPolicy(address,bytes32)", guard, commitment),
              gas=GAS_WRITE_CALL)
    print(f"    policy: guard={guard} commitment=0x{commitment.hex()[:16]}…")

    ev = Evidence(
        scenario="poc0_guard_provenance",
        title="Guard provenance — the assertion cannot be stripped",
        models="The structural gap in every published EIP-7906 proof-of-concept: the "
               "transaction composer controls the frame list, so a hostile composer can "
               "omit the assertion",
        defense_kind="reverts the attack",
        addresses={"shim": shim, "guard": guard, "token": token,
                   "account": account, "attacker": attacker},
    )

    honest_body = sender_frame("0x" + "77" * 20, value=10**14)
    malicious_body = sender_frame(
        token, data=encode_call("approve(address,uint256)", attacker, 2**256 - 1))
    good_guard = guard_frame(guard, good_guard_data)

    def allowance_of():
        return int(rpc("eth_call", [{"to": to_checksum_address(token), "data": "0x" +
            encode_call("allowance(address,address)", account, attacker).hex()}, "latest"]), 16)

    cases = []

    print("\n  [A] honest body + correct guard -> should mine")
    o = submit(frame_tx(account, b, [verify_frame(account), honest_body, good_guard]),
               "A honest + correct guard")
    if not o.mined:
        raise RuntimeError(f"the correctly guarded honest transaction must mine: {o.simulation}")
    cases.append(("A honest body + correct guard", "mines", o.mined, o.txhash))

    print("\n  [B] malicious body, guard omitted -> account must refuse")
    o = submit(frame_tx(account, b, [verify_frame(account), malicious_body]),
               "B malicious, guard omitted")
    if o.mined:
        raise RuntimeError("a guardless transaction was accepted: provenance is NOT closed")
    cases.append(("B malicious body, guard omitted", "invalid", o.mined, None))

    print("\n  [C] malicious body, guard replaced by a harmless contract -> refuse")
    o = submit(frame_tx(account, b, [verify_frame(account), malicious_body,
                                     guard_frame(noop, encode_call("assertOnly(address,address[])", shim, []))]),
               "C malicious, guard substituted")
    if o.mined:
        raise RuntimeError("a substituted guard was accepted")
    cases.append(("C malicious body, guard substituted", "invalid", o.mined, None))

    print("\n  [D] malicious body, genuine guard but attacker allowlisted -> refuse")
    weakened = encode_call("assertNoApprovalOutside(address,address,address[])",
                           shim, account, [attacker])
    o = submit(frame_tx(account, b, [verify_frame(account), malicious_body,
                                     guard_frame(guard, weakened)]),
               "D malicious, guard weakened")
    if o.mined:
        raise RuntimeError("a weakened guard was accepted: the policy commitment is not enforced")
    cases.append(("D malicious body, guard weakened", "invalid", o.mined, None))

    print("\n  [E] honest body, correct guard, signed by a non-owner -> refuse")
    stranger = Signer(bytes(range(9, 41)))
    o = submit(frame_tx(account, stranger, [verify_frame(account), honest_body, good_guard]),
               "E non-owner signature")
    if o.mined:
        raise RuntimeError("a non-owner signature was accepted")
    cases.append(("E honest body, non-owner signature", "invalid", o.mined, None))

    print("\n  [F] malicious body + correct guard -> the assertion itself must fire")
    o = submit(frame_tx(account, b, [verify_frame(account), malicious_body, good_guard]),
               "F malicious + correct guard")
    if o.mined:
        raise RuntimeError("the assertion failed to catch the hidden approval")
    cases.append(("F malicious body + correct guard", "invalid", o.mined, None))

    residual = allowance_of()
    print(f"\n    attacker's allowance after all six attempts: {residual}")
    if residual != 0:
        raise RuntimeError(f"an approval survived: {residual}")

    ev.phase_a = {"description": "cases B-E are the hostile composer trying to evade the "
                                 "assertion; each was refused by the account before the body ran",
                  "cases": [{"case": c, "expected": e, "mined": m, "tx": t} for c, e, m, t in cases]}
    ev.phase_b = {"attacker_allowance_after_all_attempts": residual,
                  "honest_transaction_mined": cases[0][3]}
    ev.note("Guard provenance is closable on-chain with primitives EIP-8141 already ships: "
            "TXPARAM(0x09) for the frame count, FRAMEPARAM for each frame's mode and target, "
            "FRAMEDATACOPY for its calldata, and a reverting VERIFY frame to invalidate the "
            "transaction. No specification change is required.")
    ev.note("Cases B-E were refused at MEMPOOL ADMISSION, because VERIFY frames run in the "
            "validation prefix that admission simulates. A bare POST_TX assertion is instead "
            "admitted and then silently never mined, so mandating the guard at the account "
            "also converts a silent failure into an immediate, actionable rejection.")
    ev.note("The account's VERIFY frame reads only its own storage and makes no external "
            "calls, so it stays admissible through the public mempool under the ERC-7562 "
            "validation observer rather than requiring builder-direct inclusion.")
    return ev


if __name__ == "__main__":
    run_scenario("P0 — guard provenance (the assertion cannot be stripped)", main)
