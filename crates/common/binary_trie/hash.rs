/// BLAKE3 hash of arbitrary input, returns 32 bytes.
pub fn blake3_hash(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_hash_produces_32_bytes() {
        let result = blake3_hash(b"hello");
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn blake3_hash_is_deterministic() {
        let a = blake3_hash(b"test data");
        let b = blake3_hash(b"test data");
        assert_eq!(a, b);
    }

    #[test]
    fn blake3_hash_different_inputs_differ() {
        let a = blake3_hash(b"foo");
        let b = blake3_hash(b"bar");
        assert_ne!(a, b);
    }
}
