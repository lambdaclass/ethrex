//! Minimal JitContext for stencil compilation.
//!
//! This is a copy of the essential JitContext fields needed by stencils.
//! It must match the layout of the main JitContext exactly.

/// U256 representation - 4 x u64 limbs, little-endian
/// Must match ethrex_common::U256 layout
#[repr(C)]
#[derive(Clone, Copy)]
pub struct U256(pub [u64; 4]);

impl U256 {
    pub const ZERO: Self = Self([0, 0, 0, 0]);

    #[inline(always)]
    pub fn wrapping_add(self, rhs: Self) -> Self {
        let mut result = [0u64; 4];
        let mut carry = 0u64;

        // Limb 0
        let (sum, c1) = self.0[0].overflowing_add(rhs.0[0]);
        let (sum, c2) = sum.overflowing_add(carry);
        result[0] = sum;
        carry = u64::from(c1) + u64::from(c2);

        // Limb 1
        let (sum, c1) = self.0[1].overflowing_add(rhs.0[1]);
        let (sum, c2) = sum.overflowing_add(carry);
        result[1] = sum;
        carry = u64::from(c1) + u64::from(c2);

        // Limb 2
        let (sum, c1) = self.0[2].overflowing_add(rhs.0[2]);
        let (sum, c2) = sum.overflowing_add(carry);
        result[2] = sum;
        carry = u64::from(c1) + u64::from(c2);

        // Limb 3
        let (sum, c1) = self.0[3].overflowing_add(rhs.0[3]);
        let (sum, _c2) = sum.overflowing_add(carry);
        result[3] = sum;
        // Discard final carry for wrapping semantics

        Self(result)
    }

    #[inline(always)]
    pub fn wrapping_sub(self, rhs: Self) -> Self {
        let mut result = [0u64; 4];
        let mut borrow = 0u64;

        // Limb 0
        let (diff, b1) = self.0[0].overflowing_sub(rhs.0[0]);
        let (diff, b2) = diff.overflowing_sub(borrow);
        result[0] = diff;
        borrow = u64::from(b1) + u64::from(b2);

        // Limb 1
        let (diff, b1) = self.0[1].overflowing_sub(rhs.0[1]);
        let (diff, b2) = diff.overflowing_sub(borrow);
        result[1] = diff;
        borrow = u64::from(b1) + u64::from(b2);

        // Limb 2
        let (diff, b1) = self.0[2].overflowing_sub(rhs.0[2]);
        let (diff, b2) = diff.overflowing_sub(borrow);
        result[2] = diff;
        borrow = u64::from(b1) + u64::from(b2);

        // Limb 3
        let (diff, b1) = self.0[3].overflowing_sub(rhs.0[3]);
        let (diff, _b2) = diff.overflowing_sub(borrow);
        result[3] = diff;

        Self(result)
    }

    #[inline(always)]
    pub fn wrapping_mul(self, rhs: Self) -> Self {
        // Full 256-bit multiplication keeping only lower 256 bits
        // This is a simplified schoolbook multiplication
        let mut result = [0u64; 4];

        // For wrapping mul, we only need products that fit in 256 bits
        // a0*b0 -> r0, r1
        // a0*b1 + a1*b0 -> r1, r2
        // a0*b2 + a1*b1 + a2*b0 -> r2, r3
        // a0*b3 + a1*b2 + a2*b1 + a3*b0 -> r3 (discard overflow)

        let a = self.0;
        let b = rhs.0;

        // Helper for 64x64 -> 128 bit multiply
        #[inline(always)]
        fn mul64(x: u64, y: u64) -> (u64, u64) {
            let full = (x as u128) * (y as u128);
            (full as u64, (full >> 64) as u64)
        }

        // Helper for adding with carry
        #[inline(always)]
        fn adc(a: u64, b: u64, carry: u64) -> (u64, u64) {
            let sum = (a as u128) + (b as u128) + (carry as u128);
            (sum as u64, (sum >> 64) as u64)
        }

        // r0 = a0*b0 (low)
        let (lo, mut carry) = mul64(a[0], b[0]);
        result[0] = lo;

        // r1 = a0*b0 (high) + a0*b1 (low) + a1*b0 (low)
        let (p01_lo, p01_hi) = mul64(a[0], b[1]);
        let (p10_lo, p10_hi) = mul64(a[1], b[0]);
        let (sum, c1) = adc(carry, p01_lo, 0);
        let (sum, c2) = adc(sum, p10_lo, 0);
        result[1] = sum;
        carry = c1 + c2 + p01_hi + p10_hi;

        // r2 = carry + a0*b2 (low) + a1*b1 (low) + a2*b0 (low)
        let (p02_lo, p02_hi) = mul64(a[0], b[2]);
        let (p11_lo, p11_hi) = mul64(a[1], b[1]);
        let (p20_lo, p20_hi) = mul64(a[2], b[0]);
        let (sum, c1) = adc(carry, p02_lo, 0);
        let (sum, c2) = adc(sum, p11_lo, 0);
        let (sum, c3) = adc(sum, p20_lo, 0);
        result[2] = sum;
        carry = c1 + c2 + c3 + p02_hi + p11_hi + p20_hi;

        // r3 = carry + a0*b3 (low) + a1*b2 (low) + a2*b1 (low) + a3*b0 (low)
        let (p03_lo, _) = mul64(a[0], b[3]);
        let (p12_lo, _) = mul64(a[1], b[2]);
        let (p21_lo, _) = mul64(a[2], b[1]);
        let (p30_lo, _) = mul64(a[3], b[0]);
        let (sum, c1) = adc(carry, p03_lo, 0);
        let (sum, c2) = adc(sum, p12_lo, 0);
        let (sum, c3) = adc(sum, p21_lo, 0);
        let (sum, _) = adc(sum, p30_lo, 0);
        result[3] = sum;

        Self(result)
    }
}

/// JitContext for stencils - must match main JitContext layout exactly
#[repr(C)]
pub struct JitContext {
    // Stack
    pub stack_values: *mut U256,
    pub stack_offset: usize,

    // Gas
    pub gas_remaining: i64,

    // Memory
    pub memory_ptr: *mut u8,
    pub memory_size: usize,
    pub memory_capacity: usize,

    // Bytecode
    pub pc: usize,
    pub bytecode: *const u8,
    pub bytecode_len: usize,

    // Jump table
    pub jump_table: *const *const u8,

    // VM pointer
    pub vm_ptr: *mut (),

    // Exit info
    pub exit_reason: u32,
    pub return_offset: usize,
    pub return_size: usize,
}

/// Stack limit constant
pub const STACK_LIMIT: usize = 1024;

/// Exit reasons
pub const EXIT_CONTINUE: u32 = 0;
pub const EXIT_STOP: u32 = 1;
pub const EXIT_RETURN: u32 = 2;
pub const EXIT_REVERT: u32 = 3;
pub const EXIT_OUT_OF_GAS: u32 = 4;
pub const EXIT_STACK_UNDERFLOW: u32 = 5;
pub const EXIT_STACK_OVERFLOW: u32 = 6;
pub const EXIT_INVALID_JUMP: u32 = 7;
pub const EXIT_TO_INTERPRETER: u32 = 8;

/// Gas costs
pub const GAS_ADD: i64 = 3;
pub const GAS_MUL: i64 = 5;
pub const GAS_SUB: i64 = 3;
pub const GAS_POP: i64 = 2;
pub const GAS_PUSH: i64 = 3;
