use ethrex_blockchain::constants::GAS_LIMIT_BOUND_DIVISOR;
use ethrex_blockchain::payload::{BuildPayloadArgs, calc_gas_limit};
use ethrex_common::{Address, H256};

fn base_args() -> BuildPayloadArgs {
    BuildPayloadArgs {
        parent: H256::repeat_byte(0xAA),
        timestamp: 100,
        fee_recipient: Address::repeat_byte(0xBB),
        random: H256::repeat_byte(0xCC),
        withdrawals: None,
        beacon_root: None,
        slot_number: Some(42),
        version: 4,
        elasticity_multiplier: 2,
        gas_ceil: 60_000_000,
    }
}

#[test]
fn payload_id_distinguishes_different_gas_ceil() {
    // execution-apis#796: two FCUs differing only in CL-supplied
    // targetGasLimit must produce different payload IDs.
    let mut a = base_args();
    let mut b = base_args();
    a.gas_ceil = 30_000_000;
    b.gas_ceil = 60_000_000;
    assert_ne!(a.id().unwrap(), b.id().unwrap());
}

#[test]
fn payload_id_distinguishes_different_slot_number() {
    // Latent bug pre-glamsterdam-devnet-4: slot_number was not hashed,
    // so two attribute sets that differed only in slot collided.
    let mut a = base_args();
    let mut b = base_args();
    a.slot_number = Some(100);
    b.slot_number = Some(101);
    assert_ne!(a.id().unwrap(), b.id().unwrap());
}

#[test]
fn payload_id_stable_when_inputs_match() {
    let a = base_args();
    let b = base_args();
    assert_eq!(a.id().unwrap(), b.id().unwrap());
}

#[test]
fn gas_limit_steps_up_toward_higher_target() {
    // execution-apis#796: CL-supplied target above parent → one
    // EIP-1559 1/1024 step upward, capped at the target.
    let parent = 30_000_000u64;
    let target = 50_000_000u64;
    let step = parent / GAS_LIMIT_BOUND_DIVISOR - 1;
    assert_eq!(calc_gas_limit(parent, target), parent + step);
}

#[test]
fn gas_limit_steps_down_toward_lower_target() {
    // CL-supplied target below parent → one EIP-1559 step downward.
    let parent = 60_000_000u64;
    let target = 45_000_000u64;
    let step = parent / GAS_LIMIT_BOUND_DIVISOR - 1;
    assert_eq!(calc_gas_limit(parent, target), parent - step);
}

#[test]
fn gas_limit_clamps_to_target_when_step_overshoots() {
    // If target is within one step, we land exactly on it.
    let parent = 30_000_000u64;
    let target = parent + 100;
    assert_eq!(calc_gas_limit(parent, target), target);
}
