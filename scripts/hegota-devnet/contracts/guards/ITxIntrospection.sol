// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title ITxIntrospection
/// @notice Interface to the Yul shim that exposes the EIP-7906 transaction-introspection
///         opcodes. Guards depend on this interface rather than emitting opcodes, because
///         `verbatim_*` is unavailable inside Solidity inline assembly.
///
/// Parameter ids follow the ethrex implementation.
///
/// TXTRACE (`txtrace`) — transaction-wide effect diff:
///   0x00 count of balance changes            (in2 must be 0)
///   0x01 count of storage-slot changes       (in2 must be 0)
///   0x02 count of deployed contracts         (in2 must be 0)
///   0x03/0x04/0x05  balance change[in2]: account / before / after
///   0x06/0x07/0x08/0x09  slot change[in2]: account / slot / before / after
///   0x0A/0x0B  deployed[in2]: address / code hash
///   0x0C count of emitted events             (in2 must be 0)
///   0x0D event[in2] emitting address
///   0x0E event[in2] topic count
///   0x0F/0x10/0x11/0x12 event[in2] topic0..topic3 (halts if the topic is absent)
///   0x13 event[in2] data length
///   0x14 gas pre-charge (the maximum cost debited from the payer)
///   0x15 gas payer
///
/// TXDIFF (`txdiff`) — keyed before/after lookup for one account:
///   0x00 slot_before / 0x01 slot_after        (in3 = slot key)
///   0x02 balance_before / 0x03 balance_after  (in3 must be 0)
///   0x04 codehash_before / 0x05 codehash_after (in3 must be 0)
///
/// IMPORTANT — "before" is the transaction prestate and "after" is live post-body state,
/// so the comparison is scoped to THIS transaction. A slot this transaction never wrote
/// reads the same value both ways. Detecting a change made by an EARLIER transaction
/// therefore requires an absolute assertion against a value committed at signing time,
/// not a before/after comparison.
interface ITxIntrospection {
    function txtrace(uint256 param, uint256 in2) external view returns (uint256);

    function txdiff(uint256 param, address account, uint256 in3) external view returns (uint256);

    function eventData(uint256 index, uint256 offset, uint256 length)
        external
        view
        returns (bytes memory);
}
