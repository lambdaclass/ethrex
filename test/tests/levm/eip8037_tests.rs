//! EIP-8037: Dynamic cost_per_state_byte Tests

use ethrex_levm::gas_cost::cost_per_state_byte;

/// Sanity check: cost_per_state_byte(120_000_000) == 1174
/// (matches the legacy hardcoded COST_PER_STATE_BYTE constant)
#[test]
fn test_cpsb_120m() {
    assert_eq!(cost_per_state_byte(120_000_000), 1174);
}

/// gas_limit = 30_000_000
/// num = 30_000_000 * 2_628_000 = 78_840_000_000_000
/// denom = 2 * 100 * 2^30 = 214_748_364_800
/// raw = ceil(78_840_000_000_000 / 214_748_364_800) = 368
/// shifted = 368 + 9578 = 9946
/// bit_length = 14, shift = 9
/// quantized = (9946 >> 9) << 9 = 19 * 512 = 9728
/// result = 9728 - 9578 = 150
#[test]
fn test_cpsb_30m() {
    assert_eq!(cost_per_state_byte(30_000_000), 150);
}

/// gas_limit = 500_000_000
/// raw = ceil(500_000_000 * 2_628_000 / 214_748_364_800) = 6119
/// shifted = 6119 + 9578 = 15697
/// bit_length = 14, shift = 9
/// quantized = (15697 >> 9) << 9 = 30 * 512 = 15360
/// result = 15360 - 9578 = 5782
#[test]
fn test_cpsb_500m() {
    assert_eq!(cost_per_state_byte(500_000_000), 5782);
}
