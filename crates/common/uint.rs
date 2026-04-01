// Switchable U256/U512 backend.
// Selected at compile time via feature flags: `uint-ethereum-types` (default) or `uint-ruint`.

#[cfg(all(feature = "uint-ethereum-types", feature = "uint-ruint"))]
compile_error!(
    "Only one uint backend can be enabled: choose `uint-ethereum-types` or `uint-ruint`"
);

#[cfg(not(any(feature = "uint-ethereum-types", feature = "uint-ruint")))]
compile_error!("A uint backend must be enabled: `uint-ethereum-types` or `uint-ruint`");

// ---------------------------------------------------------------------------
// Backend selection
// ---------------------------------------------------------------------------

#[cfg(feature = "uint-ethereum-types")]
mod backend {
    pub use ethereum_types::{U256, U512};
}

#[cfg(feature = "uint-ruint")]
mod backend {
    pub type U256 = ruint::Uint<256, 4>;
    pub type U512 = ruint::Uint<512, 8>;
}

// ---------------------------------------------------------------------------
// ParseU256Error
// ---------------------------------------------------------------------------

/// Error type for U256 string parsing.
#[derive(Debug)]
pub enum ParseU256Error {
    #[cfg(feature = "uint-ethereum-types")]
    EthereumTypes(ethereum_types::FromDecStrErr),
    #[cfg(feature = "uint-ruint")]
    Ruint(ruint::ParseError),
}

impl core::fmt::Display for ParseU256Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            #[cfg(feature = "uint-ethereum-types")]
            Self::EthereumTypes(e) => write!(f, "{e}"),
            #[cfg(feature = "uint-ruint")]
            Self::Ruint(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ParseU256Error {}

// ---------------------------------------------------------------------------
// U256
// ---------------------------------------------------------------------------

/// 256-bit unsigned integer. Backend selected at compile time.
#[repr(transparent)]
#[derive(
    Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct U256(backend::U256);

// ---- constants & constructors ----

impl U256 {
    pub const ZERO: Self = Self::from_limbs([0; 4]);
    pub const ONE: Self = Self::from_limbs([1, 0, 0, 0]);
    pub const MAX: Self = Self::from_limbs([u64::MAX; 4]);

    /// Construct from four u64 limbs in little-endian word order (limbs[0] is least significant).
    #[inline]
    pub const fn from_limbs(limbs: [u64; 4]) -> Self {
        #[cfg(feature = "uint-ethereum-types")]
        {
            Self(ethereum_types::U256(limbs))
        }
        #[cfg(feature = "uint-ruint")]
        {
            Self(backend::U256::from_limbs(limbs))
        }
    }

    /// Reference to internal limbs in little-endian word order.
    #[inline]
    pub fn as_limbs(&self) -> &[u64; 4] {
        #[cfg(feature = "uint-ethereum-types")]
        {
            &self.0.0
        }
        #[cfg(feature = "uint-ruint")]
        {
            self.0.as_limbs()
        }
    }

    /// Mutable reference to internal limbs.
    #[inline]
    pub fn as_limbs_mut(&mut self) -> &mut [u64; 4] {
        #[cfg(feature = "uint-ethereum-types")]
        {
            &mut self.0.0
        }
        #[cfg(feature = "uint-ruint")]
        {
            // SAFETY: mutating limbs in-place is safe as long as we don't break invariants.
            // ruint marks this unsafe because arbitrary writes could create invalid state,
            // but our callers only copy limb values from other valid U256s.
            unsafe { self.0.as_limbs_mut() }
        }
    }

    // Compatibility methods (match ethereum_types API)

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
        #[cfg(feature = "uint-ethereum-types")]
        {
            Self(ethereum_types::U256::from_big_endian(bytes))
        }
        #[cfg(feature = "uint-ruint")]
        {
            // alloy expects exactly 32 bytes via from_be_slice, pad if shorter
            if bytes.len() >= 32 {
                Self(backend::U256::from_be_slice(&bytes[bytes.len() - 32..]))
            } else {
                let mut padded = [0u8; 32];
                padded[32 - bytes.len()..].copy_from_slice(bytes);
                Self(backend::U256::from_be_slice(&padded))
            }
        }
    }

    /// Convert to 32-byte big-endian representation.
    #[inline]
    pub fn to_big_endian(self) -> [u8; 32] {
        #[cfg(feature = "uint-ethereum-types")]
        {
            self.0.to_big_endian()
        }
        #[cfg(feature = "uint-ruint")]
        {
            self.0.to_be_bytes::<32>()
        }
    }

    /// Construct from little-endian bytes.
    #[inline]
    pub fn from_little_endian(bytes: &[u8]) -> Self {
        #[cfg(feature = "uint-ethereum-types")]
        {
            Self(ethereum_types::U256::from_little_endian(bytes))
        }
        #[cfg(feature = "uint-ruint")]
        {
            if bytes.len() >= 32 {
                Self(backend::U256::from_le_slice(&bytes[..32]))
            } else {
                let mut padded = [0u8; 32];
                padded[..bytes.len()].copy_from_slice(bytes);
                Self(backend::U256::from_le_slice(&padded))
            }
        }
    }
}

// ---- arithmetic ----

impl U256 {
    #[inline]
    pub fn overflowing_add(self, rhs: Self) -> (Self, bool) {
        let (v, o) = self.0.overflowing_add(rhs.0);
        (Self(v), o)
    }

    #[inline]
    pub fn overflowing_sub(self, rhs: Self) -> (Self, bool) {
        let (v, o) = self.0.overflowing_sub(rhs.0);
        (Self(v), o)
    }

    #[inline]
    pub fn overflowing_mul(self, rhs: Self) -> (Self, bool) {
        let (v, o) = self.0.overflowing_mul(rhs.0);
        (Self(v), o)
    }

    #[inline]
    pub fn overflowing_pow(self, exp: Self) -> (Self, bool) {
        #[cfg(feature = "uint-ethereum-types")]
        {
            let (v, o) = self.0.overflowing_pow(exp.0);
            (Self(v), o)
        }
        #[cfg(feature = "uint-ruint")]
        {
            let (v, o) = self.0.overflowing_pow(exp.0);
            (Self(v), o)
        }
    }

    #[inline]
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    #[inline]
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }

    #[inline]
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        self.0.checked_mul(rhs.0).map(Self)
    }

    #[inline]
    pub fn checked_div(self, rhs: Self) -> Option<Self> {
        self.0.checked_div(rhs.0).map(Self)
    }

    #[inline]
    pub fn checked_rem(self, rhs: Self) -> Option<Self> {
        self.0.checked_rem(rhs.0).map(Self)
    }

    #[inline]
    pub fn saturating_add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }

    #[inline]
    pub fn saturating_mul(self, rhs: Self) -> Self {
        Self(self.0.saturating_mul(rhs.0))
    }

    #[inline]
    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
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
        #[cfg(feature = "uint-ethereum-types")]
        {
            self.0.bit(index)
        }
        #[cfg(feature = "uint-ruint")]
        {
            self.0.bit(index)
        }
    }

    /// Returns the byte at the given index (0 = most significant byte, EVM convention).
    #[inline]
    pub fn byte(&self, index: usize) -> u8 {
        #[cfg(feature = "uint-ethereum-types")]
        {
            self.0.byte(index)
        }
        #[cfg(feature = "uint-ruint")]
        {
            self.0.byte(31 - index)
        }
    }

    /// Number of leading zero bits.
    #[inline]
    pub fn leading_zeros(&self) -> u32 {
        #[cfg(feature = "uint-ethereum-types")]
        {
            self.0.leading_zeros()
        }
        #[cfg(feature = "uint-ruint")]
        {
            self.0.leading_zeros() as u32
        }
    }

    /// Returns true if the value is zero.
    #[inline]
    pub fn is_zero(&self) -> bool {
        self.0.is_zero()
    }

    /// Number of bits needed to represent this value.
    #[inline]
    pub fn bits(&self) -> usize {
        #[cfg(feature = "uint-ethereum-types")]
        {
            self.0.bits()
        }
        #[cfg(feature = "uint-ruint")]
        {
            self.0.bit_len()
        }
    }
}

// ---- numeric conversion ----

impl U256 {
    /// Returns the low 64 bits.
    #[inline]
    pub fn as_u64(&self) -> u64 {
        #[cfg(feature = "uint-ethereum-types")]
        {
            self.0.as_u64()
        }
        #[cfg(feature = "uint-ruint")]
        {
            self.0.as_limbs()[0]
        }
    }

    /// Returns the low 32 bits.
    #[inline]
    pub fn low_u32(&self) -> u32 {
        #[cfg(feature = "uint-ethereum-types")]
        {
            self.0.low_u32()
        }
        #[cfg(feature = "uint-ruint")]
        {
            self.0.as_limbs()[0] as u32
        }
    }

    /// Returns the low 64 bits.
    #[inline]
    pub fn low_u64(&self) -> u64 {
        #[cfg(feature = "uint-ethereum-types")]
        {
            self.0.low_u64()
        }
        #[cfg(feature = "uint-ruint")]
        {
            self.0.as_limbs()[0]
        }
    }

    /// Returns the low bits as usize.
    #[inline]
    pub fn as_usize(&self) -> usize {
        #[cfg(feature = "uint-ethereum-types")]
        {
            self.0.as_usize()
        }
        #[cfg(feature = "uint-ruint")]
        {
            self.0.as_limbs()[0] as usize
        }
    }

    /// Parse from decimal string.
    #[inline]
    pub fn from_dec_str(s: &str) -> Result<Self, ParseU256Error> {
        #[cfg(feature = "uint-ethereum-types")]
        {
            ethereum_types::U256::from_dec_str(s)
                .map(Self)
                .map_err(ParseU256Error::EthereumTypes)
        }
        #[cfg(feature = "uint-ruint")]
        {
            backend::U256::from_str_radix(s, 10)
                .map(Self)
                .map_err(ParseU256Error::Ruint)
        }
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
        #[cfg(feature = "uint-ethereum-types")]
        {
            ethereum_types::U256::from_str_radix(s, radix)
                .map(Self)
                .map_err(|_| {
                    // ethereum_types::FromStrRadixErr doesn't implement Error;
                    // convert through from_dec_str error type for consistent wrapping.
                    ParseU256Error::EthereumTypes(ethereum_types::FromDecStrErr::InvalidLength)
                })
        }
        #[cfg(feature = "uint-ruint")]
        {
            backend::U256::from_str_radix(s, radix as u64)
                .map(Self)
                .map_err(ParseU256Error::Ruint)
        }
    }
}

// ---- From impls ----

macro_rules! impl_from_uint {
    ($($t:ty),*) => {
        $(
            impl From<$t> for U256 {
                #[inline]
                fn from(v: $t) -> Self {
                    Self(backend::U256::from(v))
                }
            }
        )*
    };
}

impl_from_uint!(u8, u16, u32, u64, usize, u128);

// Signed conversions need special handling: ruint panics on negative values,
// but ethereum_types sign-extends (e.g., -1i32 → U256::MAX).
// The EVM relies on two's complement sign extension for signed arithmetic.
impl From<i32> for U256 {
    #[inline]
    fn from(v: i32) -> Self {
        if v >= 0 {
            Self::from(v as u64)
        } else {
            // Two's complement: -n = NOT(n-1) = MAX - (n-1)
            // -1 → MAX, -2 → MAX-1, etc.
            let abs_minus_one = (!(v as u32)) as u64; // = |v| - 1
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
            let abs_minus_one = (!(v as u64)) as u64;
            Self::MAX - Self::from(abs_minus_one)
        }
    }
}

impl From<bool> for U256 {
    #[inline]
    fn from(v: bool) -> Self {
        Self(backend::U256::from(v as u64))
    }
}

// ---- TryFrom impls (reverse conversions) ----

impl TryFrom<U256> for u64 {
    type Error = &'static str;
    #[inline]
    fn try_from(v: U256) -> Result<Self, Self::Error> {
        let limbs = v.as_limbs();
        if limbs[1] != 0 || limbs[2] != 0 || limbs[3] != 0 {
            Err("U256 value too large for u64")
        } else {
            Ok(limbs[0])
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

macro_rules! impl_binop {
    ($trait:ident, $method:ident) => {
        impl core::ops::$trait for U256 {
            type Output = Self;
            #[inline]
            fn $method(self, rhs: Self) -> Self {
                Self(core::ops::$trait::$method(self.0, rhs.0))
            }
        }
    };
}

macro_rules! impl_binop_assign {
    ($trait:ident, $method:ident) => {
        impl core::ops::$trait for U256 {
            #[inline]
            fn $method(&mut self, rhs: Self) {
                core::ops::$trait::$method(&mut self.0, rhs.0);
            }
        }
    };
}

impl_binop!(Add, add);
impl_binop!(Sub, sub);
impl_binop!(Mul, mul);
impl_binop!(Div, div);
impl_binop!(Rem, rem);
impl_binop!(BitAnd, bitand);
impl_binop!(BitOr, bitor);
impl_binop!(BitXor, bitxor);

// Cross-type arithmetic (U256 op primitive). ethereum_types supports these.
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
impl_cross_binop!(i32, Add, add);
impl_cross_binop!(i32, Sub, sub);
impl_cross_binop!(i32, Mul, mul);
impl_cross_binop!(i32, Div, div);

impl_binop_assign!(AddAssign, add_assign);
impl_binop_assign!(SubAssign, sub_assign);
impl_binop_assign!(MulAssign, mul_assign);
impl_binop_assign!(DivAssign, div_assign);
impl_binop_assign!(RemAssign, rem_assign);
impl_binop_assign!(BitAndAssign, bitand_assign);
impl_binop_assign!(BitOrAssign, bitor_assign);
impl_binop_assign!(BitXorAssign, bitxor_assign);

impl core::ops::Not for U256 {
    type Output = Self;
    #[inline]
    fn not(self) -> Self {
        Self(!self.0)
    }
}

impl core::ops::Shl<usize> for U256 {
    type Output = Self;
    #[inline]
    fn shl(self, rhs: usize) -> Self {
        Self(self.0 << rhs)
    }
}

impl core::ops::Shr<usize> for U256 {
    type Output = Self;
    #[inline]
    fn shr(self, rhs: usize) -> Self {
        Self(self.0 >> rhs)
    }
}

// Additional shift impls for common integer types (matching ethereum_types)
macro_rules! impl_shift {
    ($($t:ty),*) => {
        $(
            impl core::ops::Shl<$t> for U256 {
                type Output = Self;
                #[inline]
                fn shl(self, rhs: $t) -> Self {
                    Self(self.0 << rhs)
                }
            }
            impl core::ops::Shr<$t> for U256 {
                type Output = Self;
                #[inline]
                fn shr(self, rhs: $t) -> Self {
                    Self(self.0 >> rhs)
                }
            }
        )*
    };
}

impl_shift!(u8, u16, u32, u64, i32, i64);

impl core::ops::ShlAssign<usize> for U256 {
    #[inline]
    fn shl_assign(&mut self, rhs: usize) {
        self.0 <<= rhs;
    }
}

impl core::ops::ShrAssign<usize> for U256 {
    #[inline]
    fn shr_assign(&mut self, rhs: usize) {
        self.0 >>= rhs;
    }
}

// ---- formatting ----

impl core::fmt::Debug for U256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(&self.0, f)
    }
}

impl core::fmt::Display for U256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(&self.0, f)
    }
}

impl core::fmt::LowerHex for U256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::LowerHex::fmt(&self.0, f)
    }
}

impl core::fmt::UpperHex for U256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::UpperHex::fmt(&self.0, f)
    }
}

// ---- conversion to/from inner type (for interop with ethereum_types in specific backends) ----

#[cfg(feature = "uint-ethereum-types")]
impl U256 {
    /// Access the underlying `ethereum_types::U256`. Only available with the ethereum-types backend.
    #[inline]
    pub fn inner(&self) -> &ethereum_types::U256 {
        &self.0
    }

    /// Construct from an `ethereum_types::U256`.
    #[inline]
    pub fn from_inner(inner: ethereum_types::U256) -> Self {
        Self(inner)
    }
}

// ---------------------------------------------------------------------------
// BigEndianHash interop
// ---------------------------------------------------------------------------

/// Implement BigEndianHash-like conversion between H256 and our U256.
/// This avoids depending on the ethereum_types BigEndianHash trait directly.
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

/// Implement the BigEndianHash trait from ethereum_types for our wrapper.
impl ethereum_types::BigEndianHash for U256 {
    type Uint = Self;

    fn from_uint(value: &Self) -> Self {
        *value
    }

    fn into_uint(&self) -> Self {
        *self
    }
}

// Conversions between our U256 and ethereum_types::U256 (for BigEndianHash interop)
#[cfg(feature = "uint-ethereum-types")]
impl From<U256> for ethereum_types::U256 {
    #[inline]
    fn from(v: U256) -> Self {
        v.0
    }
}

// ---------------------------------------------------------------------------
// U512
// ---------------------------------------------------------------------------

/// 512-bit unsigned integer. Used for intermediate ADDMOD arithmetic.
#[repr(transparent)]
#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct U512(backend::U512);

impl U512 {
    #[inline]
    pub fn overflowing_add(self, rhs: Self) -> (Self, bool) {
        let (v, o) = self.0.overflowing_add(rhs.0);
        (Self(v), o)
    }

    /// Extract the low 256 bits as a U256.
    #[inline]
    pub fn low_u256(self) -> U256 {
        #[cfg(feature = "uint-ethereum-types")]
        {
            // ethereum_types::U512 stores as [u64; 8], low limbs first
            U256::from_limbs([self.0.0[0], self.0.0[1], self.0.0[2], self.0.0[3]])
        }
        #[cfg(feature = "uint-ruint")]
        {
            let limbs = self.0.as_limbs();
            U256::from_limbs([limbs[0], limbs[1], limbs[2], limbs[3]])
        }
    }

    /// Reference to internal limbs.
    #[inline]
    pub fn as_limbs(&self) -> &[u64; 8] {
        #[cfg(feature = "uint-ethereum-types")]
        {
            &self.0.0
        }
        #[cfg(feature = "uint-ruint")]
        {
            self.0.as_limbs()
        }
    }
}

impl From<U256> for U512 {
    #[inline]
    fn from(v: U256) -> Self {
        Self(backend::U512::from(v.0))
    }
}

impl core::ops::Rem for U512 {
    type Output = Self;
    #[inline]
    fn rem(self, rhs: Self) -> Self {
        Self(self.0 % rhs.0)
    }
}

/// U512 % U256 — used in ADDMOD opcode.
/// Converts the U256 modulus to U512 for the operation.
impl core::ops::Rem<U256> for U512 {
    type Output = Self;
    #[inline]
    fn rem(self, rhs: U256) -> Self {
        Self(self.0 % backend::U512::from(rhs.0))
    }
}

impl core::fmt::Debug for U512 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(&self.0, f)
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
        // Inlined from ethrex_rlp::encode::impl_length_integers
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
