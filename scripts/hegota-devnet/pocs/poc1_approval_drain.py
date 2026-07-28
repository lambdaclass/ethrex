#!/usr/bin/env python3
"""P1 — hidden unlimited approval with a delayed drain.

The most common real-world wallet-drain pattern. The victim is shown a harmless action
("claim", "connect", "verify"); the transaction they actually sign grants an unlimited
token allowance. No funds move at signing time, so nothing looks wrong. The attacker
drains later with their own transaction.

Phase A  the victim signs the harmless-looking action and is drained afterwards.
Phase B  the same intent, from an account that mandates a POST_TX assertion forbidding
         any approval on the victim's behalf: the transaction is invalidated, no allowance
         is ever created, and the attacker's identical later sweep fails for want of one.

The guard is MANDATED by the account rather than attached voluntarily, because the threat
model here is a hostile transaction composer — see poc0_guard_provenance.py.
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
    drainer = b.deploy(compile_sol("targets/Targets.sol", "Drainer"), "Drainer")
    attacker = "0x" + "ba" * 20

    ev = Evidence(
        scenario="poc1_approval_drain",
        title="Hidden unlimited approval with a delayed drain",
        models="The wallet-drainer pattern behind the 2025-2026 frontend and registrar "
               "hijacks (a DNS hijack in Nov 2025, ~$1M+; a registrar takeover in Apr 2026, "
               "~$1.2M), where phishing interfaces prompted unlimited-approval signatures",
        defense_kind="reverts the attack",
        addresses={"shim": shim, "guard": guard, "token": token,
                   "drainer": drainer, "attacker": attacker},
    )

    def allowance(owner):
        return int(rpc("eth_call", [{"to": to_checksum_address(token), "data": "0x" +
            encode_call("allowance(address,address)", owner, drainer).hex()}, "latest"]), 16)

    def token_balance(who):
        return int(rpc("eth_call", [{"to": to_checksum_address(token), "data": "0x" +
            encode_call("balanceOf(address)", who).hex()}, "latest"]), 16)

    # ---------------- phase A: plain victim, drained after the fact ----------------
    print("\n  [phase A] victim signs the 'claim'; nothing moves; drained later")
    victim = fresh_account(b, 3 * 10**17, "victim A")
    b.send_tx(token, 0, encode_call("mint(address,uint256)", victim.address, 1000), gas=GAS_WRITE_CALL)

    victim.send_tx(token, 0, encode_call("approve(address,uint256)", drainer, 2**256 - 1),
                   gas=GAS_WRITE_CALL)
    granted = allowance(victim.address)
    moved_at_signing = 1000 - token_balance(victim.address)
    print(f"    allowance granted: {'unlimited' if granted > 2**200 else granted}; "
          f"tokens moved at signing: {moved_at_signing}")
    if granted == 0:
        raise RuntimeError("phase A setup failed: no allowance was granted")

    b.send_tx(drainer, 0, encode_call("sweep(address,address,address,uint256)",
                                      token, victim.address, attacker, 1000), gas=GAS_WRITE_CALL)
    stolen = token_balance(attacker)
    print(f"    attacker's later sweep took {stolen} tokens; victim has {token_balance(victim.address)}")
    if stolen == 0:
        raise RuntimeError("phase A did not demonstrate the drain")

    ev.phase_a = {"victim": victim.address, "allowance_unlimited": granted > 2**200,
                  "tokens_moved_at_signing": moved_at_signing, "stolen_later": stolen}
    ev.note("Phase A: nothing moved when the victim signed, which is exactly why the pattern "
            "works — the loss happened later, in a transaction the victim never saw.")

    # ------------- phase B: guarded account, the approval never lands -------------
    print("\n  [phase B] account mandates 'no approval on my behalf'")
    account = b.deploy(compile_yul("accounts/GuardMandatingAccount.yul"),
                       "GuardMandatingAccount", value=2 * 10**18)
    b.send_tx(token, 0, encode_call("mint(address,uint256)", account, 1000), gas=GAS_WRITE_CALL)

    guard_data = encode_call("assertNoApprovalOutside(address,address,address[])", shim, account, [])
    b.send_tx(account, 0, encode_call("setPolicy(address,bytes32)", guard, keccak(guard_data)),
              gas=GAS_WRITE_CALL)

    o = submit(frame_tx(account, b, [
        verify_frame(account),
        sender_frame(token, data=encode_call("approve(address,uint256)", drainer, 2**256 - 1)),
        guard_frame(guard, guard_data),
    ]), "the same hidden approval, guarded")
    if o.mined:
        raise RuntimeError("the hidden approval was accepted")

    residual = allowance(account)
    print(f"    allowance on the guarded account: {residual}")
    if residual != 0:
        raise RuntimeError(f"an allowance survived: {residual}")

    before = token_balance(attacker)
    sweep_failed = False
    try:
        b.send_tx(drainer, 0, encode_call("sweep(address,address,address,uint256)",
                                          token, account, attacker, 1000), gas=GAS_WRITE_CALL)
    except RuntimeError:
        sweep_failed = True
    delta = token_balance(attacker) - before
    print(f"    attacker's identical sweep: {'reverted' if sweep_failed else 'succeeded'} (delta {delta})")
    if delta != 0:
        raise RuntimeError("the guarded account was still drained")

    ev.phase_b = {"account": account, "guarded_tx_mined": o.mined, "residual_allowance": residual,
                  "attacker_sweep_reverted": sweep_failed, "attacker_delta": delta}
    ev.note("Phase B: the assertion fired on the approval itself, so the transaction was "
            "invalidated before any allowance existed. The attacker's later sweep then had "
            "nothing to spend.")

    # Negative control: an intended, allowlisted approval must still be permitted.
    print("\n  [negative control] an intended approval to an allowlisted spender must pass")
    allow_data = encode_call("assertNoApprovalOutside(address,address,address[])",
                             shim, account, [drainer])
    b.send_tx(account, 0, encode_call("setPolicy(address,bytes32)", guard, keccak(allow_data)),
              gas=GAS_WRITE_CALL)
    o2 = submit(frame_tx(account, b, [
        verify_frame(account),
        sender_frame(token, data=encode_call("approve(address,uint256)", drainer, 100)),
        guard_frame(guard, allow_data),
    ]), "intended approve(100) to an allowlisted spender")
    if not o2.mined:
        raise RuntimeError(f"the guard rejected a legitimate, allowlisted approval: {o2.simulation}")
    ev.extra["negative_control"] = {
        "description": "the guard is not a blanket ban: an approval to a spender the owner "
                       "allowlisted in the committed policy is permitted",
        "mined": o2.mined, "tx": o2.txhash, "allowance_after": allowance(account)}
    ev.note("Negative control: the same guard permitted a deliberate approval to an "
            "allowlisted spender, so it discriminates rather than blocking everything.")
    return ev


if __name__ == "__main__":
    run_scenario("P1 — hidden unlimited approval with a delayed drain", main)
