// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {ITxIntrospection} from "./ITxIntrospection.sol";

/// @notice The assertion-guard library. Each guard is invoked as the resolved target of a
///         POST_TX frame and takes its whole policy from that frame's calldata, so one
///         deployment serves every scenario and the asserted policy is visible in the
///         transaction itself.
///
/// Guards are read-only by construction: a POST_TX frame is static, and APPROVE inside a
/// POST_TX frame exceptional-halts. Their only external interaction is a STATICCALL to the
/// introspection shim.
///
/// A guard reverts with a named custom error when its invariant is violated, which
/// invalidates the entire transaction. Note that the revert reason is NOT observable on the
/// devnet: an invalidated transaction has no receipt, and simulation misattributes a POST_TX
/// revert to the VERIFY frame (see GATE-RESULTS.md). Which assertion fires is therefore
/// established by the local forge tests against a mocked introspection source; on-chain the
/// evidence is that the transaction never mines and victim state is unchanged.

// ---------------------------------------------------------------------------
// TXTRACE / TXDIFF parameter ids, named once so the guards read declaratively.
// ---------------------------------------------------------------------------
library P {
    uint256 internal constant BAL_CHANGE_COUNT = 0x00;
    uint256 internal constant SLOT_CHANGE_COUNT = 0x01;
    uint256 internal constant BAL_CHANGE_ADDR = 0x03;
    uint256 internal constant SLOT_CHANGE_ADDR = 0x06;
    uint256 internal constant SLOT_CHANGE_KEY = 0x07;
    uint256 internal constant EVENT_COUNT = 0x0C;
    uint256 internal constant EVENT_ADDR = 0x0D;
    uint256 internal constant EVENT_TOPIC0 = 0x0F;
    uint256 internal constant EVENT_TOPIC1 = 0x10;
    uint256 internal constant EVENT_TOPIC2 = 0x11;

    uint256 internal constant SLOT_BEFORE = 0x00;
    uint256 internal constant SLOT_AFTER = 0x01;
    uint256 internal constant BALANCE_BEFORE = 0x02;
    uint256 internal constant BALANCE_AFTER = 0x03;
    uint256 internal constant CODEHASH_AFTER = 0x05;
}

/// @notice Storage-slot derivation for the mock ERC-20 layout, done on-chain so callers
///         pass semantic arguments rather than precomputed slots.
library Slots {
    function balanceSlot(address owner, uint256 base) internal pure returns (uint256) {
        return uint256(keccak256(abi.encode(owner, base)));
    }

    function allowanceSlot(address owner, address spender, uint256 base)
        internal pure returns (uint256)
    {
        return uint256(keccak256(abi.encode(spender, keccak256(abi.encode(owner, base)))));
    }
}

/// @notice Asserts the transaction granted no ERC-20 approval on `owner`'s behalf outside
///         an allowlist. Defends the "harmless action that secretly approves" pattern.
contract AssertNoApproval {
    bytes32 private constant APPROVAL_TOPIC =
        keccak256("Approval(address,address,uint256)");

    error UnexpectedApproval(address token, address spender);

    /// @param allowedSpenders spenders the owner legitimately intended to approve; pass an
    ///        empty array for strict mode (no approval by this owner at all).
    function assertNoApprovalOutside(
        ITxIntrospection shim,
        address owner,
        address[] calldata allowedSpenders
    ) external view {
        uint256 n = shim.txtrace(P.EVENT_COUNT, 0);
        for (uint256 i = 0; i < n; i++) {
            if (bytes32(shim.txtrace(P.EVENT_TOPIC0, i)) != APPROVAL_TOPIC) continue;
            if (address(uint160(shim.txtrace(P.EVENT_TOPIC1, i))) != owner) continue;

            address spender = address(uint160(shim.txtrace(P.EVENT_TOPIC2, i)));
            bool allowed = false;
            for (uint256 j = 0; j < allowedSpenders.length; j++) {
                if (allowedSpenders[j] == spender) { allowed = true; break; }
            }
            if (!allowed) {
                revert UnexpectedApproval(address(uint160(shim.txtrace(P.EVENT_ADDR, i))), spender);
            }
        }
    }
}

/// @notice Asserts no ERC-20 allowance from `owner` to `spender` survives the transaction.
///         This is what makes an allowance-free flow enforceable: approve exactly what is
///         needed, use it, and prove nothing is left for a later drain.
contract AssertAllowanceZero {
    error ResidualAllowance(uint256 remaining);

    function assertZero(
        ITxIntrospection shim,
        address token,
        address owner,
        address spender,
        uint256 allowanceBaseSlot
    ) external view {
        uint256 slot = Slots.allowanceSlot(owner, spender, allowanceBaseSlot);
        uint256 remaining = shim.txdiff(P.SLOT_AFTER, token, slot);
        if (remaining != 0) revert ResidualAllowance(remaining);
    }
}

/// @notice Asserts a storage slot was not modified BY THIS TRANSACTION.
///
/// @dev CORRECT ONLY for adversarial writes inside the guarded transaction. TXDIFF's
///      "before" is the transaction prestate, so a slot this transaction never wrote reads
///      the same value both ways — a change made by an EARLIER transaction passes silently.
///      For that case use AssertSlotEquals.
contract AssertSlotUnchanged {
    error SlotChanged(address account, uint256 slot, uint256 before_, uint256 after_);

    function assertUnchanged(ITxIntrospection shim, address account, uint256 slot) external view {
        uint256 b = shim.txdiff(P.SLOT_BEFORE, account, slot);
        uint256 a = shim.txdiff(P.SLOT_AFTER, account, slot);
        if (b != a) revert SlotChanged(account, slot, b, a);
    }
}

/// @notice Asserts a storage slot equals a value committed at signing time.
///
/// @dev Required whenever the adversarial change happened in a PRIOR transaction, where a
///      differential comparison reads identical values on both sides and passes.
contract AssertSlotEquals {
    error SlotMismatch(address account, uint256 slot, uint256 expected, uint256 actual);
    error CodeHashMismatch(address account, uint256 expected, uint256 actual);

    function assertSlotEquals(
        ITxIntrospection shim,
        address account,
        uint256 slot,
        uint256 expected
    ) external view {
        uint256 actual = shim.txdiff(P.SLOT_AFTER, account, slot);
        if (actual != expected) revert SlotMismatch(account, slot, expected, actual);
    }

    /// @dev Note for proxies: the code hash of the PROXY never changes on an upgrade. Point
    ///      this at the implementation address, or assert the implementation slot instead.
    function assertCodeHashEquals(ITxIntrospection shim, address account, uint256 expected)
        external view
    {
        uint256 actual = shim.txdiff(P.CODEHASH_AFTER, account, 0);
        if (actual != expected) revert CodeHashMismatch(account, expected, actual);
    }
}

/// @notice Bounds the transaction's net effect on one account: a minimum received and a
///         maximum spent, both committed at signing time. Defends sandwiching and any
///         execution that diverges from the quote the user was shown.
contract AssertNetEffect {
    error OutputBelowMinimum(uint256 got, uint256 min);
    error InputAboveMaximum(uint256 spent, uint256 max);

    function assertTokenDelta(
        ITxIntrospection shim,
        address token,
        address account,
        uint256 balanceBaseSlot,
        uint256 minReceived,
        uint256 maxSpent
    ) external view {
        uint256 slot = Slots.balanceSlot(account, balanceBaseSlot);
        uint256 b = shim.txdiff(P.SLOT_BEFORE, token, slot);
        uint256 a = shim.txdiff(P.SLOT_AFTER, token, slot);
        if (a >= b) {
            if (a - b < minReceived) revert OutputBelowMinimum(a - b, minReceived);
        } else {
            if (b - a > maxSpent) revert InputAboveMaximum(b - a, maxSpent);
            if (minReceived != 0) revert OutputBelowMinimum(0, minReceived);
        }
    }
}

/// @notice Asserts every account whose balance changed is the sender or allowlisted.
///         Defends transactions that quietly move value to an extra counterparty.
contract AssertCounterparties {
    error UnexpectedCounterparty(address account);

    function assertOnly(ITxIntrospection shim, address[] calldata allowed) external view {
        uint256 n = shim.txtrace(P.BAL_CHANGE_COUNT, 0);
        for (uint256 i = 0; i < n; i++) {
            address acct = address(uint160(shim.txtrace(P.BAL_CHANGE_ADDR, i)));
            bool ok = false;
            for (uint256 j = 0; j < allowed.length; j++) {
                if (allowed[j] == acct) { ok = true; break; }
            }
            if (!ok) revert UnexpectedCounterparty(acct);
        }
    }
}

/// @notice Deny-by-default over storage writes: every written slot must be allowlisted.
///         Required where the adversarial slot cannot be enumerated in advance, such as a
///         mapping keyed by an attacker-chosen address.
contract AssertNoUnexpectedWrites {
    error UnexpectedWrite(address account, uint256 slot);

    function assertOnly(ITxIntrospection shim, bytes32[] calldata allowedFingerprints)
        external view
    {
        uint256 n = shim.txtrace(P.SLOT_CHANGE_COUNT, 0);
        for (uint256 i = 0; i < n; i++) {
            address acct = address(uint160(shim.txtrace(P.SLOT_CHANGE_ADDR, i)));
            uint256 slot = shim.txtrace(P.SLOT_CHANGE_KEY, i);
            bytes32 fp = keccak256(abi.encodePacked(acct, slot));
            bool ok = false;
            for (uint256 j = 0; j < allowedFingerprints.length; j++) {
                if (allowedFingerprints[j] == fp) { ok = true; break; }
            }
            if (!ok) revert UnexpectedWrite(acct, slot);
        }
    }
}
