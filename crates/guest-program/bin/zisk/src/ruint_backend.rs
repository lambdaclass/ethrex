//! Ruint-based U256 backend for the Uint256Ops trait.

use ethrex_common::{ParseU256Error, Uint256Ops};

type RU256 = ruint::Uint<256, 4>;
type RU512 = ruint::Uint<512, 8>;

#[derive(Debug)]
pub struct RuintUint256Ops;

impl Uint256Ops for RuintUint256Ops {
    fn overflowing_add(&self, a: [u64; 4], b: [u64; 4]) -> ([u64; 4], bool) {
        let (v, o) = RU256::from_limbs(a).overflowing_add(RU256::from_limbs(b));
        (*v.as_limbs(), o)
    }

    fn overflowing_sub(&self, a: [u64; 4], b: [u64; 4]) -> ([u64; 4], bool) {
        let (v, o) = RU256::from_limbs(a).overflowing_sub(RU256::from_limbs(b));
        (*v.as_limbs(), o)
    }

    fn overflowing_mul(&self, a: [u64; 4], b: [u64; 4]) -> ([u64; 4], bool) {
        let (v, o) = RU256::from_limbs(a).overflowing_mul(RU256::from_limbs(b));
        (*v.as_limbs(), o)
    }

    fn overflowing_pow(&self, base: [u64; 4], exp: [u64; 4]) -> ([u64; 4], bool) {
        let (v, o) = RU256::from_limbs(base).overflowing_pow(RU256::from_limbs(exp));
        (*v.as_limbs(), o)
    }

    fn checked_add(&self, a: [u64; 4], b: [u64; 4]) -> Option<[u64; 4]> {
        RU256::from_limbs(a)
            .checked_add(RU256::from_limbs(b))
            .map(|v| *v.as_limbs())
    }

    fn checked_sub(&self, a: [u64; 4], b: [u64; 4]) -> Option<[u64; 4]> {
        RU256::from_limbs(a)
            .checked_sub(RU256::from_limbs(b))
            .map(|v| *v.as_limbs())
    }

    fn checked_mul(&self, a: [u64; 4], b: [u64; 4]) -> Option<[u64; 4]> {
        RU256::from_limbs(a)
            .checked_mul(RU256::from_limbs(b))
            .map(|v| *v.as_limbs())
    }

    fn checked_div(&self, a: [u64; 4], b: [u64; 4]) -> Option<[u64; 4]> {
        RU256::from_limbs(a)
            .checked_div(RU256::from_limbs(b))
            .map(|v| *v.as_limbs())
    }

    fn checked_rem(&self, a: [u64; 4], b: [u64; 4]) -> Option<[u64; 4]> {
        RU256::from_limbs(a)
            .checked_rem(RU256::from_limbs(b))
            .map(|v| *v.as_limbs())
    }

    fn saturating_add(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        *RU256::from_limbs(a)
            .saturating_add(RU256::from_limbs(b))
            .as_limbs()
    }

    fn saturating_sub(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        *RU256::from_limbs(a)
            .saturating_sub(RU256::from_limbs(b))
            .as_limbs()
    }

    fn saturating_mul(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        *RU256::from_limbs(a)
            .saturating_mul(RU256::from_limbs(b))
            .as_limbs()
    }

    fn shl(&self, a: [u64; 4], shift: usize) -> [u64; 4] {
        *(RU256::from_limbs(a) << shift).as_limbs()
    }

    fn shr(&self, a: [u64; 4], shift: usize) -> [u64; 4] {
        *(RU256::from_limbs(a) >> shift).as_limbs()
    }

    fn div(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        *(RU256::from_limbs(a) / RU256::from_limbs(b)).as_limbs()
    }

    fn rem(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        *(RU256::from_limbs(a) % RU256::from_limbs(b)).as_limbs()
    }

    fn leading_zeros(&self, a: [u64; 4]) -> u32 {
        RU256::from_limbs(a).leading_zeros() as u32
    }

    fn bits(&self, a: [u64; 4]) -> usize {
        RU256::from_limbs(a).bit_len()
    }

    fn bit(&self, a: [u64; 4], index: usize) -> bool {
        RU256::from_limbs(a).bit(index)
    }

    fn byte(&self, a: [u64; 4], index: usize) -> u8 {
        RU256::from_limbs(a).byte(index)
    }

    fn to_big_endian(&self, a: [u64; 4]) -> [u8; 32] {
        RU256::from_limbs(a).to_be_bytes::<32>()
    }

    fn from_big_endian(&self, bytes: &[u8]) -> [u64; 4] {
        if bytes.len() >= 32 {
            *RU256::from_be_slice(&bytes[bytes.len() - 32..]).as_limbs()
        } else {
            let mut padded = [0u8; 32];
            padded[32 - bytes.len()..].copy_from_slice(bytes);
            *RU256::from_be_slice(&padded).as_limbs()
        }
    }

    fn from_little_endian(&self, bytes: &[u8]) -> [u64; 4] {
        if bytes.len() >= 32 {
            *RU256::from_le_slice(&bytes[..32]).as_limbs()
        } else {
            let mut padded = [0u8; 32];
            padded[..bytes.len()].copy_from_slice(bytes);
            *RU256::from_le_slice(&padded).as_limbs()
        }
    }

    fn from_dec_str(&self, s: &str) -> Result<[u64; 4], ParseU256Error> {
        RU256::from_str_radix(s, 10)
            .map(|v| *v.as_limbs())
            .map_err(|e| ParseU256Error(e.to_string()))
    }

    fn from_str_radix(&self, s: &str, radix: u32) -> Result<[u64; 4], ParseU256Error> {
        RU256::from_str_radix(s, radix as u64)
            .map(|v| *v.as_limbs())
            .map_err(|e| ParseU256Error(e.to_string()))
    }

    fn u512_from_u256(&self, a: [u64; 4]) -> [u64; 8] {
        *RU512::from(RU256::from_limbs(a)).as_limbs()
    }

    fn u512_overflowing_add(&self, a: [u64; 8], b: [u64; 8]) -> ([u64; 8], bool) {
        let (v, o) = RU512::from_limbs(a).overflowing_add(RU512::from_limbs(b));
        (*v.as_limbs(), o)
    }

    fn u512_rem(&self, a: [u64; 8], b: [u64; 8]) -> [u64; 8] {
        *(RU512::from_limbs(a) % RU512::from_limbs(b)).as_limbs()
    }

    fn u512_rem_u256(&self, a: [u64; 8], b: [u64; 4]) -> [u64; 8] {
        *(RU512::from_limbs(a) % RU512::from(RU256::from_limbs(b))).as_limbs()
    }
}
