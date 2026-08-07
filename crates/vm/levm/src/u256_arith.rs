//! 256-bit arithmetic for the opcode handlers, with the ZisK-native vs host
//! implementation selected once here instead of at every call site.
//!
//! On ZisK these lower to the native 256-bit syscalls in
//! [`crate::zisk_u256`] (a constrained AIR op each, far cheaper than the
//! limb-wise software loops); everywhere else they are the ordinary
//! `ethrex_common::U256`/`U512` operations. Both arms are semantically
//! identical, including the EVM's divide-by-zero-is-zero rule, so the handlers
//! carry no `cfg` of their own.

use ethrex_common::U256;
#[cfg(not(feature = "zisk"))]
use ethrex_common::U512;
use ethrex_crypto::Crypto;

/// Two's-complement negation (`0 - x`), used by the signed opcodes.
#[inline(always)]
pub fn negate(x: U256) -> U256 {
    #[cfg(feature = "zisk")]
    {
        crate::zisk_u256::overflowing_sub(U256::zero(), x).0
    }
    #[cfg(not(feature = "zisk"))]
    {
        U256::zero().overflowing_sub(x).0
    }
}

/// `MUL`: the low 256 bits of `a * b`.
#[inline(always)]
pub fn wrapping_mul(a: U256, b: U256) -> U256 {
    #[cfg(feature = "zisk")]
    {
        crate::zisk_u256::wrapping_mul(a, b)
    }
    #[cfg(not(feature = "zisk"))]
    {
        a.overflowing_mul(b).0
    }
}

/// `DIV`/`SDIV` quotient, zero when `b == 0` (the EVM's divide-by-zero rule).
#[inline(always)]
pub fn div_or_zero(a: U256, b: U256) -> U256 {
    #[cfg(feature = "zisk")]
    {
        crate::zisk_u256::checked_div(a, b)
    }
    #[cfg(not(feature = "zisk"))]
    {
        a.checked_div(b).unwrap_or(U256::zero())
    }
}

/// `MOD`/`SMOD` remainder, zero when `b == 0`.
#[inline(always)]
pub fn rem_or_zero(a: U256, b: U256) -> U256 {
    #[cfg(feature = "zisk")]
    {
        crate::zisk_u256::checked_rem(a, b)
    }
    #[cfg(not(feature = "zisk"))]
    {
        a.checked_rem(b).unwrap_or(U256::zero())
    }
}

/// `ADDMOD`: `(a + b) % m` at full width. The caller must have rejected
/// `m == 0` and `m == 1` (both push zero without arithmetic).
#[inline(always)]
pub fn addmod(a: U256, b: U256, m: U256) -> U256 {
    #[cfg(feature = "zisk")]
    {
        crate::zisk_u256::addmod(a, b, m)
    }
    #[cfg(not(feature = "zisk"))]
    {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "mod is checked non-zero by the caller"
        )]
        let res = U512::from(a).overflowing_add(b.into()).0 % m;
        U256([res.0[0], res.0[1], res.0[2], res.0[3]])
    }
}

/// `MULMOD`: `(a * b) % m` at full width. The caller must have rejected
/// `m == 0` and either operand being zero (all push zero without arithmetic).
#[inline(always)]
pub fn mulmod(crypto: &dyn Crypto, a: U256, b: U256, m: U256) -> U256 {
    #[cfg(feature = "zisk")]
    {
        // The syscall covers the whole operation; no host big-int backend needed.
        let _ = crypto;
        crate::zisk_u256::mulmod(a, b, m)
    }
    #[cfg(not(feature = "zisk"))]
    {
        let result_bytes =
            crypto.mulmod256(&a.to_big_endian(), &b.to_big_endian(), &m.to_big_endian());
        U256::from_big_endian(&result_bytes)
    }
}
