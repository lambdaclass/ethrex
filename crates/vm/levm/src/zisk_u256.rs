use ethrex_common::U256;
use ziskos::syscalls::{
    SyscallAdd256Params, SyscallArith256ModParams, SyscallArith256Params, syscall_add256,
    syscall_arith256, syscall_arith256_mod,
};

const ZERO_LIMBS: [u64; 4] = [0u64; 4];
const ONE_LIMBS: [u64; 4] = [1u64, 0, 0, 0];

// Hint-and-verify 256-bit division: the untrusted host supplies (quotient, remainder)
// correctness q*b + r == a and r < b check by assert
fn div_rem256(a: &[u64; 4], b: &[u64; 4]) -> ([u64; 4], [u64; 4]) {
    let (quo, rem) = ziskos::zisklib::fcall_uint256_div(a, b);

    let mut dl = ZERO_LIMBS;
    let mut dh = ZERO_LIMBS;
    let mut params = SyscallArith256Params {
        a: &quo,
        b,
        c: &rem,
        dl: &mut dl,
        dh: &mut dh,
    };
    syscall_arith256(&mut params);
    assert!(dl == *a, "division verification failed: q*b+r != a");
    assert!(dh == ZERO_LIMBS, "division overflow: q*b+r > 2^256");
    assert!(limbs_lt(&rem, b), "remainder must be less than divisor");

    (quo, rem)
}

#[inline(always)]
fn limbs_lt(a: &[u64; 4], b: &[u64; 4]) -> bool {
    if a[3] != b[3] {
        return a[3] < b[3];
    }
    if a[2] != b[2] {
        return a[2] < b[2];
    }
    if a[1] != b[1] {
        return a[1] < b[1];
    }
    a[0] < b[0]
}

// Unsigned 256-bit div, returns (0, 0) when b == 0
#[inline(always)]
pub fn div_rem(a: U256, b: U256) -> (U256, U256) {
    if b.is_zero() {
        return (U256::zero(), U256::zero());
    }
    let (quo, rem) = div_rem256(&a.0, &b.0);
    (U256(quo), U256(rem))
}

#[inline(always)]
pub fn checked_div(a: U256, b: U256) -> U256 {
    div_rem(a, b).0
}

#[inline(always)]
pub fn checked_rem(a: U256, b: U256) -> U256 {
    div_rem(a, b).1
}

// Wrapping 256-bit multiply: low 256 bits of a*b via arith256
#[inline(always)]
pub fn wrapping_mul(a: U256, b: U256) -> U256 {
    let mut dl = ZERO_LIMBS;
    let mut dh = ZERO_LIMBS;
    let mut params = SyscallArith256Params {
        a: &a.0,
        b: &b.0,
        c: &ZERO_LIMBS,
        dl: &mut dl,
        dh: &mut dh,
    };
    syscall_arith256(&mut params);
    U256(dl)
}

#[inline(always)]
pub fn mulmod(a: U256, b: U256, m: U256) -> U256 {
    if m.is_zero() {
        return U256::zero();
    }
    let mut result = ZERO_LIMBS;
    let mut params = SyscallArith256ModParams {
        a: &a.0,
        b: &b.0,
        c: &ZERO_LIMBS,
        module: &m.0,
        d: &mut result,
    };
    syscall_arith256_mod(&mut params);
    U256(result)
}

// Reuse arith256_mod as (a * 1 + b) mod m
#[inline(always)]
pub fn addmod(a: U256, b: U256, m: U256) -> U256 {
    if m.is_zero() {
        return U256::zero();
    }
    let mut result = ZERO_LIMBS;
    let mut params = SyscallArith256ModParams {
        a: &a.0,
        b: &ONE_LIMBS,
        c: &b.0,
        module: &m.0,
        d: &mut result,
    };
    syscall_arith256_mod(&mut params);
    U256(result)
}

#[inline(always)]
pub fn overflowing_add(a: U256, b: U256) -> (U256, bool) {
    let mut result = ZERO_LIMBS;
    let mut params = SyscallAdd256Params {
        a: &a.0,
        b: &b.0,
        cin: 0,
        c: &mut result,
    };
    let carry = syscall_add256(&mut params);
    (U256(result), carry != 0)
}

// Reuse add256 with: a + (-b) + 1.
#[inline(always)]
pub fn overflowing_sub(a: U256, b: U256) -> (U256, bool) {
    let neg_b = [!b.0[0], !b.0[1], !b.0[2], !b.0[3]];
    let mut result = ZERO_LIMBS;
    let mut params = SyscallAdd256Params {
        a: &a.0,
        b: &neg_b,
        cin: 1,
        c: &mut result,
    };
    let carry = syscall_add256(&mut params);
    (U256(result), carry == 0)
}
