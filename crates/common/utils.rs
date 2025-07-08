use ethereum_types::{H160, U256, U512};
use hex::FromHexError;

/// Converts a big endian slice to a u256, faster than `u256::from_big_endian`.
pub fn u256_from_big_endian(slice: &[u8]) -> U256 {
    let mut padded = [0u8; 32];
    padded[32 - slice.len()..32].copy_from_slice(slice);

    let mut ret = [0; 4];

    let mut u64_bytes = [0u8; 8];
    for i in 0..4 {
        u64_bytes.copy_from_slice(&padded[8 * i..(8 * i + 8)]);
        ret[4 - i - 1] = u64::from_be_bytes(u64_bytes);
    }

    U256(ret)
}

/// Converts a constant big endian slice to a u256, faster than `u256::from_big_endian` and `u256_from_big_endian`.
///
/// Note: N should not exceed 32.
pub fn u256_from_big_endian_const<const N: usize>(slice: [u8; N]) -> U256 {
    const { assert!(N <= 32, "N must be less or equal to 32") };

    let mut padded = [0u8; 32];
    padded[32 - N..32].copy_from_slice(&slice);

    let mut ret = [0u64; 4];

    let mut u64_bytes = [0u8; 8];
    for i in 0..4 {
        u64_bytes.copy_from_slice(&padded[8 * i..(8 * i + 8)]);
        ret[4 - i - 1] = u64::from_be_bytes(u64_bytes);
    }

    U256(ret)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("overflow error")]
    OverFlowError,
}

#[inline]
pub fn u256_from_u512(value: U512) -> Result<ethnum::U256, Error> {
    let value: ethereum_types::U256 = value.try_into().map_err(|_| Error::OverFlowError)?;

    Ok(ethnum::U256::from_le_bytes(value.to_little_endian()))
}

#[inline]
pub fn u512_from_u256(value: ethnum::U256) -> U512 {
    U512::from_little_endian(&value.to_le_bytes())
}

#[inline]
pub fn u256_from_h160(value: H160) -> ethnum::U256 {
    let value = value.to_fixed_bytes();
    let mut buffer = [0u8; 32];
    buffer[12..].copy_from_slice(&value);
    ethnum::U256::from_be_bytes(buffer)
}

#[inline]
pub fn h160_from_u256(value: ethnum::U256) -> H160 {
    H160::from_slice(&value.to_be_bytes()[12..])
}

pub fn decode_hex(hex: &str) -> Result<Vec<u8>, FromHexError> {
    let trimmed = hex.strip_prefix("0x").unwrap_or(hex);
    hex::decode(trimmed)
}

pub fn u256_overflowing_pow(mut base: ethnum::U256, mut exp: ethnum::U256) -> (ethnum::U256, bool) {
    use ethnum::U256;
    let mut acc = U256::ONE;
    let mut overflown = false;
    // Scratch space for storing results of overflowing_mul.
    let mut r;

    while exp > 1 {
        if (exp & 1) == 1 {
            r = acc.overflowing_mul(base);
            acc = r.0;
            overflown |= r.1;
        }
        exp /= 2;
        r = base.overflowing_mul(base);
        base = r.0;
        overflown |= r.1;
    }

    // Deal with the final bit of the exponent separately, since
    // squaring the base afterwards is not necessary and may cause a
    // needless overflow.
    if exp == 1 {
        r = acc.overflowing_mul(base);
        acc = r.0;
        overflown |= r.1;
    }

    (acc, overflown)
}

#[cfg(test)]
mod tests {
    use ethereum_types::H160;

    use crate::utils::{u256_from_h160, u256_from_u512, u512_from_u256};

    #[test]
    fn test_u256_from_u512() {
        assert_eq!(
            u256_from_u512(u512_from_u256(ethnum::U256::MAX)).unwrap(),
            ethnum::U256::MAX
        );
    }

    #[test]
    fn test_u256_from_h160() {
        let address = H160::repeat_byte(64);
        let value = u256_from_h160(address);
        let address2 = H160::from_slice(&value.to_be_bytes()[12..]);
        assert_eq!(address, address2);
    }
}
