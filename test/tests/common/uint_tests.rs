use ethrex_common::{U256, U512};

// EVM-critical operations — these must produce identical results regardless of backend.
// Expected values are from ethereum_types (the reference backend).

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
    // 2^10 = 1024
    assert_eq!(
        U256::from(2u64).overflowing_pow(U256::from(10u64)),
        (U256::from(1024u64), false)
    );
    // 0^0 = 1 (EVM convention)
    assert_eq!(
        U256::zero().overflowing_pow(U256::zero()),
        (U256::one(), false)
    );
    // 2^256 overflows
    assert!(U256::from(2u64).overflowing_pow(U256::from(256u64)).1);
    // 2^255
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
    // Shift by exactly 256 should return 0 (as in EVM)
    assert_eq!(U256::MAX << 256usize, U256::zero());
    assert_eq!(U256::MAX >> 256usize, U256::zero());
    // Shift by 255
    assert_eq!(
        U256::one() << 255usize,
        U256::from_limbs([0, 0, 0, 1 << 63])
    );
    assert_eq!(U256::MAX >> 255usize, U256::one());
}

#[test]
fn test_signextend_pattern() {
    // SIGNEXTEND opcode pattern: value |= U256::MAX << (8 * (x + 1))
    // For x=0: MAX << 8
    let mask = U256::MAX << 8usize;
    let val = U256::from(0x80u64); // sign bit set for byte 0
    assert_eq!(val | mask, U256::MAX - U256::from(0x7fu64));
}

#[test]
fn test_byte_method() {
    // byte(0) = least significant byte (LE convention, same as both backends)
    let val = U256::from(0xABu64);
    assert_eq!(val.byte(0), 0xAB); // least significant byte
    assert_eq!(val.byte(31), 0x00); // most significant byte
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
    // -1i32 → MAX
    assert_eq!(U256::from(-1i32), U256::MAX);
    // -1i64 → MAX
    assert_eq!(U256::from(-1i64), U256::MAX);
    // -2i32 → MAX - 1
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
    // Simulate ADDMOD opcode: (a + b) % mod
    let a = U256::MAX;
    let b = U256::from(2u64);
    let m = U256::from(3u64);
    let result = (U512::from(a).overflowing_add(b.into()).0 % m).low_u256();
    // MAX + 2 = 2^256 + 1; (2^256 + 1) % 3 = ?
    // 2^256 mod 3 = (2^2)^128 mod 3 = 1^128 mod 3 = 1; so (1+1) % 3 = 2
    assert_eq!(result, U256::from(2u64));
}

#[test]
fn test_cross_type_arith_negative_i32() {
    // The cross-type macro casts i32 to u64 before U256::from.
    // For negative i32: (-1i32 as u64) = 0xFFFFFFFF_FFFFFFFF
    // So U256 + (-1i32) actually adds a very large number.
    let x = U256::from(100u64);
    let result = x + (-1i32);
    // -1i32 as u64 = 18446744073709551615, so U256::from(18446744073709551615u64)
    assert_eq!(result, x + U256::from(u64::MAX));
}
