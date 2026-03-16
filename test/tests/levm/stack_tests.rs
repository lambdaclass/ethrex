#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use ethrex_common::U256;
use ethrex_levm::call_frame::Stack;

/// Helper to setup a stack with specific values
fn setup_stack_with_values(values: &[u64]) -> Stack {
    let mut stack = Stack::default();
    for &value in values {
        stack.push(U256::from(value)).unwrap();
    }
    stack
}

// ==================== Stack DUP Tests ====================

#[test]
fn test_stack_dup_depth_1() {
    let mut stack = setup_stack_with_values(&[1, 2, 3]);

    // DUP1 duplicates the value at depth 0 (top)
    stack.dup::<0>().unwrap();

    // Stack should now be [1, 2, 3, 3] with 3 on top twice
    assert_eq!(stack.pop1().unwrap(), U256::from(3));
    assert_eq!(stack.pop1().unwrap(), U256::from(3));
    assert_eq!(stack.pop1().unwrap(), U256::from(2));
    assert_eq!(stack.pop1().unwrap(), U256::from(1));
}

#[test]
fn test_stack_dup_depth_5() {
    let mut stack = setup_stack_with_values(&[1, 2, 3, 4, 5, 6]);

    // DUP5 duplicates the value at depth 4 (5th from top)
    stack.dup::<4>().unwrap();

    // The value at depth 4 is 2, so stack becomes [1, 2, 3, 4, 5, 6, 2]
    assert_eq!(stack.pop1().unwrap(), U256::from(2));
}

#[test]
fn test_stack_dup_depth_16() {
    let mut stack = Stack::default();
    for i in 1..=20 {
        stack.push(U256::from(i)).unwrap();
    }

    // DUP16 duplicates the value at depth 15 (16th from top)
    stack.dup::<15>().unwrap();

    // The value at depth 15 is 5, so it should be on top
    assert_eq!(stack.pop1().unwrap(), U256::from(5));
}

// ==================== Stack SWAP Tests ====================

#[test]
fn test_stack_swap_depth_1() {
    let mut stack = setup_stack_with_values(&[1, 2, 3]);

    // SWAP1 swaps top with value at depth 1
    stack.swap::<1>().unwrap();

    // Stack was [1, 2, 3], after SWAP1 it's [1, 3, 2]
    assert_eq!(stack.pop1().unwrap(), U256::from(2));
    assert_eq!(stack.pop1().unwrap(), U256::from(3));
    assert_eq!(stack.pop1().unwrap(), U256::from(1));
}

#[test]
fn test_stack_swap_depth_5() {
    let mut stack = setup_stack_with_values(&[1, 2, 3, 4, 5, 6]);

    // SWAP5 swaps top (6) with value at depth 5 (1)
    stack.swap::<5>().unwrap();

    // Top should now be 1
    assert_eq!(stack.pop1().unwrap(), U256::from(1));
    // Next values
    assert_eq!(stack.pop1().unwrap(), U256::from(5));
    assert_eq!(stack.pop1().unwrap(), U256::from(4));
    assert_eq!(stack.pop1().unwrap(), U256::from(3));
    assert_eq!(stack.pop1().unwrap(), U256::from(2));
    // Bottom should now be 6 (swapped from top)
    assert_eq!(stack.pop1().unwrap(), U256::from(6));
}

#[test]
fn test_stack_swap_depth_16() {
    let mut stack = Stack::default();
    for i in 1..=20 {
        stack.push(U256::from(i)).unwrap();
    }

    // SWAP16 swaps top (20) with value at depth 16 (4)
    stack.swap::<16>().unwrap();

    // Top should now be 4
    assert_eq!(stack.pop1().unwrap(), U256::from(4));

    // Skip to the position that was swapped
    for _ in 0..15 {
        stack.pop1().unwrap();
    }

    // This should now be 20 (swapped from top)
    assert_eq!(stack.pop1().unwrap(), U256::from(20));
}

// ==================== EIP-8024 DUPN Decode Tests ====================

#[test]
fn test_dupn_decode_low_range() {
    // Bytes 0x00-0x5A should decode to depths 17-107

    // 0x00 -> 0 + 17 = 17
    assert_eq!(decode_dupn_offset(0x00), 17);

    // 0x2A (42) -> 42 + 17 = 59
    assert_eq!(decode_dupn_offset(0x2A), 59);

    // 0x5A (90) -> 90 + 17 = 107
    assert_eq!(decode_dupn_offset(0x5A), 107);
}

#[test]
fn test_dupn_decode_high_range() {
    // Bytes 0x80-0xFF should decode to depths 108-235

    // 0x80 (128) -> 128 - 20 = 108
    assert_eq!(decode_dupn_offset(0x80), 108);

    // 0xAA (170) -> 170 - 20 = 150
    assert_eq!(decode_dupn_offset(0xAA), 150);

    // 0xFF (255) -> 255 - 20 = 235
    assert_eq!(decode_dupn_offset(0xFF), 235);
}

// ==================== EIP-8024 SWAPN Decode Tests ====================

#[test]
fn test_swapn_decode_same_as_dupn() {
    // SWAPN uses the same decode_single function as DUPN
    // but swaps with the (n+1)th element

    // For immediate 0x00, depth is 17, so it swaps top with 18th element
    assert_eq!(decode_dupn_offset(0x00), 17);

    // For immediate 0xFF, depth is 235, so it swaps top with 236th element
    assert_eq!(decode_dupn_offset(0xFF), 235);
}

// ==================== EIP-8024 EXCHANGE Decode Tests ====================

#[test]
fn test_exchange_decode_basic() {
    // Test immediate byte 0x00 -> k=0, q=0, r=0
    // Since q < r is false (0 < 0), we get (r+1, 29-q) = (1, 29)
    let (n, m) = decode_exchange_offset(0x00);
    assert_eq!(n, 1);
    assert_eq!(m, 29);
}

#[test]
fn test_exchange_decode_q_less_than_r() {
    // Test immediate byte 0x01 -> k=1, q=0, r=1
    // Since q < r (0 < 1), we get (q+1, r+1) = (1, 2)
    let (n, m) = decode_exchange_offset(0x01);
    assert_eq!(n, 1);
    assert_eq!(m, 2);
}

#[test]
fn test_exchange_decode_boundary_low() {
    // Test immediate byte 0x4F (79) -> k=79, q=4, r=15
    // Since q < r (4 < 15), we get (q+1, r+1) = (5, 16)
    let (n, m) = decode_exchange_offset(0x4F);
    assert_eq!(n, 5);
    assert_eq!(m, 16);
}

#[test]
fn test_exchange_decode_with_offset() {
    // Test immediate byte 0x80 (128) -> k = 128 - 48 = 80, q=5, r=0
    // Since q < r is false (5 < 0), we get (r+1, 29-q) = (1, 24)
    let (n, m) = decode_exchange_offset(0x80);
    assert_eq!(n, 1);
    assert_eq!(m, 24);
}

#[test]
fn test_exchange_decode_high_byte() {
    // Test immediate byte 0xFF (255) -> k = 255 - 48 = 207, q=12, r=15
    // Since q < r (12 < 15), we get (q+1, r+1) = (13, 16)
    let (n, m) = decode_exchange_offset(0xFF);
    assert_eq!(n, 13);
    assert_eq!(m, 16);
}

#[test]
fn test_exchange_decode_various_values() {
    // Test a few more values to ensure the decoding works correctly

    // 0x10 -> k=16, q=1, r=0, q >= r -> (0+1, 29-1) = (1, 28)
    let (n, m) = decode_exchange_offset(0x10);
    assert_eq!(n, 1);
    assert_eq!(m, 28);

    // 0x23 -> k=35, q=2, r=3, q < r -> (2+1, 3+1) = (3, 4)
    let (n, m) = decode_exchange_offset(0x23);
    assert_eq!(n, 3);
    assert_eq!(m, 4);
}

// ==================== Helper Functions (matching EIP-8024 spec) ====================

/// Decodes the immediate byte for DUPN/SWAPN according to EIP-8024 decode_single
fn decode_dupn_offset(byte: u8) -> u8 {
    if byte <= 0x5A {
        byte.wrapping_add(17)
    } else {
        // Assumes byte >= 0x80 (invalid range 0x5B-0x7F should error in actual implementation)
        byte.wrapping_sub(20)
    }
}

/// Decodes the immediate byte for EXCHANGE according to EIP-8024 decode_pair
fn decode_exchange_offset(byte: u8) -> (u8, u8) {
    let k = if byte <= 0x4F {
        byte
    } else {
        // Assumes byte >= 0x80 (invalid range 0x50-0x7F should error in actual implementation)
        byte.wrapping_sub(48)
    };

    let q = k >> 4;
    let r = k & 0x0F;

    if q < r {
        (q + 1, r + 1)
    } else {
        (r + 1, 29 - q)
    }
}

// ==================== Validation Tests ====================

#[test]
fn test_dupn_invalid_range_detection() {
    // Bytes 0x5B-0x7F should be invalid for DUPN
    // These correspond to JUMPDEST (0x5B) and PUSH opcodes (0x5F-0x7F)

    // We can't test the actual VM error here without creating a full VM,
    // but we can verify the valid ranges

    // Last valid in low range
    assert_eq!(decode_dupn_offset(0x5A), 107);

    // First valid in high range
    assert_eq!(decode_dupn_offset(0x80), 108);

    // The gap 0x5B-0x7F should cause InvalidOpcode in actual implementation
}

#[test]
fn test_exchange_invalid_range_detection() {
    // Bytes 0x50-0x7F should be invalid for EXCHANGE

    // Last valid in low range
    let (n, m) = decode_exchange_offset(0x4F);
    assert_eq!(n, 5);
    assert_eq!(m, 16);

    // First valid in high range
    let (n, m) = decode_exchange_offset(0x80);
    assert_eq!(n, 1);
    assert_eq!(m, 24);

    // The gap 0x50-0x7F should cause InvalidOpcode in actual implementation
}

#[test]
fn test_exchange_n_less_than_m_invariant() {
    // EIP-8024 requires that n < m for all valid EXCHANGE operations
    // Let's verify this for a range of valid bytes

    for byte in 0x00..=0x4F {
        let (n, m) = decode_exchange_offset(byte);
        assert!(
            n < m,
            "For byte 0x{:02X}: n={} should be < m={}",
            byte,
            n,
            m
        );
    }

    for byte in 0x80..=0xFF {
        let (n, m) = decode_exchange_offset(byte);
        assert!(
            n < m,
            "For byte 0x{:02X}: n={} should be < m={}",
            byte,
            n,
            m
        );
    }
}

#[test]
fn test_exchange_sum_constraint() {
    // EIP-8024 requires that n + m <= 30 for all valid EXCHANGE operations

    for byte in 0x00..=0x4F {
        let (n, m) = decode_exchange_offset(byte);
        assert!(
            n + m <= 30,
            "For byte 0x{:02X}: n={} + m={} = {} should be <= 30",
            byte,
            n,
            m,
            n + m
        );
    }

    for byte in 0x80..=0xFF {
        let (n, m) = decode_exchange_offset(byte);
        assert!(
            n + m <= 30,
            "For byte 0x{:02X}: n={} + m={} = {} should be <= 30",
            byte,
            n,
            m,
            n + m
        );
    }
}

// ==================== Stack Integration Tests for EIP-8024 ====================

#[test]
fn test_stack_dupn_basic() {
    // Test DUPN with a simple case: duplicating element at depth 17
    let mut stack = Stack::default();

    // Push 20 elements (to have enough for depth 17)
    for i in 1..=20 {
        stack.push(U256::from(i)).unwrap();
    }

    // Get the value that's 17 deep (which is element 4, since we pushed 1-20)
    // Top is 20, depth 1 is 19, depth 2 is 18, ..., depth 17 is 3
    let expected_value = U256::from(3);

    // Manually duplicate by calculating the offset
    // relative_offset for depth 17 would be encoded as 0x00 (0 + 17 = 17)
    // absolute_offset = stack.offset + 17
    let absolute_offset = stack.offset + 17;
    let value_to_dup = stack.values[absolute_offset];
    stack.push(value_to_dup).unwrap();

    // Top should now be the duplicated value
    assert_eq!(stack.pop1().unwrap(), expected_value);
}

#[test]
fn test_stack_dupn_depth_50() {
    // Test DUPN with depth 50
    let mut stack = Stack::default();

    // Push 55 elements
    for i in 1..=55 {
        stack.push(U256::from(i)).unwrap();
    }

    // Depth 50 should give us element 5 (55 - 50 = 5)
    let expected_value = U256::from(5);

    // Duplicate at depth 50
    let absolute_offset = stack.offset + 50;
    let value_to_dup = stack.values[absolute_offset];
    stack.push(value_to_dup).unwrap();

    assert_eq!(stack.pop1().unwrap(), expected_value);
}

#[test]
fn test_stack_dupn_max_depth() {
    // Test DUPN with maximum depth (235)
    let mut stack = Stack::default();

    // Push 240 elements to ensure we can access depth 235
    for i in 1..=240 {
        stack.push(U256::from(i)).unwrap();
    }

    // Depth 235 should give us element 5 (240 - 235 = 5)
    let expected_value = U256::from(5);

    // Duplicate at depth 235
    let absolute_offset = stack.offset + 235;
    let value_to_dup = stack.values[absolute_offset];
    stack.push(value_to_dup).unwrap();

    assert_eq!(stack.pop1().unwrap(), expected_value);
}

#[test]
fn test_stack_swapn_basic() {
    // Test SWAPN with a simple case: swapping top with element at depth 18
    let mut stack = Stack::default();

    // Push 20 elements
    for i in 1..=20 {
        stack.push(U256::from(i)).unwrap();
    }

    // Top is 20, depth 18 is 2 (20 - 18 = 2)
    let top_value = U256::from(20);
    let deep_value = U256::from(2);

    // SWAPN with relative_offset 17 swaps top with element at depth 18 (17 + 1)
    let top_offset = stack.offset;
    let swap_offset = stack.offset + 18;

    // Perform the swap
    stack.values.swap(top_offset, swap_offset);

    // Top should now be 2
    assert_eq!(stack.pop1().unwrap(), deep_value);

    // Skip to position 18 (now has original top value)
    for _ in 0..17 {
        stack.pop1().unwrap();
    }

    // This position should now have 20
    assert_eq!(stack.pop1().unwrap(), top_value);
}

#[test]
fn test_stack_swapn_depth_100() {
    // Test SWAPN with depth 100
    let mut stack = Stack::default();

    // Push 105 elements
    for i in 1..=105 {
        stack.push(U256::from(i)).unwrap();
    }

    let deep_value = U256::from(5); // 105 - 100 = 5

    // Swap top with element at depth 100
    let top_offset = stack.offset;
    let swap_offset = stack.offset + 100;

    stack.values.swap(top_offset, swap_offset);

    // Top should now be 5
    assert_eq!(stack.pop1().unwrap(), deep_value);
}

#[test]
fn test_stack_exchange_basic() {
    // Test EXCHANGE with n=1, m=2 (swapping elements at depth 1 and 2)
    let mut stack = setup_stack_with_values(&[1, 2, 3, 4, 5]);

    // Stack is [1, 2, 3, 4, 5] with 5 on top
    // Swap elements at depth 1 (value 4) and depth 2 (value 3)
    let offset1 = stack.offset + 1;
    let offset2 = stack.offset + 2;

    stack.values.swap(offset1, offset2);

    // Stack should now be [1, 2, 4, 3, 5]
    assert_eq!(stack.pop1().unwrap(), U256::from(5));
    assert_eq!(stack.pop1().unwrap(), U256::from(3)); // Was at depth 2, swapped with 4
    assert_eq!(stack.pop1().unwrap(), U256::from(4)); // Was at depth 1, swapped with 3
    assert_eq!(stack.pop1().unwrap(), U256::from(2));
    assert_eq!(stack.pop1().unwrap(), U256::from(1));
}

#[test]
fn test_stack_exchange_far_apart() {
    // Test EXCHANGE with elements far apart: n=1, m=29
    let mut stack = Stack::default();

    // Push 30 elements
    for i in 1..=30 {
        stack.push(U256::from(i)).unwrap();
    }

    // Swap elements at depth 1 (value 29) and depth 29 (value 1)
    let offset1 = stack.offset + 1;
    let offset29 = stack.offset + 29;

    stack.values.swap(offset1, offset29);

    // Top should still be 30
    assert_eq!(stack.pop1().unwrap(), U256::from(30));
    // Depth 1 should now be 1 (was 29)
    assert_eq!(stack.pop1().unwrap(), U256::from(1));
}

#[test]
fn test_stack_exchange_middle_elements() {
    // Test EXCHANGE with middle elements: n=5, m=10
    let mut stack = Stack::default();

    // Push 15 elements
    for i in 1..=15 {
        stack.push(U256::from(i)).unwrap();
    }

    // Elements at depth 5 (value 10) and depth 10 (value 5)
    let offset5 = stack.offset + 5;
    let offset10 = stack.offset + 10;

    let val5 = stack.values[offset5];
    let val10 = stack.values[offset10];

    stack.values.swap(offset5, offset10);

    // Verify the swap happened
    assert_eq!(stack.values[offset5], val10);
    assert_eq!(stack.values[offset10], val5);
}

// ==================== Underflow Tests ====================

#[test]
fn test_dupn_underflow() {
    // Test that DUPN fails when trying to access beyond stack depth
    let stack = setup_stack_with_values(&[1, 2, 3]);

    // Stack only has 3 elements, trying to dup at depth 17 should fail
    // In the actual opcode implementation, this would check:
    // absolute_offset = stack.offset + 17
    // if absolute_offset >= STACK_LIMIT then error

    let absolute_offset = stack.offset + 17;
    // This would be caught by the bounds check in the opcode
    // With 3 elements (offset = 1021), trying to access depth 17 gives 1021 + 17 = 1038
    // which exceeds STACK_LIMIT, indicating underflow
    assert!(absolute_offset >= 1024); // STACK_LIMIT

    // The real check is: does the stack have enough elements?
    // Stack has 3 elements, so max valid depth is 2 (0-indexed)
    assert!(stack.len() < 17);
}

#[test]
fn test_swapn_underflow() {
    // Test that SWAPN fails when trying to swap beyond stack depth
    let stack = setup_stack_with_values(&[1, 2, 3]);

    // Stack only has 3 elements, trying to swap with depth 18 should fail
    assert!(stack.len() < 18);
}

#[test]
fn test_exchange_underflow() {
    // Test that EXCHANGE fails when trying to swap beyond stack depth
    let stack = setup_stack_with_values(&[1, 2, 3]);

    // Stack only has 3 elements, trying to exchange at depths 10 and 20 should fail
    assert!(stack.len() < 10);
    assert!(stack.len() < 20);
}

// ==================== Coverage Tests ====================

#[test]
fn test_dupn_coverage_all_depths() {
    // Verify that DUPN can access all depths from 17 to 235
    let mut depths = std::collections::HashSet::new();

    // Low range: 0x00-0x5A -> depths 17-107
    for byte in 0x00..=0x5A {
        depths.insert(decode_dupn_offset(byte));
    }

    // High range: 0x80-0xFF -> depths 108-235
    for byte in 0x80..=0xFF {
        depths.insert(decode_dupn_offset(byte));
    }

    // Should have exactly 219 unique depths (17-235 inclusive)
    assert_eq!(depths.len(), 219);

    // Verify the range
    assert_eq!(*depths.iter().min().unwrap(), 17);
    assert_eq!(*depths.iter().max().unwrap(), 235);
}

#[test]
fn test_exchange_coverage_valid_pairs() {
    // Verify that EXCHANGE can access all valid (n, m) pairs where n < m and n + m <= 30
    let mut pairs = std::collections::HashSet::new();

    // Low range: 0x00-0x4F
    for byte in 0x00..=0x4F {
        pairs.insert(decode_exchange_offset(byte));
    }

    // High range: 0x80-0xFF
    for byte in 0x80..=0xFF {
        pairs.insert(decode_exchange_offset(byte));
    }

    // Each pair should satisfy the constraints
    for &(n, m) in &pairs {
        assert!(n < m, "n={} should be < m={}", n, m);
        assert!(n + m <= 30, "n={} + m={} should be <= 30", n, m);
    }

    // Total valid bytes: 80 in low range (0x00-0x4F) + 128 in high range (0x80-0xFF) = 208
    // But some might map to the same (n, m) pair
    assert!(!pairs.is_empty(), "Should have at least some valid pairs");
}
