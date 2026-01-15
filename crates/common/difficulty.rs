//! Ethereum difficulty calculation algorithms for pre-merge (PoW) blocks.
//!
//! This module implements the difficulty adjustment algorithms as specified in various EIPs:
//! - Frontier: Basic adjustment
//! - Homestead (EIP-2): Exponential difficulty adjustment
//! - Byzantium (EIP-649): Difficulty bomb delay by 3M blocks
//! - Constantinople (EIP-1234): Delay by 5M blocks total
//! - Muir Glacier (EIP-2384): Delay by 9M blocks total
//! - London (EIP-3554): Delay to block ~9.7M
//! - Arrow Glacier (EIP-4345): Delay to block ~10.7M
//! - Gray Glacier (EIP-5133): Delay to block ~11.4M

use crate::{U256, types::Fork};

/// Minimum difficulty value (prevents difficulty from going to zero)
pub const MIN_DIFFICULTY: u64 = 131072; // 2^17

/// Difficulty bound divisor - limits how much difficulty can change per block
const DIFFICULTY_BOUND_DIVISOR: u64 = 2048;

/// Block time target in seconds for difficulty adjustment
const BLOCK_TIME_TARGET: u64 = 13;

/// Bomb delay constants for each fork (in blocks)
mod bomb_delays {
    /// Byzantium: delay by 3M blocks (EIP-649)
    pub const BYZANTIUM: u64 = 3_000_000;
    /// Constantinople: delay by 5M blocks total (EIP-1234)
    pub const CONSTANTINOPLE: u64 = 5_000_000;
    /// Muir Glacier: delay by 9M blocks total (EIP-2384)
    pub const MUIR_GLACIER: u64 = 9_000_000;
    /// London: delay by 9.7M blocks (EIP-3554)
    pub const LONDON: u64 = 9_700_000;
    /// Arrow Glacier: delay by 10.7M blocks (EIP-4345)
    pub const ARROW_GLACIER: u64 = 10_700_000;
    /// Gray Glacier: delay by 11.4M blocks (EIP-5133)
    pub const GRAY_GLACIER: u64 = 11_400_000;
}

/// Calculate the expected difficulty for a block given its parent.
///
/// The difficulty adjustment algorithm varies by fork:
/// - Pre-Homestead (Frontier): Simple timestamp-based adjustment
/// - Homestead+: Exponential adjustment with difficulty bomb
///
/// # Arguments
/// * `parent_difficulty` - The difficulty of the parent block
/// * `parent_timestamp` - The timestamp of the parent block
/// * `block_timestamp` - The timestamp of the current block
/// * `block_number` - The block number of the current block
/// * `fork` - The active fork at this block
///
/// # Returns
/// The calculated difficulty for the block, never less than MIN_DIFFICULTY
pub fn calculate_difficulty(
    parent_difficulty: U256,
    parent_timestamp: u64,
    block_timestamp: u64,
    block_number: u64,
    fork: Fork,
) -> U256 {
    // Genesis block has no parent - difficulty is set in genesis config
    if block_number == 0 {
        return parent_difficulty;
    }

    // Calculate base difficulty adjustment
    let adjustment = if fork < Fork::Homestead {
        // Frontier difficulty adjustment (simpler)
        calculate_frontier_adjustment(parent_difficulty, parent_timestamp, block_timestamp)
    } else {
        // Homestead+ difficulty adjustment (EIP-2)
        calculate_homestead_adjustment(
            parent_difficulty,
            parent_timestamp,
            block_timestamp,
            fork >= Fork::Byzantium, // has_uncles parameter not needed for pure calculation
        )
    };

    // Calculate difficulty bomb component (exponential increase)
    let bomb = calculate_difficulty_bomb(block_number, fork);

    // Final difficulty = adjustment + bomb, but never below minimum
    let difficulty = adjustment.saturating_add(bomb);
    U256::max(difficulty, U256::from(MIN_DIFFICULTY))
}

/// Calculate Frontier-era difficulty adjustment (pre-Homestead).
///
/// Simple formula: if block was mined too fast, increase difficulty; if too slow, decrease.
fn calculate_frontier_adjustment(
    parent_difficulty: U256,
    parent_timestamp: u64,
    block_timestamp: u64,
) -> U256 {
    let time_diff = block_timestamp.saturating_sub(parent_timestamp);
    let bound = parent_difficulty / U256::from(DIFFICULTY_BOUND_DIVISOR);

    if time_diff < BLOCK_TIME_TARGET {
        // Block was mined fast - increase difficulty
        parent_difficulty.saturating_add(bound)
    } else {
        // Block was mined slow - decrease difficulty
        parent_difficulty.saturating_sub(bound)
    }
}

/// Calculate Homestead+ difficulty adjustment (EIP-2).
///
/// Formula:
/// ```text
/// diff = parent_diff + parent_diff // 2048 * max(1 - (block_timestamp - parent_timestamp) // 10, -99)
/// ```
///
/// For Byzantium+, the adjustment factor includes uncle consideration:
/// ```text
/// diff = parent_diff + parent_diff // 2048 * max(y - (block_timestamp - parent_timestamp) // 9, -99)
/// ```
/// where y = 2 if parent has uncles, else 1
fn calculate_homestead_adjustment(
    parent_difficulty: U256,
    parent_timestamp: u64,
    block_timestamp: u64,
    is_byzantium: bool,
) -> U256 {
    let time_diff = block_timestamp.saturating_sub(parent_timestamp);

    // Calculate the adjustment factor
    let (base_factor, time_divisor) = if is_byzantium {
        // Byzantium+: y = 2 if parent has uncles, else 1
        // For difficulty calculation without uncle info, we use 1 (conservative)
        // The actual y value would come from parent block's uncle count
        (1i64, 9u64)
    } else {
        // Homestead: simpler formula
        (1i64, 10u64)
    };

    // Calculate sigma: max(base_factor - time_diff / time_divisor, -99)
    let time_component = (time_diff / time_divisor) as i64;
    let sigma = (base_factor - time_component).max(-99);

    // Calculate adjustment: parent_diff // 2048 * sigma
    let bound = parent_difficulty / U256::from(DIFFICULTY_BOUND_DIVISOR);

    if sigma >= 0 {
        parent_difficulty.saturating_add(bound * U256::from(sigma as u64))
    } else {
        parent_difficulty.saturating_sub(bound * U256::from((-sigma) as u64))
    }
}

/// Calculate Homestead+ difficulty adjustment with uncle information (EIP-100).
///
/// This is the full formula used when we know if the parent had uncles.
/// For Byzantium+:
/// ```text
/// diff = parent_diff + parent_diff // 2048 * max(y - (block_timestamp - parent_timestamp) // 9, -99)
/// ```
/// where y = 2 if parent has uncles, else 1
pub fn calculate_homestead_adjustment_with_uncles(
    parent_difficulty: U256,
    parent_timestamp: u64,
    block_timestamp: u64,
    parent_has_uncles: bool,
    fork: Fork,
) -> U256 {
    let time_diff = block_timestamp.saturating_sub(parent_timestamp);

    // Calculate the adjustment factor based on fork
    let (base_factor, time_divisor): (i64, u64) = if fork >= Fork::Byzantium {
        // Byzantium+ (EIP-100): y = 2 if parent has uncles, else 1
        let y = if parent_has_uncles { 2 } else { 1 };
        (y, 9)
    } else if fork >= Fork::Homestead {
        // Homestead (EIP-2)
        (1, 10)
    } else {
        // Frontier - shouldn't reach here, but handle gracefully
        return calculate_frontier_adjustment(parent_difficulty, parent_timestamp, block_timestamp);
    };

    // Calculate sigma: max(base_factor - time_diff / time_divisor, -99)
    let time_component = (time_diff / time_divisor) as i64;
    let sigma = (base_factor - time_component).max(-99);

    // Calculate adjustment: parent_diff // 2048 * sigma
    let bound = parent_difficulty / U256::from(DIFFICULTY_BOUND_DIVISOR);

    if sigma >= 0 {
        parent_difficulty.saturating_add(bound * U256::from(sigma as u64))
    } else {
        parent_difficulty.saturating_sub(bound * U256::from((-sigma) as u64))
    }
}

/// Calculate the difficulty bomb component.
///
/// The difficulty bomb is an exponential increase in difficulty designed to make
/// mining increasingly difficult over time, incentivizing the switch to PoS.
///
/// Formula: 2^(period_count - 2) where period_count = (block_number - delay) / 100000
///
/// The delay varies by fork to postpone the bomb's effect.
fn calculate_difficulty_bomb(block_number: u64, fork: Fork) -> U256 {
    // Get the bomb delay for this fork
    let delay = get_bomb_delay(fork);

    // If block number is below delay, no bomb effect
    if block_number <= delay {
        return U256::zero();
    }

    // Calculate the fake block number (actual - delay)
    let fake_block_number = block_number.saturating_sub(delay);

    // Period count = fake_block_number / 100000
    let period_count = fake_block_number / 100_000;

    // Bomb only activates after period 2
    if period_count <= 2 {
        return U256::zero();
    }

    // Bomb = 2^(period_count - 2)
    let exponent = period_count - 2;

    // Prevent overflow for very large exponents
    if exponent >= 256 {
        // Return max U256 value (practically unreachable)
        return U256::MAX;
    }

    U256::one() << exponent
}

/// Get the difficulty bomb delay for a given fork.
fn get_bomb_delay(fork: Fork) -> u64 {
    match fork {
        Fork::GrayGlacier => bomb_delays::GRAY_GLACIER,
        Fork::ArrowGlacier => bomb_delays::ARROW_GLACIER,
        Fork::London => bomb_delays::LONDON,
        Fork::MuirGlacier | Fork::Berlin => bomb_delays::MUIR_GLACIER,
        Fork::Istanbul | Fork::Constantinople | Fork::Petersburg => bomb_delays::CONSTANTINOPLE,
        Fork::Byzantium => bomb_delays::BYZANTIUM,
        // Pre-Byzantium: no delay
        _ => 0,
    }
}

/// Verify that the calculated difficulty matches the block's difficulty.
///
/// # Arguments
/// * `header_difficulty` - The difficulty claimed in the block header
/// * `parent_difficulty` - The difficulty of the parent block
/// * `parent_timestamp` - The timestamp of the parent block
/// * `block_timestamp` - The timestamp of the current block
/// * `block_number` - The block number of the current block
/// * `parent_has_uncles` - Whether the parent block had uncles
/// * `fork` - The active fork at this block
///
/// # Returns
/// `true` if the difficulty is valid, `false` otherwise
pub fn verify_difficulty(
    header_difficulty: U256,
    parent_difficulty: U256,
    parent_timestamp: u64,
    block_timestamp: u64,
    block_number: u64,
    parent_has_uncles: bool,
    fork: Fork,
) -> bool {
    // Genesis block difficulty is set in config
    if block_number == 0 {
        return true;
    }

    // Calculate expected difficulty
    let base_adjustment = calculate_homestead_adjustment_with_uncles(
        parent_difficulty,
        parent_timestamp,
        block_timestamp,
        parent_has_uncles,
        fork,
    );

    let bomb = calculate_difficulty_bomb(block_number, fork);
    let expected = base_adjustment.saturating_add(bomb);
    let expected = U256::max(expected, U256::from(MIN_DIFFICULTY));

    header_difficulty == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_min_difficulty() {
        // Difficulty should never go below MIN_DIFFICULTY
        let result = calculate_difficulty(
            U256::from(MIN_DIFFICULTY),
            0,
            1000, // Very slow block
            1,
            Fork::Frontier,
        );
        assert!(result >= U256::from(MIN_DIFFICULTY));
    }

    #[test]
    fn test_frontier_fast_block() {
        // Fast block should increase difficulty
        let parent_diff = U256::from(1_000_000u64);
        let result = calculate_difficulty(
            parent_diff,
            100,
            101, // 1 second - very fast
            1,
            Fork::Frontier,
        );
        assert!(result > parent_diff);
    }

    #[test]
    fn test_frontier_slow_block() {
        // Slow block should decrease difficulty
        let parent_diff = U256::from(1_000_000u64);
        let result = calculate_difficulty(
            parent_diff,
            100,
            200, // 100 seconds - very slow
            1,
            Fork::Frontier,
        );
        assert!(result < parent_diff);
    }

    #[test]
    fn test_homestead_adjustment() {
        // Test Homestead difficulty adjustment
        let parent_diff = U256::from(1_000_000_000u64);

        // Fast block (5 seconds) - should increase difficulty
        let fast_result = calculate_difficulty(parent_diff, 100, 105, 1000, Fork::Homestead);
        // sigma = max(1 - 5/10, -99) = max(0.5, -99) = 0 (integer division)
        // Actually 5/10 = 0, so sigma = max(1 - 0, -99) = 1
        assert!(fast_result > parent_diff);

        // Slow block (20 seconds) - should decrease difficulty
        let slow_result = calculate_difficulty(parent_diff, 100, 120, 1000, Fork::Homestead);
        // sigma = max(1 - 20/10, -99) = max(1 - 2, -99) = max(-1, -99) = -1
        assert!(slow_result < parent_diff);

        // Exactly 10 seconds - sigma = max(1 - 1, -99) = 0, no change
        let neutral_result = calculate_difficulty(parent_diff, 100, 110, 1000, Fork::Homestead);
        assert_eq!(neutral_result, parent_diff);
    }

    #[test]
    fn test_difficulty_bomb_delay() {
        // Test that bomb delay works correctly
        assert_eq!(get_bomb_delay(Fork::Frontier), 0);
        assert_eq!(get_bomb_delay(Fork::Byzantium), 3_000_000);
        assert_eq!(get_bomb_delay(Fork::Constantinople), 5_000_000);
        assert_eq!(get_bomb_delay(Fork::MuirGlacier), 9_000_000);
        assert_eq!(get_bomb_delay(Fork::GrayGlacier), 11_400_000);
    }

    #[test]
    fn test_bomb_before_activation() {
        // Before period 2, bomb should be zero
        let bomb = calculate_difficulty_bomb(100_000, Fork::Frontier);
        assert_eq!(bomb, U256::zero());
    }

    #[test]
    fn test_bomb_after_activation() {
        // After period 2 (200,001 blocks), bomb should be non-zero
        let bomb = calculate_difficulty_bomb(300_001, Fork::Frontier);
        assert!(bomb > U256::zero());
        // At period 3: 2^(3-2) = 2
        assert_eq!(bomb, U256::from(2u64));
    }

    #[test]
    fn test_bomb_with_delay() {
        // With Byzantium delay (3M blocks), bomb at block 3_100_000 should be small
        let bomb = calculate_difficulty_bomb(3_100_000, Fork::Byzantium);
        // Fake block = 3_100_000 - 3_000_000 = 100_000
        // Period = 1, so bomb = 0
        assert_eq!(bomb, U256::zero());

        // At block 3_300_001 (fake block 300_001, period 3)
        let bomb = calculate_difficulty_bomb(3_300_001, Fork::Byzantium);
        assert_eq!(bomb, U256::from(2u64));
    }
}
