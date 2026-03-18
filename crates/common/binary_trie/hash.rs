pub trait TreeHasher {
    fn hash(data: &[u8]) -> [u8; 32];
}

pub struct Blake3Hasher;

impl TreeHasher for Blake3Hasher {
    fn hash(data: &[u8]) -> [u8; 32] {
        *blake3::hash(data).as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_hasher_produces_32_bytes() {
        let result = Blake3Hasher::hash(b"hello");
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn blake3_hasher_is_deterministic() {
        let a = Blake3Hasher::hash(b"test data");
        let b = Blake3Hasher::hash(b"test data");
        assert_eq!(a, b);
    }

    #[test]
    fn blake3_hasher_different_inputs_differ() {
        let a = Blake3Hasher::hash(b"foo");
        let b = Blake3Hasher::hash(b"bar");
        assert_ne!(a, b);
    }
}
