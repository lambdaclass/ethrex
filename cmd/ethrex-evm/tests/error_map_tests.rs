//! Unit tests for `statetest::error_map::vm_error_to_geth_string`.
//!
//! One assertion per mapped variant, verifying the exact geth error string.

use ethrex_evm::statetest::error_map::vm_error_to_geth_string;
use ethrex_levm::errors::{ExceptionalHalt, VMError};

#[test]
fn revert_opcode_maps_to_execution_reverted() {
    let err = VMError::RevertOpcode;
    assert_eq!(vm_error_to_geth_string(&err), "execution reverted");
}

#[test]
fn out_of_gas_maps_correctly() {
    let err = VMError::ExceptionalHalt(ExceptionalHalt::OutOfGas);
    assert_eq!(vm_error_to_geth_string(&err), "out of gas");
}

#[test]
fn stack_underflow_uses_display() {
    let err = VMError::ExceptionalHalt(ExceptionalHalt::StackUnderflow {
        stack_len: 0,
        required: 2,
    });
    let s = vm_error_to_geth_string(&err);
    // Phase 4a made Display geth-compatible: "stack underflow (N <=> M)"
    assert!(
        s.contains("stack underflow"),
        "expected 'stack underflow' in: {s}"
    );
    assert!(s.contains("0"), "expected stack_len in: {s}");
    assert!(s.contains("2"), "expected required in: {s}");
}

#[test]
fn stack_overflow_uses_display() {
    let err = VMError::ExceptionalHalt(ExceptionalHalt::StackOverflow {
        stack_len: 1024,
        limit: 1024,
    });
    let s = vm_error_to_geth_string(&err);
    // Phase 4a Display: "stack limit reached L (N)"
    assert!(
        s.contains("stack limit reached"),
        "expected 'stack limit reached' in: {s}"
    );
}

#[test]
fn invalid_jump_maps_correctly() {
    let err = VMError::ExceptionalHalt(ExceptionalHalt::InvalidJump);
    assert_eq!(vm_error_to_geth_string(&err), "invalid jump destination");
}

#[test]
fn static_context_maps_correctly() {
    let err = VMError::ExceptionalHalt(ExceptionalHalt::OpcodeNotAllowedInStaticContext);
    assert_eq!(vm_error_to_geth_string(&err), "write protection");
}

#[test]
fn invalid_contract_prefix_maps_correctly() {
    let err = VMError::ExceptionalHalt(ExceptionalHalt::InvalidContractPrefix);
    assert_eq!(
        vm_error_to_geth_string(&err),
        "invalid code: must not begin with 0xef"
    );
}

#[test]
fn invalid_opcode_maps_correctly() {
    let err = VMError::ExceptionalHalt(ExceptionalHalt::InvalidOpcode);
    let s = vm_error_to_geth_string(&err);
    assert!(
        s.contains("invalid opcode"),
        "expected 'invalid opcode' in: {s}"
    );
}

#[test]
fn address_already_occupied_maps_correctly() {
    let err = VMError::ExceptionalHalt(ExceptionalHalt::AddressAlreadyOccupied);
    assert_eq!(vm_error_to_geth_string(&err), "contract address collision");
}

#[test]
fn contract_output_too_big_maps_correctly() {
    let err = VMError::ExceptionalHalt(ExceptionalHalt::ContractOutputTooBig);
    assert_eq!(vm_error_to_geth_string(&err), "max code size exceeded");
}

#[test]
fn out_of_bounds_maps_correctly() {
    let err = VMError::ExceptionalHalt(ExceptionalHalt::OutOfBounds);
    assert_eq!(vm_error_to_geth_string(&err), "return data out of bounds");
}

#[test]
fn very_large_number_maps_correctly() {
    let err = VMError::ExceptionalHalt(ExceptionalHalt::VeryLargeNumber);
    assert_eq!(vm_error_to_geth_string(&err), "gas uint64 overflow");
}
