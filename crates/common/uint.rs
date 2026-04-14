// Injectable U256/U512 backend.
// Vendors install a custom backend via `install_uint256_backend()` at startup.
// Default: ethereum_types-based implementation.

use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Uint256Ops trait — the backend contract
// ---------------------------------------------------------------------------

/// Trait defining all U256/U512 operations that a backend must support.
/// Every method has a default implementation using ethereum_types.
/// Vendors override only the operations they want to accelerate.
pub trait Uint256Ops: Send + Sync + core::fmt::Debug {
    // ── U256 arithmetic ─────────────────────────────────────────────

    fn overflowing_add(&self, a: [u64; 4], b: [u64; 4]) -> ([u64; 4], bool) {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        let (v, o) = a.overflowing_add(b);
        (v.0, o)
    }

    fn overflowing_sub(&self, a: [u64; 4], b: [u64; 4]) -> ([u64; 4], bool) {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        let (v, o) = a.overflowing_sub(b);
        (v.0, o)
    }

    fn overflowing_mul(&self, a: [u64; 4], b: [u64; 4]) -> ([u64; 4], bool) {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        let (v, o) = a.overflowing_mul(b);
        (v.0, o)
    }

    fn overflowing_pow(&self, base: [u64; 4], exp: [u64; 4]) -> ([u64; 4], bool) {
        let base = ethereum_types::U256(base);
        let exp = ethereum_types::U256(exp);
        let (v, o) = base.overflowing_pow(exp);
        (v.0, o)
    }

    fn checked_add(&self, a: [u64; 4], b: [u64; 4]) -> Option<[u64; 4]> {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        a.checked_add(b).map(|v| v.0)
    }

    fn checked_sub(&self, a: [u64; 4], b: [u64; 4]) -> Option<[u64; 4]> {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        a.checked_sub(b).map(|v| v.0)
    }

    fn checked_mul(&self, a: [u64; 4], b: [u64; 4]) -> Option<[u64; 4]> {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        a.checked_mul(b).map(|v| v.0)
    }

    fn checked_div(&self, a: [u64; 4], b: [u64; 4]) -> Option<[u64; 4]> {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        a.checked_div(b).map(|v| v.0)
    }

    fn checked_rem(&self, a: [u64; 4], b: [u64; 4]) -> Option<[u64; 4]> {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        a.checked_rem(b).map(|v| v.0)
    }

    fn saturating_add(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        a.saturating_add(b).0
    }

    fn saturating_sub(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        a.saturating_sub(b).0
    }

    fn saturating_mul(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        a.saturating_mul(b).0
    }

    // ── U256 bitwise & inspection ───────────────────────────────────

    fn not(&self, a: [u64; 4]) -> [u64; 4] {
        [!a[0], !a[1], !a[2], !a[3]]
    }

    fn bitand(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        [a[0] & b[0], a[1] & b[1], a[2] & b[2], a[3] & b[3]]
    }

    fn bitor(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        [a[0] | b[0], a[1] | b[1], a[2] | b[2], a[3] | b[3]]
    }

    fn bitxor(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        [a[0] ^ b[0], a[1] ^ b[1], a[2] ^ b[2], a[3] ^ b[3]]
    }

    fn shl(&self, a: [u64; 4], shift: usize) -> [u64; 4] {
        let a = ethereum_types::U256(a);
        (a << shift).0
    }

    fn shr(&self, a: [u64; 4], shift: usize) -> [u64; 4] {
        let a = ethereum_types::U256(a);
        (a >> shift).0
    }

    fn div(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        (a / b).0
    }

    fn rem(&self, a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        let a = ethereum_types::U256(a);
        let b = ethereum_types::U256(b);
        (a % b).0
    }

    fn leading_zeros(&self, a: [u64; 4]) -> u32 {
        ethereum_types::U256(a).leading_zeros()
    }

    fn bits(&self, a: [u64; 4]) -> usize {
        ethereum_types::U256(a).bits()
    }

    fn bit(&self, a: [u64; 4], index: usize) -> bool {
        ethereum_types::U256(a).bit(index)
    }

    fn byte(&self, a: [u64; 4], index: usize) -> u8 {
        ethereum_types::U256(a).byte(index)
    }

    // ── U256 byte conversion ────────────────────────────────────────

    fn to_big_endian(&self, a: [u64; 4]) -> [u8; 32] {
        ethereum_types::U256(a).to_big_endian()
    }

    fn from_big_endian(&self, bytes: &[u8]) -> [u64; 4] {
        ethereum_types::U256::from_big_endian(bytes).0
    }

    fn from_little_endian(&self, bytes: &[u8]) -> [u64; 4] {
        ethereum_types::U256::from_little_endian(bytes).0
    }

    // ── U256 string parsing ─────────────────────────────────────────

    fn from_dec_str(&self, s: &str) -> Result<[u64; 4], ParseU256Error> {
        ethereum_types::U256::from_dec_str(s)
            .map(|v| v.0)
            .map_err(|e| ParseU256Error(e.to_string()))
    }

    fn from_str_radix(&self, s: &str, radix: u32) -> Result<[u64; 4], ParseU256Error> {
        ethereum_types::U256::from_str_radix(s, radix)
            .map(|v| v.0)
            .map_err(|_| ParseU256Error("invalid string for radix".to_string()))
    }

    // ── U512 operations (for ADDMOD) ────────────────────────────────

    fn u512_from_u256(&self, a: [u64; 4]) -> [u64; 8] {
        let v = ethereum_types::U512::from(ethereum_types::U256(a));
        v.0
    }

    fn u512_overflowing_add(&self, a: [u64; 8], b: [u64; 8]) -> ([u64; 8], bool) {
        let a = ethereum_types::U512(a);
        let b = ethereum_types::U512(b);
        let (v, o) = a.overflowing_add(b);
        (v.0, o)
    }

    fn u512_rem(&self, a: [u64; 8], b: [u64; 8]) -> [u64; 8] {
        let a = ethereum_types::U512(a);
        let b = ethereum_types::U512(b);
        (a % b).0
    }

    fn u512_rem_u256(&self, a: [u64; 8], b: [u64; 4]) -> [u64; 8] {
        let a = ethereum_types::U512(a);
        let b = ethereum_types::U512::from(ethereum_types::U256(b));
        (a % b).0
    }
}

// ---------------------------------------------------------------------------
// Default backend
// ---------------------------------------------------------------------------

/// Default U256 backend using ethereum_types. All trait methods use defaults.
#[derive(Debug)]
pub struct DefaultUint256Ops;

impl Uint256Ops for DefaultUint256Ops {}

// ---------------------------------------------------------------------------
// Global backend
// ---------------------------------------------------------------------------

static U256_OPS: OnceLock<Box<dyn Uint256Ops>> = OnceLock::new();

/// Install a custom U256 backend globally. Returns `true` if installed,
/// `false` if a backend was already set.
pub fn install_uint256_backend<B: Uint256Ops + 'static>(backend: B) -> bool {
    U256_OPS.set(Box::new(backend)).is_ok()
}

/// Get the installed backend, or the default if none was installed.
#[inline]
fn ops() -> &'static dyn Uint256Ops {
    U256_OPS
        .get_or_init(|| Box::new(DefaultUint256Ops))
        .as_ref()
}

// ---------------------------------------------------------------------------
// ParseU256Error
// ---------------------------------------------------------------------------

/// Error type for U256 string parsing.
#[derive(Debug)]
pub struct ParseU256Error(pub String);

impl core::fmt::Display for ParseU256Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseU256Error {}

// ---------------------------------------------------------------------------
// U256
// ---------------------------------------------------------------------------

/// 256-bit unsigned integer. Operations delegate through the installed backend.
#[repr(transparent)]
#[derive(Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct U256([u64; 4]);

// Manual Ord: compare from most-significant limb (index 3) to least (index 0).
// Derived Ord would compare in array order (index 0 first), which is wrong for LE limbs.
impl Ord for U256 {
    #[inline]
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0[3]
            .cmp(&other.0[3])
            .then(self.0[2].cmp(&other.0[2]))
            .then(self.0[1].cmp(&other.0[1]))
            .then(self.0[0].cmp(&other.0[0]))
    }
}

impl PartialOrd for U256 {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Custom serde: serialize as ethereum_types::U256 for wire compatibility.
impl serde::Serialize for U256 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ethereum_types::U256(self.0).serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for U256 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ethereum_types::U256::deserialize(deserializer).map(|v| Self(v.0))
    }
}

// ---- constants & constructors ----

impl U256 {
    pub const ZERO: Self = Self::from_limbs([0; 4]);
    pub const ONE: Self = Self::from_limbs([1, 0, 0, 0]);
    pub const MAX: Self = Self::from_limbs([u64::MAX; 4]);

    /// Construct from four u64 limbs in little-endian word order (limbs[0] is least significant).
    #[inline]
    pub const fn from_limbs(limbs: [u64; 4]) -> Self {
        Self(limbs)
    }

    /// Reference to internal limbs in little-endian word order.
    #[inline]
    pub fn as_limbs(&self) -> &[u64; 4] {
        &self.0
    }

    /// Mutable reference to internal limbs.
    #[inline]
    pub fn as_limbs_mut(&mut self) -> &mut [u64; 4] {
        &mut self.0
    }

    #[inline]
    pub const fn zero() -> Self {
        Self::ZERO
    }

    #[inline]
    pub const fn one() -> Self {
        Self::ONE
    }

    #[inline]
    pub const fn max_value() -> Self {
        Self::MAX
    }
}

// ---- byte conversion ----

impl U256 {
    /// Construct from big-endian bytes. Input may be shorter than 32 bytes (left-padded with zeros).
    #[inline]
    pub fn from_big_endian(bytes: &[u8]) -> Self {
        Self(ops().from_big_endian(bytes))
    }

    /// Convert to 32-byte big-endian representation.
    #[inline]
    pub fn to_big_endian(self) -> [u8; 32] {
        ops().to_big_endian(self.0)
    }

    /// Construct from little-endian bytes.
    #[inline]
    pub fn from_little_endian(bytes: &[u8]) -> Self {
        Self(ops().from_little_endian(bytes))
    }
}

// ---- arithmetic ----

impl U256 {
    #[inline]
    pub fn overflowing_add(self, rhs: Self) -> (Self, bool) {
        let (v, o) = ops().overflowing_add(self.0, rhs.0);
        (Self(v), o)
    }

    #[inline]
    pub fn overflowing_sub(self, rhs: Self) -> (Self, bool) {
        let (v, o) = ops().overflowing_sub(self.0, rhs.0);
        (Self(v), o)
    }

    #[inline]
    pub fn overflowing_mul(self, rhs: Self) -> (Self, bool) {
        let (v, o) = ops().overflowing_mul(self.0, rhs.0);
        (Self(v), o)
    }

    #[inline]
    pub fn overflowing_pow(self, exp: Self) -> (Self, bool) {
        let (v, o) = ops().overflowing_pow(self.0, exp.0);
        (Self(v), o)
    }

    #[inline]
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        ops().checked_add(self.0, rhs.0).map(Self)
    }

    #[inline]
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        ops().checked_sub(self.0, rhs.0).map(Self)
    }

    #[inline]
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        ops().checked_mul(self.0, rhs.0).map(Self)
    }

    #[inline]
    pub fn checked_div(self, rhs: Self) -> Option<Self> {
        ops().checked_div(self.0, rhs.0).map(Self)
    }

    #[inline]
    pub fn checked_rem(self, rhs: Self) -> Option<Self> {
        ops().checked_rem(self.0, rhs.0).map(Self)
    }

    #[inline]
    pub fn saturating_add(self, rhs: Self) -> Self {
        Self(ops().saturating_add(self.0, rhs.0))
    }

    #[inline]
    pub fn saturating_mul(self, rhs: Self) -> Self {
        Self(ops().saturating_mul(self.0, rhs.0))
    }

    #[inline]
    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(ops().saturating_sub(self.0, rhs.0))
    }

    #[inline]
    pub fn pow(self, exp: Self) -> Self {
        self.overflowing_pow(exp).0
    }

    /// Absolute difference.
    #[inline]
    pub fn abs_diff(self, other: Self) -> Self {
        if self > other {
            self - other
        } else {
            other - self
        }
    }
}

// ---- bitwise inspection ----

impl U256 {
    /// Returns the bit at the given index (0 = least significant).
    #[inline]
    pub fn bit(&self, index: usize) -> bool {
        ops().bit(self.0, index)
    }

    /// Returns the byte at the given index (0 = least significant byte).
    #[inline]
    pub fn byte(&self, index: usize) -> u8 {
        ops().byte(self.0, index)
    }

    /// Number of leading zero bits.
    #[inline]
    pub fn leading_zeros(&self) -> u32 {
        ops().leading_zeros(self.0)
    }

    /// Returns true if the value is zero.
    #[inline]
    pub fn is_zero(&self) -> bool {
        self.0 == [0; 4]
    }

    /// Number of bits needed to represent this value.
    #[inline]
    pub fn bits(&self) -> usize {
        ops().bits(self.0)
    }
}

// ---- numeric conversion ----

impl U256 {
    /// Returns the value as u64.
    ///
    /// # Panics
    ///
    /// Panics if the value does not fit in u64 (upper limbs are nonzero),
    /// matching the behavior of `ethereum_types::U256::as_u64()`.
    #[inline]
    pub fn as_u64(&self) -> u64 {
        assert!(
            self.0[1] == 0 && self.0[2] == 0 && self.0[3] == 0,
            "U256 value does not fit in u64"
        );
        self.0[0]
    }

    /// Returns the low 32 bits.
    #[inline]
    pub fn low_u32(&self) -> u32 {
        self.0[0] as u32
    }

    /// Returns the low 64 bits.
    #[inline]
    pub fn low_u64(&self) -> u64 {
        self.0[0]
    }

    /// Returns the low bits as usize.
    #[inline]
    pub fn as_usize(&self) -> usize {
        self.0[0] as usize
    }

    /// Parse from decimal string.
    #[inline]
    pub fn from_dec_str(s: &str) -> Result<Self, ParseU256Error> {
        ops().from_dec_str(s).map(Self)
    }

    /// Convert to 32-byte little-endian representation.
    #[inline]
    pub fn to_little_endian(self) -> [u8; 32] {
        let be = self.to_big_endian();
        let mut le = [0u8; 32];
        for i in 0..32 {
            le[i] = be[31 - i];
        }
        le
    }

    /// Parse from hex or decimal string (tries hex with 0x prefix, otherwise decimal).
    #[inline]
    pub fn from_str(s: &str) -> Result<Self, ParseU256Error> {
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            Self::from_str_radix(hex, 16)
        } else {
            Self::from_dec_str(s)
        }
    }

    /// Parse from string with given radix.
    #[inline]
    pub fn from_str_radix(s: &str, radix: u32) -> Result<Self, ParseU256Error> {
        ops().from_str_radix(s, radix).map(Self)
    }
}

impl core::str::FromStr for U256 {
    type Err = ParseU256Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str(s)
    }
}

// ---- From impls ----

macro_rules! impl_from_uint {
    ($($t:ty),*) => {
        $(
            impl From<$t> for U256 {
                #[inline]
                fn from(v: $t) -> Self {
                    let mut limbs = [0u64; 4];
                    limbs[0] = v as u64;
                    #[allow(unused_comparisons)]
                    if core::mem::size_of::<$t>() > 8 {
                        limbs[1] = ((v as u128) >> 64) as u64;
                    }
                    Self(limbs)
                }
            }
        )*
    };
}

impl_from_uint!(u8, u16, u32, u64, usize);

impl From<u128> for U256 {
    #[inline]
    fn from(v: u128) -> Self {
        Self([v as u64, (v >> 64) as u64, 0, 0])
    }
}

// Signed conversions: two's complement sign extension for EVM compatibility.
impl From<i32> for U256 {
    #[inline]
    fn from(v: i32) -> Self {
        if v >= 0 {
            Self::from(v as u64)
        } else {
            let abs_minus_one = (!(v as u32)) as u64;
            Self::MAX - Self::from(abs_minus_one)
        }
    }
}

impl From<i64> for U256 {
    #[inline]
    fn from(v: i64) -> Self {
        if v >= 0 {
            Self::from(v as u64)
        } else {
            let abs_minus_one = !(v as u64);
            Self::MAX - Self::from(abs_minus_one)
        }
    }
}

impl From<bool> for U256 {
    #[inline]
    fn from(v: bool) -> Self {
        Self::from(v as u64)
    }
}

// ---- TryFrom impls (reverse conversions) ----

impl TryFrom<U256> for u64 {
    type Error = &'static str;
    #[inline]
    fn try_from(v: U256) -> Result<Self, Self::Error> {
        if v.0[1] != 0 || v.0[2] != 0 || v.0[3] != 0 {
            Err("U256 value too large for u64")
        } else {
            Ok(v.0[0])
        }
    }
}

impl TryFrom<U256> for usize {
    type Error = &'static str;
    #[inline]
    fn try_from(v: U256) -> Result<Self, Self::Error> {
        let val: u64 = v.try_into()?;
        usize::try_from(val).map_err(|_| "U256 value too large for usize")
    }
}

impl TryFrom<U256> for u8 {
    type Error = &'static str;
    #[inline]
    fn try_from(v: U256) -> Result<Self, Self::Error> {
        let val: u64 = v.try_into()?;
        u8::try_from(val).map_err(|_| "U256 value too large for u8")
    }
}

// ---- operator traits ----

impl core::ops::Add for U256 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        self.overflowing_add(rhs).0
    }
}

impl core::ops::Sub for U256 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        self.overflowing_sub(rhs).0
    }
}

impl core::ops::Mul for U256 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        self.overflowing_mul(rhs).0
    }
}

impl core::ops::Div for U256 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        Self(ops().div(self.0, rhs.0))
    }
}

impl core::ops::Rem for U256 {
    type Output = Self;
    #[inline]
    fn rem(self, rhs: Self) -> Self {
        Self(ops().rem(self.0, rhs.0))
    }
}

impl core::ops::BitAnd for U256 {
    type Output = Self;
    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        Self(ops().bitand(self.0, rhs.0))
    }
}

impl core::ops::BitOr for U256 {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(ops().bitor(self.0, rhs.0))
    }
}

impl core::ops::BitXor for U256 {
    type Output = Self;
    #[inline]
    fn bitxor(self, rhs: Self) -> Self {
        Self(ops().bitxor(self.0, rhs.0))
    }
}

impl core::ops::Not for U256 {
    type Output = Self;
    #[inline]
    fn not(self) -> Self {
        Self(ops().not(self.0))
    }
}

impl core::ops::Shl<usize> for U256 {
    type Output = Self;
    #[inline]
    fn shl(self, rhs: usize) -> Self {
        Self(ops().shl(self.0, rhs))
    }
}

impl core::ops::Shr<usize> for U256 {
    type Output = Self;
    #[inline]
    fn shr(self, rhs: usize) -> Self {
        Self(ops().shr(self.0, rhs))
    }
}

// Cross-type arithmetic (U256 op primitive).
macro_rules! impl_cross_binop {
    ($prim:ty, $trait:ident, $method:ident) => {
        impl core::ops::$trait<$prim> for U256 {
            type Output = Self;
            #[inline]
            fn $method(self, rhs: $prim) -> Self {
                core::ops::$trait::$method(self, U256::from(rhs as u64))
            }
        }
    };
}

impl_cross_binop!(u64, Add, add);
impl_cross_binop!(u64, Sub, sub);
impl_cross_binop!(u64, Mul, mul);
impl_cross_binop!(u64, Div, div);
impl_cross_binop!(usize, Add, add);
impl_cross_binop!(usize, Sub, sub);
impl_cross_binop!(usize, Mul, mul);
impl_cross_binop!(usize, Div, div);
// i32 cross-ops intentionally omitted: `rhs as u64` sign-extends negative
// values (e.g. -1i32 becomes u64::MAX), producing silently wrong results.
// Callers should cast explicitly: `u256 - U256::from(n as u64)`

macro_rules! impl_assign_via_op {
    ($trait:ident, $method:ident, $op_trait:ident, $op_method:ident) => {
        impl core::ops::$trait for U256 {
            #[inline]
            fn $method(&mut self, rhs: Self) {
                *self = core::ops::$op_trait::$op_method(*self, rhs);
            }
        }
    };
}

impl_assign_via_op!(AddAssign, add_assign, Add, add);
impl_assign_via_op!(SubAssign, sub_assign, Sub, sub);
impl_assign_via_op!(MulAssign, mul_assign, Mul, mul);
impl_assign_via_op!(DivAssign, div_assign, Div, div);
impl_assign_via_op!(RemAssign, rem_assign, Rem, rem);
impl_assign_via_op!(BitAndAssign, bitand_assign, BitAnd, bitand);
impl_assign_via_op!(BitOrAssign, bitor_assign, BitOr, bitor);
impl_assign_via_op!(BitXorAssign, bitxor_assign, BitXor, bitxor);

// Additional shift impls for common integer types
macro_rules! impl_shift {
    ($($t:ty),*) => {
        $(
            impl core::ops::Shl<$t> for U256 {
                type Output = Self;
                #[inline]
                fn shl(self, rhs: $t) -> Self {
                    Self(ops().shl(self.0, rhs as usize))
                }
            }
            impl core::ops::Shr<$t> for U256 {
                type Output = Self;
                #[inline]
                fn shr(self, rhs: $t) -> Self {
                    Self(ops().shr(self.0, rhs as usize))
                }
            }
        )*
    };
}

impl_shift!(u8, u16, u32, u64, i32, i64);

impl core::ops::ShlAssign<usize> for U256 {
    #[inline]
    fn shl_assign(&mut self, rhs: usize) {
        *self = *self << rhs;
    }
}

impl core::ops::ShrAssign<usize> for U256 {
    #[inline]
    fn shr_assign(&mut self, rhs: usize) {
        *self = *self >> rhs;
    }
}

// ---- formatting ----

impl core::fmt::Debug for U256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#x}", ethereum_types::U256(self.0))
    }
}

impl core::fmt::Display for U256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", ethereum_types::U256(self.0))
    }
}

impl core::fmt::LowerHex for U256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::LowerHex::fmt(&ethereum_types::U256(self.0), f)
    }
}

impl core::fmt::UpperHex for U256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::UpperHex::fmt(&ethereum_types::U256(self.0), f)
    }
}

// ---------------------------------------------------------------------------
// BigEndianHash interop
// ---------------------------------------------------------------------------

impl U256 {
    /// Convert H256 to U256 (big-endian interpretation).
    #[inline]
    pub fn from_h256(h: ethereum_types::H256) -> Self {
        Self::from_big_endian(h.as_bytes())
    }

    /// Convert U256 to H256 (big-endian representation).
    #[inline]
    pub fn to_h256(self) -> ethereum_types::H256 {
        ethereum_types::H256::from(self.to_big_endian())
    }
}

impl ethereum_types::BigEndianHash for U256 {
    type Uint = Self;

    fn from_uint(value: &Self) -> Self {
        *value
    }

    fn into_uint(&self) -> Self {
        *self
    }
}

// Conversions between our U256 and ethereum_types::U256
impl From<U256> for ethereum_types::U256 {
    #[inline]
    fn from(v: U256) -> Self {
        ethereum_types::U256(v.0)
    }
}

impl From<ethereum_types::U256> for U256 {
    #[inline]
    fn from(v: ethereum_types::U256) -> Self {
        Self(v.0)
    }
}

// ---------------------------------------------------------------------------
// U512
// ---------------------------------------------------------------------------

/// 512-bit unsigned integer. Used for intermediate ADDMOD arithmetic.
#[repr(transparent)]
#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct U512([u64; 8]);

impl U512 {
    #[inline]
    pub fn overflowing_add(self, rhs: Self) -> (Self, bool) {
        let (v, o) = ops().u512_overflowing_add(self.0, rhs.0);
        (Self(v), o)
    }

    /// Extract the low 256 bits as a U256.
    #[inline]
    pub fn low_u256(self) -> U256 {
        U256::from_limbs([self.0[0], self.0[1], self.0[2], self.0[3]])
    }

    /// Reference to internal limbs.
    #[inline]
    pub fn as_limbs(&self) -> &[u64; 8] {
        &self.0
    }
}

impl From<U256> for U512 {
    #[inline]
    fn from(v: U256) -> Self {
        Self(ops().u512_from_u256(v.0))
    }
}

impl core::ops::Rem for U512 {
    type Output = Self;
    #[inline]
    fn rem(self, rhs: Self) -> Self {
        Self(ops().u512_rem(self.0, rhs.0))
    }
}

/// U512 % U256 — used in ADDMOD opcode.
impl core::ops::Rem<U256> for U512 {
    type Output = Self;
    #[inline]
    fn rem(self, rhs: U256) -> Self {
        Self(ops().u512_rem_u256(self.0, rhs.0))
    }
}

impl core::fmt::Debug for U512 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#x}", ethereum_types::U512(self.0))
    }
}

// ---------------------------------------------------------------------------
// RLP encode/decode for U256 (backend-agnostic, via byte conversion)
// ---------------------------------------------------------------------------

impl ethrex_rlp::encode::RLPEncode for U256 {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        let leading_zeros_in_bytes: usize = (self.leading_zeros() / 8) as usize;
        let bytes = self.to_big_endian();
        bytes[leading_zeros_in_bytes..].encode(buf)
    }

    fn length(&self) -> usize {
        let bits = self.bits().saturating_sub(1) as u32;
        let lsb = (self.low_u32() & 0xff) as u8;
        let sig_len = (bits + 8) >> 3;
        let is_multibyte_mask = ((sig_len > 1) as usize) | ((lsb > 0x7f) as usize);
        1 + sig_len as usize * is_multibyte_mask
    }
}

impl ethrex_rlp::decode::RLPDecode for U256 {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), ethrex_rlp::error::RLPDecodeError> {
        let (bytes, rest) = ethrex_rlp::decode::decode_bytes(rlp)?;
        let padded_bytes: [u8; 32] = ethrex_rlp::decode::static_left_pad(bytes)?;
        Ok((U256::from_big_endian(&padded_bytes), rest))
    }
}
