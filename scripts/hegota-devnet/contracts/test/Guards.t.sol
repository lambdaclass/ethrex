// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Test} from "forge-std/Test.sol";
import {MockTxIntrospection} from "../mocks/MockTxIntrospection.sol";
import {ITxIntrospection} from "../guards/ITxIntrospection.sol";
import {
    AssertNoApproval,
    AssertAllowanceZero,
    AssertSlotUnchanged,
    AssertSlotEquals,
    AssertNetEffect,
    AssertCounterparties,
    AssertNoUnexpectedWrites
} from "../guards/Guards.sol";

/// @notice Offline coverage of guard LOGIC against scripted diff data.
///
/// The devnet scenarios prove each guard discriminates on one realistic case. These tests
/// cover what a devnet round-trip is too slow and too coarse to reach: empty diffs, boundary
/// indices, allowlist hits and misses, and storage-slot derivation. Both matter — a guard can
/// be logically correct here and still be defeated on-chain by the divergence recorded in
/// pocs/GATE-RESULTS.md.
contract GuardsTest is Test {
    MockTxIntrospection shim;

    address constant TOKEN = address(0x7);
    address constant OWNER = address(0xA11CE);
    address constant SPENDER = address(0xB0B);
    address constant ATTACKER = address(0xBAD);

    // TXTRACE / TXDIFF parameter ids
    uint256 constant BAL_CHANGE_COUNT = 0x00;
    uint256 constant SLOT_CHANGE_COUNT = 0x01;
    uint256 constant BAL_CHANGE_ADDR = 0x03;
    uint256 constant SLOT_CHANGE_ADDR = 0x06;
    uint256 constant SLOT_CHANGE_KEY = 0x07;
    uint256 constant EVENT_COUNT = 0x0C;
    uint256 constant EVENT_ADDR = 0x0D;
    uint256 constant EVENT_TOPIC0 = 0x0F;
    uint256 constant EVENT_TOPIC1 = 0x10;
    uint256 constant EVENT_TOPIC2 = 0x11;
    uint256 constant SLOT_BEFORE = 0x00;
    uint256 constant SLOT_AFTER = 0x01;

    bytes32 constant APPROVAL_TOPIC = keccak256("Approval(address,address,uint256)");

    function setUp() public {
        shim = new MockTxIntrospection();
    }

    function allowanceSlot(address owner, address spender, uint256 base)
        internal pure returns (uint256)
    {
        return uint256(keccak256(abi.encode(spender, keccak256(abi.encode(owner, base)))));
    }

    function balanceSlot(address owner, uint256 base) internal pure returns (uint256) {
        return uint256(keccak256(abi.encode(owner, base)));
    }

    // ---------------------------------------------------------------- AssertNoApproval

    function _scriptApproval(address owner, address spender) internal {
        shim.setTrace(EVENT_COUNT, 0, 1);
        shim.setTrace(EVENT_ADDR, 0, uint256(uint160(TOKEN)));
        shim.setTrace(EVENT_TOPIC0, 0, uint256(APPROVAL_TOPIC));
        shim.setTrace(EVENT_TOPIC1, 0, uint256(uint160(owner)));
        shim.setTrace(EVENT_TOPIC2, 0, uint256(uint160(spender)));
    }

    function test_NoApproval_strictRejectsAnyApproval() public {
        AssertNoApproval g = new AssertNoApproval();
        _scriptApproval(OWNER, ATTACKER);
        address[] memory none = new address[](0);
        vm.expectRevert();
        g.assertNoApprovalOutside(ITxIntrospection(address(shim)), OWNER, none);
    }

    function test_NoApproval_allowlistedSpenderPasses() public {
        AssertNoApproval g = new AssertNoApproval();
        _scriptApproval(OWNER, SPENDER);
        address[] memory ok = new address[](1);
        ok[0] = SPENDER;
        g.assertNoApprovalOutside(ITxIntrospection(address(shim)), OWNER, ok);
    }

    function test_NoApproval_ignoresApprovalsByOtherOwners() public {
        AssertNoApproval g = new AssertNoApproval();
        _scriptApproval(ATTACKER, ATTACKER);   // someone else's approval
        address[] memory none = new address[](0);
        g.assertNoApprovalOutside(ITxIntrospection(address(shim)), OWNER, none);
    }

    function test_NoApproval_emptyEventSetPasses() public {
        AssertNoApproval g = new AssertNoApproval();
        address[] memory none = new address[](0);
        g.assertNoApprovalOutside(ITxIntrospection(address(shim)), OWNER, none);
    }

    // ---------------------------------------------------------------- AssertAllowanceZero

    function test_AllowanceZero_passesWhenConsumed() public {
        AssertAllowanceZero g = new AssertAllowanceZero();
        shim.setDiff(SLOT_AFTER, TOKEN, allowanceSlot(OWNER, SPENDER, 2), 0);
        g.assertZero(ITxIntrospection(address(shim)), TOKEN, OWNER, SPENDER, 2);
    }

    function test_AllowanceZero_revertsOnResidual() public {
        AssertAllowanceZero g = new AssertAllowanceZero();
        shim.setDiff(SLOT_AFTER, TOKEN, allowanceSlot(OWNER, SPENDER, 2), 400);
        vm.expectRevert();
        g.assertZero(ITxIntrospection(address(shim)), TOKEN, OWNER, SPENDER, 2);
    }

    function test_AllowanceZero_derivesTheRightSlot() public {
        // A residual recorded at a DIFFERENT pair's slot must not trip this assertion.
        AssertAllowanceZero g = new AssertAllowanceZero();
        shim.setDiff(SLOT_AFTER, TOKEN, allowanceSlot(OWNER, ATTACKER, 2), 999);
        g.assertZero(ITxIntrospection(address(shim)), TOKEN, OWNER, SPENDER, 2);
    }

    // ------------------------------------------------- AssertSlotUnchanged / AssertSlotEquals

    function test_SlotUnchanged_revertsWhenChangedInThisTx() public {
        AssertSlotUnchanged g = new AssertSlotUnchanged();
        shim.setDiff(SLOT_BEFORE, TOKEN, 0, 1);
        shim.setDiff(SLOT_AFTER, TOKEN, 0, 2);
        vm.expectRevert();
        g.assertUnchanged(ITxIntrospection(address(shim)), TOKEN, 0);
    }

    /// @dev Pins the documented limitation: a change made by an EARLIER transaction reads the
    ///      same value on both sides, so the differential form passes. This is the behaviour
    ///      the proxy-swap and oracle scenarios demonstrate on-chain.
    function test_SlotUnchanged_passesForPriorTransactionChange() public {
        AssertSlotUnchanged g = new AssertSlotUnchanged();
        shim.setDiff(SLOT_BEFORE, TOKEN, 0, 42);
        shim.setDiff(SLOT_AFTER, TOKEN, 0, 42);
        g.assertUnchanged(ITxIntrospection(address(shim)), TOKEN, 0);
    }

    function test_SlotEquals_catchesWhatDifferentialMisses() public {
        AssertSlotEquals g = new AssertSlotEquals();
        shim.setDiff(SLOT_AFTER, TOKEN, 0, 42);       // hostile value, unchanged within the tx
        vm.expectRevert();
        g.assertSlotEquals(ITxIntrospection(address(shim)), TOKEN, 0, 7);   // committed value
    }

    function test_SlotEquals_passesOnExpectedValue() public {
        AssertSlotEquals g = new AssertSlotEquals();
        shim.setDiff(SLOT_AFTER, TOKEN, 0, 7);
        g.assertSlotEquals(ITxIntrospection(address(shim)), TOKEN, 0, 7);
    }

    // ---------------------------------------------------------------- AssertNetEffect

    function test_NetEffect_revertsBelowMinimum() public {
        AssertNetEffect g = new AssertNetEffect();
        uint256 slot = balanceSlot(OWNER, 1);
        shim.setDiff(SLOT_BEFORE, TOKEN, slot, 100);
        shim.setDiff(SLOT_AFTER, TOKEN, slot, 150);        // received 50
        vm.expectRevert();
        g.assertTokenDelta(ITxIntrospection(address(shim)), TOKEN, OWNER, 1, 95, type(uint256).max);
    }

    function test_NetEffect_passesAtExactlyTheMinimum() public {
        AssertNetEffect g = new AssertNetEffect();
        uint256 slot = balanceSlot(OWNER, 1);
        shim.setDiff(SLOT_BEFORE, TOKEN, slot, 100);
        shim.setDiff(SLOT_AFTER, TOKEN, slot, 195);        // received exactly 95
        g.assertTokenDelta(ITxIntrospection(address(shim)), TOKEN, OWNER, 1, 95, type(uint256).max);
    }

    function test_NetEffect_revertsAboveMaxSpent() public {
        AssertNetEffect g = new AssertNetEffect();
        uint256 slot = balanceSlot(OWNER, 1);
        shim.setDiff(SLOT_BEFORE, TOKEN, slot, 100);
        shim.setDiff(SLOT_AFTER, TOKEN, slot, 40);         // spent 60
        vm.expectRevert();
        g.assertTokenDelta(ITxIntrospection(address(shim)), TOKEN, OWNER, 1, 0, 50);
    }

    // ------------------------------------------------- allowlist guards, incl. empty diffs

    function test_Counterparties_revertsOnUnlistedAccount() public {
        AssertCounterparties g = new AssertCounterparties();
        shim.setTrace(BAL_CHANGE_COUNT, 0, 1);
        shim.setTrace(BAL_CHANGE_ADDR, 0, uint256(uint160(ATTACKER)));
        address[] memory allowed = new address[](1);
        allowed[0] = SPENDER;
        vm.expectRevert();
        g.assertOnly(ITxIntrospection(address(shim)), allowed);
    }

    function test_Counterparties_emptyDiffPasses() public {
        AssertCounterparties g = new AssertCounterparties();
        address[] memory allowed = new address[](0);
        g.assertOnly(ITxIntrospection(address(shim)), allowed);
    }

    function test_NoUnexpectedWrites_revertsOnUnlistedSlot() public {
        AssertNoUnexpectedWrites g = new AssertNoUnexpectedWrites();
        shim.setTrace(SLOT_CHANGE_COUNT, 0, 1);
        shim.setTrace(SLOT_CHANGE_ADDR, 0, uint256(uint160(TOKEN)));
        shim.setTrace(SLOT_CHANGE_KEY, 0, 99);
        bytes32[] memory allowed = new bytes32[](1);
        allowed[0] = keccak256(abi.encodePacked(TOKEN, uint256(1)));
        vm.expectRevert();
        g.assertOnly(ITxIntrospection(address(shim)), allowed);
    }

    function test_NoUnexpectedWrites_passesWhenAllListed() public {
        AssertNoUnexpectedWrites g = new AssertNoUnexpectedWrites();
        shim.setTrace(SLOT_CHANGE_COUNT, 0, 2);
        shim.setTrace(SLOT_CHANGE_ADDR, 0, uint256(uint160(TOKEN)));
        shim.setTrace(SLOT_CHANGE_KEY, 0, 1);
        shim.setTrace(SLOT_CHANGE_ADDR, 1, uint256(uint160(TOKEN)));
        shim.setTrace(SLOT_CHANGE_KEY, 1, 2);
        bytes32[] memory allowed = new bytes32[](2);
        allowed[0] = keccak256(abi.encodePacked(TOKEN, uint256(1)));
        allowed[1] = keccak256(abi.encodePacked(TOKEN, uint256(2)));
        g.assertOnly(ITxIntrospection(address(shim)), allowed);
    }

    /// @dev The last enumerated index must still be checked; an off-by-one in the loop bound
    ///      would let the final change through, which is exactly where an attacker would hide.
    function test_NoUnexpectedWrites_checksTheLastIndex() public {
        AssertNoUnexpectedWrites g = new AssertNoUnexpectedWrites();
        shim.setTrace(SLOT_CHANGE_COUNT, 0, 2);
        shim.setTrace(SLOT_CHANGE_ADDR, 0, uint256(uint160(TOKEN)));
        shim.setTrace(SLOT_CHANGE_KEY, 0, 1);
        shim.setTrace(SLOT_CHANGE_ADDR, 1, uint256(uint160(ATTACKER)));   // hidden in last slot
        shim.setTrace(SLOT_CHANGE_KEY, 1, 5);
        bytes32[] memory allowed = new bytes32[](1);
        allowed[0] = keccak256(abi.encodePacked(TOKEN, uint256(1)));
        vm.expectRevert();
        g.assertOnly(ITxIntrospection(address(shim)), allowed);
    }
}
