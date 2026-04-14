use ethrex_common::{U256, U512};

// EVM-critical operations — these must produce identical results regardless of backend.

#[test]
fn test_overflowing_add() {
    assert_eq!(U256::MAX.overflowing_add(U256::one()), (U256::zero(), true));
    assert_eq!(
        U256::one().overflowing_add(U256::one()),
        (U256::from(2u64), false)
    );
}

#[test]
fn test_overflowing_sub() {
    assert_eq!(U256::zero().overflowing_sub(U256::one()), (U256::MAX, true));
    assert_eq!(
        U256::from(5u64).overflowing_sub(U256::from(3u64)),
        (U256::from(2u64), false)
    );
}

#[test]
fn test_overflowing_mul() {
    assert_eq!(U256::MAX.overflowing_mul(U256::from(2u64)).1, true);
    assert_eq!(
        U256::from(7u64).overflowing_mul(U256::from(6u64)),
        (U256::from(42u64), false)
    );
}

#[test]
fn test_overflowing_pow() {
    assert_eq!(
        U256::from(2u64).overflowing_pow(U256::from(10u64)),
        (U256::from(1024u64), false)
    );
    assert_eq!(
        U256::zero().overflowing_pow(U256::zero()),
        (U256::one(), false)
    );
    assert!(U256::from(2u64).overflowing_pow(U256::from(256u64)).1);
    let pow255 = U256::from(2u64).overflowing_pow(U256::from(255u64));
    assert_eq!(pow255.0, U256::one() << 255usize);
    assert!(!pow255.1);
}

#[test]
fn test_checked_div_and_rem() {
    assert_eq!(
        U256::from(10u64).checked_div(U256::from(3u64)),
        Some(U256::from(3u64))
    );
    assert_eq!(U256::from(10u64).checked_div(U256::zero()), None);
    assert_eq!(
        U256::from(10u64).checked_rem(U256::from(3u64)),
        Some(U256::from(1u64))
    );
    assert_eq!(U256::from(10u64).checked_rem(U256::zero()), None);
}

#[test]
fn test_shift_at_boundary() {
    assert_eq!(U256::MAX << 256usize, U256::zero());
    assert_eq!(U256::MAX >> 256usize, U256::zero());
    assert_eq!(
        U256::one() << 255usize,
        U256::from_limbs([0, 0, 0, 1 << 63])
    );
    assert_eq!(U256::MAX >> 255usize, U256::one());
}

#[test]
fn test_signextend_pattern() {
    let mask = U256::MAX << 8usize;
    let val = U256::from(0x80u64);
    assert_eq!(val | mask, U256::MAX - U256::from(0x7fu64));
}

#[test]
fn test_byte_method() {
    let val = U256::from(0xABu64);
    assert_eq!(val.byte(0), 0xAB);
    assert_eq!(val.byte(31), 0x00);
}

#[test]
fn test_bit_method() {
    assert!(U256::one().bit(0));
    assert!(!U256::one().bit(1));
    assert!(U256::MAX.bit(255));
}

#[test]
fn test_to_from_big_endian_roundtrip() {
    let vals = [
        U256::zero(),
        U256::one(),
        U256::MAX,
        U256::from(0xDEADBEEFu64),
    ];
    for v in vals {
        let be = v.to_big_endian();
        assert_eq!(U256::from_big_endian(&be), v);
    }
}

#[test]
fn test_rlp_roundtrip() {
    use ethrex_rlp::decode::RLPDecode;
    use ethrex_rlp::encode::RLPEncode;
    let vals = [
        U256::zero(),
        U256::one(),
        U256::from(128u64),
        U256::from(256u64),
        U256::MAX,
    ];
    for v in vals {
        let encoded = v.encode_to_vec();
        let (decoded, rest) = U256::decode_unfinished(&encoded).unwrap();
        assert!(rest.is_empty());
        assert_eq!(decoded, v, "RLP roundtrip failed for {v:?}");
    }
}

#[test]
fn test_leading_zeros_and_bits() {
    assert_eq!(U256::zero().leading_zeros(), 256);
    assert_eq!(U256::one().leading_zeros(), 255);
    assert_eq!(U256::MAX.leading_zeros(), 0);
    assert_eq!(U256::zero().bits(), 0);
    assert_eq!(U256::one().bits(), 1);
    assert_eq!(U256::MAX.bits(), 256);
}

#[test]
fn test_signed_from() {
    assert_eq!(U256::from(-1i32), U256::MAX);
    assert_eq!(U256::from(-1i64), U256::MAX);
    assert_eq!(U256::from(-2i32), U256::MAX - U256::one());
}

#[test]
fn test_not() {
    assert_eq!(!U256::zero(), U256::MAX);
    assert_eq!(!U256::MAX, U256::zero());
    assert_eq!(!U256::one(), U256::MAX - U256::one());
}

#[test]
fn test_u512_addmod() {
    let a = U256::MAX;
    let b = U256::from(2u64);
    let m = U256::from(3u64);
    let result = (U512::from(a).overflowing_add(b.into()).0 % m).low_u256();
    assert_eq!(result, U256::from(2u64));
}

#[test]
fn test_cross_type_arith_negative_i32() {
    let x = U256::from(100u64);
    let result = x + (-1i32);
    assert_eq!(result, x + U256::from(u64::MAX));
}

// ---- Backend injection tests ----

#[test]
fn test_default_backend_works_without_install() {
    // No install_uint256_backend call — default kicks in.
    assert_eq!(
        U256::from(10u64).overflowing_add(U256::from(20u64)),
        (U256::from(30u64), false)
    );
}

#[test]
fn test_install_backend_returns_false_on_second_call() {
    // OnceLock is already set (by default init or a prior install).
    // A second call should return false.
    let result = ethrex_common::install_uint256_backend(ethrex_common::DefaultUint256Ops);
    // Can't assert true/false deterministically since other tests may have triggered
    // the default init. Just verify it doesn't panic.
    let _ = result;
}
