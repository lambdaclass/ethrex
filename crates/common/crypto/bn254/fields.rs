//! Arithmetic in the BN254 base field and its tower
//! `Fq2 = Fq[u]/(u^2 + 1)`, `Fq6 = Fq2[v]/(v^3 - (9 + u))`, `Fq12 = Fq6[w]/(w^2 - v)`.
//!
//! Elements use the representation of `ark-bn254` (little-endian limbs of
//! `x * 2^256 mod p`, always fully reduced), so conversions cost nothing and every
//! result is bit-for-bit comparable with arkworks.
//!
//! `ADX = false` runs the base field and `Fq2` operations through arkworks.
//! `ADX = true` uses x86-64 assembly for them, with no data-dependent branches:
//! arkworks ends each addition, subtraction and multiplication with a conditional
//! subtraction of the modulus, a branch that is taken about half of the time on
//! varied inputs and is often mispredicted. `Fq2` multiplication is two sums of
//! products with one Montgomery reduction each, instead of three multiplications.
//! Callers must only instantiate `ADX = true` after detecting BMI2 and ADX.

use ark_bn254::{Fq, Fq2, Fq6Config, Fq12Config};
use ark_ff::{BigInt, Field as _, Fp6Config as _, Fp12Config as _};
use core::ops::{Add, Mul, Neg, Sub};

/// The BN254 base field modulus.
const MODULUS: [u64; 4] = [
    0x3c20_8c16_d87c_fd47,
    0x9781_6a91_6871_ca8d,
    0xb850_45b6_8181_585d,
    0x3064_4e72_e131_a029,
];

#[cfg(target_arch = "x86_64")]
mod x86 {
    use super::MODULUS;

    /// The modulus followed by `-p^-1 mod 2^64`, read by the assembly.
    static MODULUS_AND_INV: [u64; 5] = [
        MODULUS[0],
        MODULUS[1],
        MODULUS[2],
        MODULUS[3],
        0x87d2_0782_e486_6389,
    ];

    /// `a + b mod p` for `a, b < p`, with `cmov` instead of a branch.
    #[inline(always)]
    pub(super) fn add(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        let [mut r0, mut r1, mut r2, mut r3] = a;
        // SAFETY: base x86-64 instructions on registers; the only memory read is the
        // modulus static.
        unsafe {
            core::arch::asm!(
                // The sum is below 2p < 2^255, so it needs no fifth word.
                "add {r0}, {b0}",
                "adc {r1}, {b1}",
                "adc {r2}, {b2}",
                "adc {r3}, {b3}",
                "mov {b0}, {r0}",
                "sub {b0}, qword ptr [rip + {q}]",
                "mov {b1}, {r1}",
                "sbb {b1}, qword ptr [rip + {q} + 8]",
                "mov {b2}, {r2}",
                "sbb {b2}, qword ptr [rip + {q} + 16]",
                "mov {b3}, {r3}",
                "sbb {b3}, qword ptr [rip + {q} + 24]",
                // Keep the difference unless subtracting p borrowed.
                "cmovnc {r0}, {b0}",
                "cmovnc {r1}, {b1}",
                "cmovnc {r2}, {b2}",
                "cmovnc {r3}, {b3}",
                r0 = inout(reg) r0,
                r1 = inout(reg) r1,
                r2 = inout(reg) r2,
                r3 = inout(reg) r3,
                b0 = inout(reg) b[0] => _,
                b1 = inout(reg) b[1] => _,
                b2 = inout(reg) b[2] => _,
                b3 = inout(reg) b[3] => _,
                q = sym MODULUS_AND_INV,
                options(pure, readonly, nostack),
            );
        }
        [r0, r1, r2, r3]
    }

    /// `a - b mod p` for `a, b < p`: adds `p` masked by the borrow.
    #[inline(always)]
    pub(super) fn sub(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
        let [mut r0, mut r1, mut r2, mut r3] = a;
        // SAFETY: base x86-64 instructions on registers; the only memory read is the
        // modulus static.
        unsafe {
            core::arch::asm!(
                "sub {r0}, {b0}",
                "sbb {r1}, {b1}",
                "sbb {r2}, {b2}",
                "sbb {r3}, {b3}",
                // mask = 0 - borrow: all ones when a < b.
                "sbb {mask}, {mask}",
                "mov {b0}, qword ptr [rip + {q}]",
                "and {b0}, {mask}",
                "mov {b1}, qword ptr [rip + {q} + 8]",
                "and {b1}, {mask}",
                "mov {b2}, qword ptr [rip + {q} + 16]",
                "and {b2}, {mask}",
                "mov {b3}, qword ptr [rip + {q} + 24]",
                "and {b3}, {mask}",
                "add {r0}, {b0}",
                "adc {r1}, {b1}",
                "adc {r2}, {b2}",
                "adc {r3}, {b3}",
                r0 = inout(reg) r0,
                r1 = inout(reg) r1,
                r2 = inout(reg) r2,
                r3 = inout(reg) r3,
                b0 = inout(reg) b[0] => _,
                b1 = inout(reg) b[1] => _,
                b2 = inout(reg) b[2] => _,
                b3 = inout(reg) b[3] => _,
                mask = out(reg) _,
                q = sym MODULUS_AND_INV,
                options(pure, readonly, nostack),
            );
        }
        [r0, r1, r2, r3]
    }

    /// Montgomery multiplication `a * b / 2^256 mod p` (CIOS without the final carry
    /// word, valid because the top limb of the modulus is below `2^62`).
    ///
    /// # Safety
    ///
    /// The CPU must support BMI2 and ADX.
    #[inline(always)]
    pub(super) unsafe fn mul(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
        let (r0, r1, r2, r3): (u64, u64, u64, u64);
        // SAFETY: the caller guarantees BMI2 and ADX; the asm only reads the two
        // arrays behind the pointers and the modulus static, and touches no stack.
        unsafe {
            core::arch::asm!(
                "mov rdx, qword ptr [{b} + 0]",
                "xor {z:e}, {z:e}",
                "mulx {t1}, {t0}, qword ptr [{a}]",
                "mulx {t2}, {l}, qword ptr [{a} + 8]",
                "adcx {t1}, {l}",
                "mulx {t3}, {l}, qword ptr [{a} + 16]",
                "adcx {t2}, {l}",
                "mulx {t4}, {l}, qword ptr [{a} + 24]",
                "adcx {t3}, {l}",
                "adcx {t4}, {z}",
                "mov rdx, {t0}",
                "imul rdx, qword ptr [rip + {q} + 32]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [rip + {q}]",
                "adcx {l}, {t0}",
                "adox {t1}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 8]",
                "adcx {t1}, {l}",
                "adox {t2}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 16]",
                "adcx {t2}, {l}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 24]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "adcx {t4}, {z}",
                "mov rdx, qword ptr [{b} + 8]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [{a}]",
                "adcx {t1}, {l}",
                "adox {t2}, {h}",
                "mulx {h}, {l}, qword ptr [{a} + 8]",
                "adcx {t2}, {l}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [{a} + 16]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {t0}, {l}, qword ptr [{a} + 24]",
                "adcx {t4}, {l}",
                "adox {t0}, {z}",
                "adcx {t0}, {z}",
                "mov rdx, {t1}",
                "imul rdx, qword ptr [rip + {q} + 32]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [rip + {q}]",
                "adcx {l}, {t1}",
                "adox {t2}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 8]",
                "adcx {t2}, {l}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 16]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 24]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "adcx {t0}, {z}",
                "mov rdx, qword ptr [{b} + 16]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [{a}]",
                "adcx {t2}, {l}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [{a} + 8]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [{a} + 16]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "mulx {t1}, {l}, qword ptr [{a} + 24]",
                "adcx {t0}, {l}",
                "adox {t1}, {z}",
                "adcx {t1}, {z}",
                "mov rdx, {t2}",
                "imul rdx, qword ptr [rip + {q} + 32]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [rip + {q}]",
                "adcx {l}, {t2}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 8]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 16]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 24]",
                "adcx {t0}, {l}",
                "adox {t1}, {h}",
                "adcx {t1}, {z}",
                "mov rdx, qword ptr [{b} + 24]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [{a}]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [{a} + 8]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "mulx {h}, {l}, qword ptr [{a} + 16]",
                "adcx {t0}, {l}",
                "adox {t1}, {h}",
                "mulx {t2}, {l}, qword ptr [{a} + 24]",
                "adcx {t1}, {l}",
                "adox {t2}, {z}",
                "adcx {t2}, {z}",
                "mov rdx, {t3}",
                "imul rdx, qword ptr [rip + {q} + 32]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [rip + {q}]",
                "adcx {l}, {t3}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 8]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 16]",
                "adcx {t0}, {l}",
                "adox {t1}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 24]",
                "adcx {t1}, {l}",
                "adox {t2}, {h}",
                "adcx {t2}, {z}",
                // The result (t4, t0, t1, t2) is below 2p: subtract p unless that borrows.
                "mov {h}, {t4}",
                "sub {h}, qword ptr [rip + {q}]",
                "mov {l}, {t0}",
                "sbb {l}, qword ptr [rip + {q} + 8]",
                "mov {z}, {t1}",
                "sbb {z}, qword ptr [rip + {q} + 16]",
                "mov rdx, {t2}",
                "sbb rdx, qword ptr [rip + {q} + 24]",
                "cmovc {h}, {t4}",
                "cmovc {l}, {t0}",
                "cmovc {z}, {t1}",
                "cmovc rdx, {t2}",
                a = in(reg) a.as_ptr(),
                b = in(reg) b.as_ptr(),
                q = sym MODULUS_AND_INV,
                t0 = out(reg) _,
                t1 = out(reg) _,
                t2 = out(reg) _,
                t3 = out(reg) _,
                t4 = out(reg) _,
                h = out(reg) r0,
                l = out(reg) r1,
                z = out(reg) r2,
                out("rdx") r3,
                options(nostack, readonly, pure),
            );
        }
        [r0, r1, r2, r3]
    }

    /// `(a0 * b0 + a1 * b1) / 2^256 mod p` with one Montgomery reduction: each row
    /// adds both products before reducing. For inputs below `p` the result before the
    /// final subtraction is `(a0 b0 + a1 b1 + m p) / 2^256` with `m < 2^256`, which is
    /// below `(2 p / 2^256 + 1) p < 1.4 p`, so one conditional subtraction reduces it.
    ///
    /// # Safety
    ///
    /// The CPU must support BMI2 and ADX.
    #[inline(always)]
    pub(super) unsafe fn sum_of_products(
        a0: &[u64; 4],
        b0: &[u64; 4],
        a1: &[u64; 4],
        b1: &[u64; 4],
    ) -> [u64; 4] {
        let (r0, r1, r2, r3): (u64, u64, u64, u64);
        // SAFETY: the caller guarantees BMI2 and ADX; the asm only reads the four
        // arrays behind the pointers and the modulus static, and touches no stack.
        unsafe {
            core::arch::asm!(
                "mov rdx, qword ptr [{b0} + 0]",
                "xor {z:e}, {z:e}",
                "mulx {t1}, {t0}, qword ptr [{a0}]",
                "mulx {t2}, {l}, qword ptr [{a0} + 8]",
                "adcx {t1}, {l}",
                "mulx {t3}, {l}, qword ptr [{a0} + 16]",
                "adcx {t2}, {l}",
                "mulx {t4}, {l}, qword ptr [{a0} + 24]",
                "adcx {t3}, {l}",
                "adcx {t4}, {z}",
                "mov rdx, qword ptr [{b1} + 0]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [{a1} + 0]",
                "adcx {t0}, {l}",
                "adox {t1}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 8]",
                "adcx {t1}, {l}",
                "adox {t2}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 16]",
                "adcx {t2}, {l}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 24]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "adcx {t4}, {z}",
                "mov rdx, {t0}",
                "imul rdx, qword ptr [rip + {q} + 32]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [rip + {q}]",
                "adcx {l}, {t0}",
                "adox {t1}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 8]",
                "adcx {t1}, {l}",
                "adox {t2}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 16]",
                "adcx {t2}, {l}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 24]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "adcx {t4}, {z}",
                "xor {t0:e}, {t0:e}",
                "mov rdx, qword ptr [{b0} + 8]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [{a0} + 0]",
                "adcx {t1}, {l}",
                "adox {t2}, {h}",
                "mulx {h}, {l}, qword ptr [{a0} + 8]",
                "adcx {t2}, {l}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [{a0} + 16]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [{a0} + 24]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "adcx {t0}, {z}",
                "mov rdx, qword ptr [{b1} + 8]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [{a1} + 0]",
                "adcx {t1}, {l}",
                "adox {t2}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 8]",
                "adcx {t2}, {l}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 16]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 24]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "adcx {t0}, {z}",
                "mov rdx, {t1}",
                "imul rdx, qword ptr [rip + {q} + 32]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [rip + {q}]",
                "adcx {l}, {t1}",
                "adox {t2}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 8]",
                "adcx {t2}, {l}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 16]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 24]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "adcx {t0}, {z}",
                "xor {t1:e}, {t1:e}",
                "mov rdx, qword ptr [{b0} + 16]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [{a0} + 0]",
                "adcx {t2}, {l}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [{a0} + 8]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [{a0} + 16]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "mulx {h}, {l}, qword ptr [{a0} + 24]",
                "adcx {t0}, {l}",
                "adox {t1}, {h}",
                "adcx {t1}, {z}",
                "mov rdx, qword ptr [{b1} + 16]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [{a1} + 0]",
                "adcx {t2}, {l}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 8]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 16]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 24]",
                "adcx {t0}, {l}",
                "adox {t1}, {h}",
                "adcx {t1}, {z}",
                "mov rdx, {t2}",
                "imul rdx, qword ptr [rip + {q} + 32]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [rip + {q}]",
                "adcx {l}, {t2}",
                "adox {t3}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 8]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 16]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 24]",
                "adcx {t0}, {l}",
                "adox {t1}, {h}",
                "adcx {t1}, {z}",
                "xor {t2:e}, {t2:e}",
                "mov rdx, qword ptr [{b0} + 24]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [{a0} + 0]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [{a0} + 8]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "mulx {h}, {l}, qword ptr [{a0} + 16]",
                "adcx {t0}, {l}",
                "adox {t1}, {h}",
                "mulx {h}, {l}, qword ptr [{a0} + 24]",
                "adcx {t1}, {l}",
                "adox {t2}, {h}",
                "adcx {t2}, {z}",
                "mov rdx, qword ptr [{b1} + 24]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [{a1} + 0]",
                "adcx {t3}, {l}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 8]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 16]",
                "adcx {t0}, {l}",
                "adox {t1}, {h}",
                "mulx {h}, {l}, qword ptr [{a1} + 24]",
                "adcx {t1}, {l}",
                "adox {t2}, {h}",
                "adcx {t2}, {z}",
                "mov rdx, {t3}",
                "imul rdx, qword ptr [rip + {q} + 32]",
                "xor {z:e}, {z:e}",
                "mulx {h}, {l}, qword ptr [rip + {q}]",
                "adcx {l}, {t3}",
                "adox {t4}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 8]",
                "adcx {t4}, {l}",
                "adox {t0}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 16]",
                "adcx {t0}, {l}",
                "adox {t1}, {h}",
                "mulx {h}, {l}, qword ptr [rip + {q} + 24]",
                "adcx {t1}, {l}",
                "adox {t2}, {h}",
                "adcx {t2}, {z}",
                "mov {h}, {t4}",
                "sub {h}, qword ptr [rip + {q}]",
                "mov {l}, {t0}",
                "sbb {l}, qword ptr [rip + {q} + 8]",
                "mov {z}, {t1}",
                "sbb {z}, qword ptr [rip + {q} + 16]",
                "mov rdx, {t2}",
                "sbb rdx, qword ptr [rip + {q} + 24]",
                "cmovnc {t4}, {h}",
                "cmovnc {t0}, {l}",
                "cmovnc {t1}, {z}",
                "cmovnc {t2}, rdx",
                a0 = in(reg) a0.as_ptr(),
                b0 = in(reg) b0.as_ptr(),
                a1 = in(reg) a1.as_ptr(),
                b1 = in(reg) b1.as_ptr(),
                q = sym MODULUS_AND_INV,
                t0 = out(reg) r1,
                t1 = out(reg) r2,
                t2 = out(reg) r3,
                t3 = out(reg) _,
                t4 = out(reg) r0,
                h = out(reg) _,
                l = out(reg) _,
                z = out(reg) _,
                out("rdx") _,
                options(nostack, readonly, pure),
            );
        }
        [r0, r1, r2, r3]
    }
}

/// `true` when the CPU can run the `ADX = true` backend (BMI2 and ADX).
pub(super) fn adx_supported() -> bool {
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    {
        std::is_x86_feature_detected!("bmi2") && std::is_x86_feature_detected!("adx")
    }
    #[cfg(not(all(target_arch = "x86_64", feature = "std")))]
    {
        false
    }
}

/// An element of `Fq` in Montgomery form.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Fe<const ADX: bool>(pub(super) [u64; 4]);

impl<const ADX: bool> Fe<ADX> {
    pub(super) const ZERO: Self = Self([0; 4]);
    pub(super) const ONE: Self = Self::from_ark(Fq::ONE);

    pub(super) const fn from_ark(x: Fq) -> Self {
        Self(x.0.0)
    }

    pub(super) fn to_ark(self) -> Fq {
        Fq::new_unchecked(BigInt(self.0))
    }

    #[inline(always)]
    pub(super) fn double(self) -> Self {
        self + self
    }

    #[inline(always)]
    pub(super) fn square(self) -> Self {
        if ADX {
            self * self
        } else {
            Self::from_ark(self.to_ark().square())
        }
    }

    /// `self / 2`. Montgomery form is linear, so halving the representative halves
    /// the value: add the (odd) modulus when the representative is odd, without a
    /// branch, then shift. Both are below `2^254`, so the sum fits in four limbs.
    #[inline(always)]
    pub(super) fn halve(self) -> Self {
        let x = self.0;
        let odd = (x[0] & 1).wrapping_neg();
        let mut sum = [0u64; 4];
        let mut carry = 0u64;
        for i in 0..4 {
            let t = u128::from(x[i]) + u128::from(MODULUS[i] & odd) + u128::from(carry);
            sum[i] = t as u64;
            carry = (t >> 64) as u64;
        }
        Self([
            (sum[0] >> 1) | (sum[1] << 63),
            (sum[1] >> 1) | (sum[2] << 63),
            (sum[2] >> 1) | (sum[3] << 63),
            sum[3] >> 1,
        ])
    }

    /// `None` for zero.
    pub(super) fn inverse(self) -> Option<Self> {
        super::inverse(&self.to_ark()).map(Self::from_ark)
    }
}

impl<const ADX: bool> Add for Fe<ADX> {
    type Output = Self;
    #[inline(always)]
    fn add(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        if ADX {
            return Self(x86::add(self.0, rhs.0));
        }
        Self::from_ark(self.to_ark() + rhs.to_ark())
    }
}

impl<const ADX: bool> Sub for Fe<ADX> {
    type Output = Self;
    #[inline(always)]
    fn sub(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        if ADX {
            return Self(x86::sub(self.0, rhs.0));
        }
        Self::from_ark(self.to_ark() - rhs.to_ark())
    }
}

impl<const ADX: bool> Neg for Fe<ADX> {
    type Output = Self;
    #[inline(always)]
    fn neg(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        if ADX {
            return Self(x86::sub([0; 4], self.0));
        }
        Self::from_ark(-self.to_ark())
    }
}

impl<const ADX: bool> Mul for Fe<ADX> {
    type Output = Self;
    #[inline(always)]
    fn mul(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        if ADX {
            // SAFETY: `ADX = true` is only instantiated after detecting BMI2 and ADX.
            return Self(unsafe { x86::mul(&self.0, &rhs.0) });
        }
        Self::from_ark(self.to_ark() * rhs.to_ark())
    }
}

/// An element `c0 + c1 * u` of `Fq2`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Fe2<const ADX: bool> {
    pub(super) c0: Fe<ADX>,
    pub(super) c1: Fe<ADX>,
}

impl<const ADX: bool> Fe2<ADX> {
    pub(super) const ZERO: Self = Self::new(Fe::ZERO, Fe::ZERO);
    pub(super) const ONE: Self = Self::new(Fe::ONE, Fe::ZERO);

    pub(super) const fn new(c0: Fe<ADX>, c1: Fe<ADX>) -> Self {
        Self { c0, c1 }
    }

    pub(super) const fn from_ark(x: Fq2) -> Self {
        Self::new(Fe::from_ark(x.c0), Fe::from_ark(x.c1))
    }

    pub(super) fn to_ark(self) -> Fq2 {
        Fq2::new(self.c0.to_ark(), self.c1.to_ark())
    }

    #[inline(always)]
    pub(super) fn double(self) -> Self {
        Self::new(self.c0.double(), self.c1.double())
    }

    #[inline(always)]
    pub(super) fn halve(self) -> Self {
        Self::new(self.c0.halve(), self.c1.halve())
    }

    #[inline(always)]
    pub(super) fn square(self) -> Self {
        if !ADX {
            return Self::from_ark(self.to_ark().square());
        }
        // (a + bu)^2 = (a + b)(a - b) + 2ab u
        let ab = self.c0 * self.c1;
        Self::new((self.c0 + self.c1) * (self.c0 - self.c1), ab.double())
    }

    #[inline(always)]
    pub(super) fn mul_by_fe(self, s: Fe<ADX>) -> Self {
        Self::new(self.c0 * s, self.c1 * s)
    }

    /// Multiplication by the `Fq6` non-residue `9 + u`.
    #[inline(always)]
    pub(super) fn mul_by_nonresidue(self) -> Self {
        if !ADX {
            return Self::from_ark(Fq6Config::mul_fp2_by_nonresidue(self.to_ark()));
        }
        let nine = |x: Fe<ADX>| x.double().double().double() + x;
        Self::new(nine(self.c0) - self.c1, self.c0 + nine(self.c1))
    }

    #[inline(always)]
    pub(super) fn conjugate(self) -> Self {
        Self::new(self.c0, -self.c1)
    }

    /// The Frobenius map `x -> x^(p^power)`.
    #[inline(always)]
    pub(super) fn frobenius_map(self, power: usize) -> Self {
        if power % 2 == 1 {
            self.conjugate()
        } else {
            self
        }
    }

    pub(super) fn inverse(self) -> Option<Self> {
        let norm = self.c0.square() + self.c1.square();
        let inv = norm.inverse()?;
        Some(Self::new(self.c0 * inv, -(self.c1 * inv)))
    }
}

impl<const ADX: bool> Add for Fe2<ADX> {
    type Output = Self;
    #[inline(always)]
    fn add(self, rhs: Self) -> Self {
        Self::new(self.c0 + rhs.c0, self.c1 + rhs.c1)
    }
}

impl<const ADX: bool> Sub for Fe2<ADX> {
    type Output = Self;
    #[inline(always)]
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.c0 - rhs.c0, self.c1 - rhs.c1)
    }
}

impl<const ADX: bool> Neg for Fe2<ADX> {
    type Output = Self;
    #[inline(always)]
    fn neg(self) -> Self {
        Self::new(-self.c0, -self.c1)
    }
}

impl<const ADX: bool> Mul for Fe2<ADX> {
    type Output = Self;
    #[inline(always)]
    fn mul(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        if ADX {
            // (a0 + a1 u)(b0 + b1 u) = (a0 b0 - a1 b1) + (a0 b1 + a1 b0) u
            let neg_a1 = (-self.c1).0;
            // SAFETY: `ADX = true` is only instantiated after detecting BMI2 and ADX.
            let (c0, c1) = unsafe {
                (
                    x86::sum_of_products(&self.c0.0, &rhs.c0.0, &neg_a1, &rhs.c1.0),
                    x86::sum_of_products(&self.c0.0, &rhs.c1.0, &self.c1.0, &rhs.c0.0),
                )
            };
            return Self::new(Fe(c0), Fe(c1));
        }
        Self::from_ark(self.to_ark() * rhs.to_ark())
    }
}

/// An element `c0 + c1 * v + c2 * v^2` of `Fq6`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Fe6<const ADX: bool> {
    pub(super) c0: Fe2<ADX>,
    pub(super) c1: Fe2<ADX>,
    pub(super) c2: Fe2<ADX>,
}

impl<const ADX: bool> Fe6<ADX> {
    pub(super) const ZERO: Self = Self::new(Fe2::ZERO, Fe2::ZERO, Fe2::ZERO);
    pub(super) const ONE: Self = Self::new(Fe2::ONE, Fe2::ZERO, Fe2::ZERO);

    /// `(9 + u)^((p^i - 1) / 3)` and `(9 + u)^((2p^i - 2) / 3)` for `i = 0..6`.
    const FROBENIUS_C1: [Fe2<ADX>; 6] = frobenius_coefficients(Fq6Config::FROBENIUS_COEFF_FP6_C1);
    const FROBENIUS_C2: [Fe2<ADX>; 6] = frobenius_coefficients(Fq6Config::FROBENIUS_COEFF_FP6_C2);

    pub(super) const fn new(c0: Fe2<ADX>, c1: Fe2<ADX>, c2: Fe2<ADX>) -> Self {
        Self { c0, c1, c2 }
    }

    #[cfg(test)]
    pub(super) const fn from_ark(x: ark_bn254::Fq6) -> Self {
        Self::new(
            Fe2::from_ark(x.c0),
            Fe2::from_ark(x.c1),
            Fe2::from_ark(x.c2),
        )
    }

    #[cfg(test)]
    pub(super) fn to_ark(self) -> ark_bn254::Fq6 {
        ark_bn254::Fq6::new(self.c0.to_ark(), self.c1.to_ark(), self.c2.to_ark())
    }

    #[inline]
    pub(super) fn double(self) -> Self {
        Self::new(self.c0.double(), self.c1.double(), self.c2.double())
    }

    /// Multiplication by `v`.
    #[inline]
    pub(super) fn mul_by_nonresidue(self) -> Self {
        Self::new(self.c2.mul_by_nonresidue(), self.c0, self.c1)
    }

    /// Multiplication by `c0 + c1 * v`.
    #[inline]
    pub(super) fn mul_by_01(self, c0: Fe2<ADX>, c1: Fe2<ADX>) -> Self {
        let a_a = self.c0 * c0;
        let b_b = self.c1 * c1;
        let t1 = (c1 * (self.c1 + self.c2) - b_b).mul_by_nonresidue() + a_a;
        let t3 = c0 * (self.c0 + self.c2) - a_a + b_b;
        let t2 = (c0 + c1) * (self.c0 + self.c1) - a_a - b_b;
        Self::new(t1, t2, t3)
    }

    #[inline]
    pub(super) fn square(self) -> Self {
        // Chung-Hasan SQR2.
        let s0 = self.c0.square();
        let s1 = (self.c0 * self.c1).double();
        let s2 = (self.c0 - self.c1 + self.c2).square();
        let s3 = (self.c1 * self.c2).double();
        let s4 = self.c2.square();
        Self::new(
            s0 + s3.mul_by_nonresidue(),
            s1 + s4.mul_by_nonresidue(),
            s1 + s2 + s3 - s0 - s4,
        )
    }

    pub(super) fn frobenius_map(self, power: usize) -> Self {
        Self::new(
            self.c0.frobenius_map(power),
            self.c1.frobenius_map(power) * Self::FROBENIUS_C1[power % 6],
            self.c2.frobenius_map(power) * Self::FROBENIUS_C2[power % 6],
        )
    }

    pub(super) fn inverse(self) -> Option<Self> {
        let t0 = self.c0.square() - (self.c1 * self.c2).mul_by_nonresidue();
        let t1 = self.c2.square().mul_by_nonresidue() - self.c0 * self.c1;
        let t2 = self.c1.square() - self.c0 * self.c2;
        let norm = self.c0 * t0 + (self.c2 * t1 + self.c1 * t2).mul_by_nonresidue();
        let inv = norm.inverse()?;
        Some(Self::new(t0 * inv, t1 * inv, t2 * inv))
    }
}

const fn frobenius_coefficients<const ADX: bool, const N: usize>(ark: &[Fq2]) -> [Fe2<ADX>; N] {
    let mut out = [Fe2::ZERO; N];
    let mut i = 0;
    while i < N {
        out[i] = Fe2::from_ark(ark[i]);
        i += 1;
    }
    out
}

impl<const ADX: bool> Add for Fe6<ADX> {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self::new(self.c0 + rhs.c0, self.c1 + rhs.c1, self.c2 + rhs.c2)
    }
}

impl<const ADX: bool> Sub for Fe6<ADX> {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.c0 - rhs.c0, self.c1 - rhs.c1, self.c2 - rhs.c2)
    }
}

impl<const ADX: bool> Neg for Fe6<ADX> {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self::new(-self.c0, -self.c1, -self.c2)
    }
}

impl<const ADX: bool> Mul for Fe6<ADX> {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        // Karatsuba (Devegili, O hEigeartaigh, Scott and Dahab, section 4).
        let ad = self.c0 * rhs.c0;
        let be = self.c1 * rhs.c1;
        let cf = self.c2 * rhs.c2;
        let x = (self.c1 + self.c2) * (rhs.c1 + rhs.c2) - be - cf;
        let y = (self.c0 + self.c1) * (rhs.c0 + rhs.c1) - ad - be;
        let z = (self.c0 + self.c2) * (rhs.c0 + rhs.c2) - ad + be - cf;
        Self::new(ad + x.mul_by_nonresidue(), y + cf.mul_by_nonresidue(), z)
    }
}

/// An element `c0 + c1 * w` of `Fq12`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Fe12<const ADX: bool> {
    pub(super) c0: Fe6<ADX>,
    pub(super) c1: Fe6<ADX>,
}

impl<const ADX: bool> Fe12<ADX> {
    pub(super) const ONE: Self = Self::new(Fe6::ONE, Fe6::ZERO);

    /// `(9 + u)^((p^i - 1) / 6)` for `i = 0..12`.
    const FROBENIUS_C1: [Fe2<ADX>; 12] =
        frobenius_coefficients(Fq12Config::FROBENIUS_COEFF_FP12_C1);

    pub(super) const fn new(c0: Fe6<ADX>, c1: Fe6<ADX>) -> Self {
        Self { c0, c1 }
    }

    #[cfg(test)]
    pub(super) const fn from_ark(x: ark_bn254::Fq12) -> Self {
        Self::new(Fe6::from_ark(x.c0), Fe6::from_ark(x.c1))
    }

    #[cfg(test)]
    pub(super) fn to_ark(self) -> ark_bn254::Fq12 {
        ark_bn254::Fq12::new(self.c0.to_ark(), self.c1.to_ark())
    }

    pub(super) fn is_one(&self) -> bool {
        *self == Self::ONE
    }

    /// `x^(p^6)`, which inverts an element of the cyclotomic subgroup.
    #[inline]
    pub(super) fn conjugate(self) -> Self {
        Self::new(self.c0, -self.c1)
    }

    pub(super) fn square(self) -> Self {
        // Complex squaring with w^2 = v.
        let v0 = self.c0 - self.c1;
        let v3 = self.c0 - self.c1.mul_by_nonresidue();
        let v2 = self.c0 * self.c1;
        let v0 = v0 * v3 + v2;
        Self::new(v0 + v2.mul_by_nonresidue(), v2.double())
    }

    /// Multiplication by the sparse element `c0 + (c3 + c4 * v) * w`.
    #[inline]
    pub(super) fn mul_by_034(self, c0: Fe2<ADX>, c3: Fe2<ADX>, c4: Fe2<ADX>) -> Self {
        let a = Fe6::new(self.c0.c0 * c0, self.c0.c1 * c0, self.c0.c2 * c0);
        let b = self.c1.mul_by_01(c3, c4);
        let e = (self.c0 + self.c1).mul_by_01(c0 + c3, c4);
        Self::new(b.mul_by_nonresidue() + a, e - (a + b))
    }

    /// Squaring in the cyclotomic subgroup (Granger and Scott, "Faster squaring
    /// in the cyclotomic subgroup of sixth degree extensions", 2010).
    pub(super) fn cyclotomic_square(self) -> Self {
        let r0 = self.c0.c0;
        let r4 = self.c0.c1;
        let r3 = self.c0.c2;
        let r2 = self.c1.c0;
        let r1 = self.c1.c1;
        let r5 = self.c1.c2;

        let fp4_square = |a: Fe2<ADX>, b: Fe2<ADX>| {
            let tmp = a * b;
            let t0 = (a + b) * (b.mul_by_nonresidue() + a) - tmp - tmp.mul_by_nonresidue();
            (t0, tmp.double())
        };
        let (t0, t1) = fp4_square(r0, r1);
        let (t2, t3) = fp4_square(r2, r3);
        let (t4, t5) = fp4_square(r4, r5);

        // z0 = 3 t0 - 2 z0, z1 = 3 t1 + 2 z1
        let z0 = (t0 - r0).double() + t0;
        let z1 = (t1 + r1).double() + t1;
        // z2 = 3 (xi t5) + 2 z2, z3 = 3 t4 - 2 z3
        let xi_t5 = t5.mul_by_nonresidue();
        let z2 = (r2 + xi_t5).double() + xi_t5;
        let z3 = (t4 - r3).double() + t4;
        // z4 = 3 t2 - 2 z4, z5 = 3 t3 + 2 z5
        let z4 = (t2 - r4).double() + t2;
        let z5 = (r5 + t3).double() + t3;

        Self::new(Fe6::new(z0, z4, z3), Fe6::new(z2, z1, z5))
    }

    pub(super) fn frobenius_map(self, power: usize) -> Self {
        let c1 = self.c1.frobenius_map(power);
        let coeff = Self::FROBENIUS_C1[power % 12];
        Self::new(
            self.c0.frobenius_map(power),
            Fe6::new(c1.c0 * coeff, c1.c1 * coeff, c1.c2 * coeff),
        )
    }

    pub(super) fn inverse(self) -> Option<Self> {
        let norm = self.c0.square() - self.c1.square().mul_by_nonresidue();
        let inv = norm.inverse()?;
        Some(Self::new(self.c0 * inv, -(self.c1 * inv)))
    }
}

impl<const ADX: bool> Mul for Fe12<ADX> {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        // Karatsuba with w^2 = v.
        let v0 = self.c0 * rhs.c0;
        let v1 = self.c1 * rhs.c1;
        let c1 = (self.c0 + self.c1) * (rhs.c0 + rhs.c1) - v0 - v1;
        Self::new(v0 + v1.mul_by_nonresidue(), c1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::{Fq6, Fq12};
    use ark_ff::{AdditiveGroup, CyclotomicMultSubgroup, Field, One, PrimeField, Zero};
    use sha2::{Digest, Sha256};

    fn fq(seed: u64) -> Fq {
        Fq::from_be_bytes_mod_order(&Sha256::digest(seed.to_le_bytes()))
    }

    /// Random elements, with every eighth one replaced by an edge value.
    fn base(seed: u64) -> Fq {
        let edges = [
            Fq::zero(),
            Fq::one(),
            -Fq::one(),
            -Fq::from(2u64),
            Fq::from(2u64).pow([253u64]),
            Fq::from(u64::MAX),
            Fq::from(2u64).pow([128u64]) - Fq::one(),
            // The largest element and the ones whose limbs are all ones or zeros.
            Fq::new_unchecked(BigInt([MODULUS[0] - 1, MODULUS[1], MODULUS[2], MODULUS[3]])),
            Fq::new_unchecked(BigInt([u64::MAX, u64::MAX, u64::MAX, 0])),
            Fq::new_unchecked(BigInt([0, 0, 0, 1])),
        ];
        if seed % 8 == 7 {
            edges[(seed / 8) as usize % edges.len()]
        } else {
            fq(seed)
        }
    }

    fn fq2(seed: u64) -> Fq2 {
        Fq2::new(base(2 * seed), base(2 * seed + 1))
    }

    fn fq6(seed: u64) -> Fq6 {
        Fq6::new(fq2(3 * seed), fq2(3 * seed + 1), fq2(3 * seed + 2))
    }

    fn fq12(seed: u64) -> Fq12 {
        Fq12::new(fq6(2 * seed), fq6(2 * seed + 1))
    }

    fn tower_matches<const ADX: bool>() {
        let two_inv = Fq::from(2u64).inverse().unwrap();
        for seed in 0..20_000u64 {
            let (a, b) = (base(seed), base(seed + 100_000));
            let (x, y) = (Fe::<ADX>::from_ark(a), Fe::<ADX>::from_ark(b));
            assert_eq!((x + y).to_ark(), a + b, "add seed {seed}");
            assert_eq!((x - y).to_ark(), a - b, "sub seed {seed}");
            assert_eq!((y - x).to_ark(), b - a, "sub seed {seed}");
            assert_eq!((x + x).to_ark(), a.double(), "double seed {seed}");
            assert_eq!((-x).to_ark(), -a, "neg seed {seed}");
            assert_eq!((x * y).to_ark(), a * b, "mul seed {seed}");
            assert_eq!(x.square().to_ark(), a.square(), "square seed {seed}");
            assert_eq!(x.halve().to_ark(), a * two_inv, "halve seed {seed}");
            assert_eq!(x.inverse().map(Fe::to_ark), a.inverse());

            let (a2, b2) = (fq2(seed), fq2(seed + 100_000));
            let (x2, y2) = (Fe2::<ADX>::from_ark(a2), Fe2::<ADX>::from_ark(b2));
            assert_eq!((x2 * y2).to_ark(), a2 * b2, "fq2 mul seed {seed}");
            assert_eq!(
                (x2 * x2).to_ark(),
                a2.square(),
                "fq2 mul by itself seed {seed}"
            );
            assert_eq!(x2.square().to_ark(), a2.square(), "fq2 square seed {seed}");
            assert_eq!(
                x2.mul_by_nonresidue().to_ark(),
                <Fq6Config as ark_ff::Fp6Config>::mul_fp2_by_nonresidue(a2)
            );
            assert_eq!(x2.mul_by_fe(y).to_ark(), a2 * Fq2::new(b, Fq::zero()));
            assert_eq!(x2.inverse().map(Fe2::to_ark), a2.inverse());
            assert_eq!(x2.frobenius_map(1).to_ark(), a2.frobenius_map(1));

            if seed % 16 != 0 {
                continue;
            }
            let (a6, b6) = (fq6(seed), fq6(seed + 100_000));
            let (x6, y6) = (Fe6::<ADX>::from_ark(a6), Fe6::<ADX>::from_ark(b6));
            assert_eq!((x6 * y6).to_ark(), a6 * b6);
            assert_eq!(x6.square().to_ark(), a6.square());
            let mut expected = a6;
            expected.mul_by_01(&b2, &a2);
            assert_eq!(x6.mul_by_01(y2, x2).to_ark(), expected);
            let mut expected = a6;
            <Fq12Config as ark_ff::Fp12Config>::mul_fp6_by_nonresidue_in_place(&mut expected);
            assert_eq!(x6.mul_by_nonresidue().to_ark(), expected);
            assert_eq!(x6.inverse().map(Fe6::to_ark), a6.inverse());
            for power in 0..6 {
                assert_eq!(x6.frobenius_map(power).to_ark(), a6.frobenius_map(power));
            }

            let (a12, b12) = (fq12(seed), fq12(seed + 100_000));
            let (x12, y12) = (Fe12::<ADX>::from_ark(a12), Fe12::<ADX>::from_ark(b12));
            assert_eq!((x12 * y12).to_ark(), a12 * b12);
            assert_eq!(x12.square().to_ark(), a12.square());
            let (c0, c3, c4) = (fq2(seed + 1), fq2(seed + 2), fq2(seed + 3));
            let mut expected = a12;
            expected.mul_by_034(&c0, &c3, &c4);
            let line = (Fe2::from_ark(c0), Fe2::from_ark(c3), Fe2::from_ark(c4));
            assert_eq!(x12.mul_by_034(line.0, line.1, line.2).to_ark(), expected);
            assert_eq!(x12.inverse().map(Fe12::to_ark), a12.inverse());
            for power in 0..12 {
                assert_eq!(x12.frobenius_map(power).to_ark(), a12.frobenius_map(power));
            }

            // Map into the cyclotomic subgroup with the easy part of the final
            // exponentiation, then compare the cyclotomic operations.
            if let Some(inverse) = a12.inverse() {
                let mut conjugate = a12;
                conjugate.conjugate_in_place();
                let mut c = conjugate * inverse;
                let c_p6 = c;
                c.frobenius_map_in_place(2);
                c *= c_p6;
                let z = Fe12::<ADX>::from_ark(c);
                assert_eq!(z.cyclotomic_square().to_ark(), c.cyclotomic_square());
                let mut c_inverse = c;
                c_inverse.cyclotomic_inverse_in_place();
                assert_eq!(z.conjugate().to_ark(), c_inverse);
            }
        }
    }

    #[test]
    fn tower_matches_arkworks_portable() {
        tower_matches::<false>();
    }

    #[test]
    fn tower_matches_arkworks_adx() {
        if adx_supported() {
            tower_matches::<true>();
        }
    }
}
