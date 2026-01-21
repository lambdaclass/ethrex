//! RLP (Recursive Length Prefix) encoding for Ethereum.
//!
//! RLP is used to encode data structures for hashing in Merkle tries.

/// RLP encoder for building RLP-encoded data.
#[derive(Clone, Debug)]
pub struct RlpEncoder {
    buffer: Vec<u8>,
}

impl RlpEncoder {
    /// Creates a new empty encoder.
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Creates an encoder with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity),
        }
    }

    /// Returns the encoded bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer
    }

    /// Consumes the encoder and returns the encoded bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }

    /// Clears the encoder.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Encodes a single byte.
    pub fn encode_byte(&mut self, byte: u8) {
        if byte == 0x00 {
            self.buffer.push(0x80);
        } else if byte < 0x80 {
            self.buffer.push(byte);
        } else {
            self.buffer.push(0x81);
            self.buffer.push(byte);
        }
    }

    /// Encodes a byte slice as a string.
    pub fn encode_bytes(&mut self, bytes: &[u8]) {
        if bytes.len() == 1 && bytes[0] < 0x80 {
            self.buffer.push(bytes[0]);
        } else if bytes.len() < 56 {
            self.buffer.push(0x80 + bytes.len() as u8);
            self.buffer.extend_from_slice(bytes);
        } else {
            let len_bytes = Self::encode_length(bytes.len());
            self.buffer.push(0xb7 + len_bytes.len() as u8);
            self.buffer.extend_from_slice(&len_bytes);
            self.buffer.extend_from_slice(bytes);
        }
    }

    /// Encodes raw bytes without any RLP prefix.
    ///
    /// Used for inline nodes in branch children - the data is already RLP-encoded
    /// and should be embedded directly without an additional string prefix.
    pub fn encode_raw(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// Encodes an empty string.
    pub fn encode_empty(&mut self) {
        self.buffer.push(0x80);
    }

    /// Starts encoding a list, returns the position to write length later.
    pub fn start_list(&mut self) -> usize {
        let pos = self.buffer.len();
        // Reserve space for list header (we'll fill it in later)
        self.buffer.push(0); // Placeholder
        pos
    }

    /// Finishes encoding a list started at the given position.
    pub fn finish_list(&mut self, start_pos: usize) {
        let content_len = self.buffer.len() - start_pos - 1;

        if content_len < 56 {
            self.buffer[start_pos] = 0xc0 + content_len as u8;
        } else {
            let len_bytes = Self::encode_length(content_len);
            let header_len = 1 + len_bytes.len();

            // Need to make room for longer header
            let extra = header_len - 1;
            let old_len = self.buffer.len();
            self.buffer.resize(old_len + extra, 0);
            self.buffer
                .copy_within(start_pos + 1..old_len, start_pos + header_len);

            self.buffer[start_pos] = 0xf7 + len_bytes.len() as u8;
            self.buffer[start_pos + 1..start_pos + header_len].copy_from_slice(&len_bytes);
        }
    }

    /// Encodes a list of items.
    pub fn encode_list<F>(&mut self, encode_items: F)
    where
        F: FnOnce(&mut Self),
    {
        let start = self.start_list();
        encode_items(self);
        self.finish_list(start);
    }

    /// Encodes the length as big-endian bytes without leading zeros.
    fn encode_length(len: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut n = len;

        if n == 0 {
            return vec![0];
        }

        while n > 0 {
            bytes.push((n & 0xff) as u8);
            n >>= 8;
        }

        bytes.reverse();
        bytes
    }

    /// Encodes a u64 value.
    pub fn encode_u64(&mut self, value: u64) {
        if value == 0 {
            self.buffer.push(0x80);
        } else if value < 0x80 {
            self.buffer.push(value as u8);
        } else {
            let bytes = Self::encode_length(value as usize);
            self.encode_bytes(&bytes);
        }
    }

    /// Encodes a fixed-size hash (32 bytes).
    pub fn encode_hash(&mut self, hash: &[u8; 32]) {
        // If hash is all zeros, encode as empty
        if hash.iter().all(|&b| b == 0) {
            self.encode_empty();
        } else {
            self.encode_bytes(hash);
        }
    }

    /// Encodes compact nibbles (for leaf/extension nodes).
    ///
    /// HP (Hex-Prefix) encoding:
    /// - First nibble: flags (0=extension even, 1=extension odd, 2=leaf even, 3=leaf odd)
    /// - Remaining nibbles: path
    pub fn encode_nibbles(&mut self, nibbles: &[u8], is_leaf: bool) {
        let odd = nibbles.len() % 2 == 1;
        let prefix = if is_leaf {
            if odd {
                0x3
            } else {
                0x2
            }
        } else {
            if odd {
                0x1
            } else {
                0x0
            }
        };

        let mut encoded = Vec::with_capacity((nibbles.len() + 2) / 2);

        if odd {
            // Odd: combine prefix with first nibble
            encoded.push((prefix << 4) | nibbles[0]);
            for chunk in nibbles[1..].chunks(2) {
                encoded.push((chunk[0] << 4) | chunk.get(1).copied().unwrap_or(0));
            }
        } else {
            // Even: prefix byte then nibbles
            encoded.push(prefix << 4);
            for chunk in nibbles.chunks(2) {
                encoded.push((chunk[0] << 4) | chunk.get(1).copied().unwrap_or(0));
            }
        }

        self.encode_bytes(&encoded);
    }
}

impl Default for RlpEncoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_empty() {
        let mut enc = RlpEncoder::new();
        enc.encode_empty();
        assert_eq!(enc.as_bytes(), &[0x80]);
    }

    #[test]
    fn test_encode_single_byte() {
        let mut enc = RlpEncoder::new();
        enc.encode_byte(0x7f);
        assert_eq!(enc.as_bytes(), &[0x7f]);

        enc.clear();
        enc.encode_byte(0x80);
        assert_eq!(enc.as_bytes(), &[0x81, 0x80]);
    }

    #[test]
    fn test_encode_short_string() {
        let mut enc = RlpEncoder::new();
        enc.encode_bytes(b"dog");
        assert_eq!(enc.as_bytes(), &[0x83, b'd', b'o', b'g']);
    }

    #[test]
    fn test_encode_short_list() {
        let mut enc = RlpEncoder::new();
        enc.encode_list(|e| {
            e.encode_bytes(b"cat");
            e.encode_bytes(b"dog");
        });
        assert_eq!(
            enc.as_bytes(),
            &[0xc8, 0x83, b'c', b'a', b't', 0x83, b'd', b'o', b'g']
        );
    }

    #[test]
    fn test_encode_u64() {
        let mut enc = RlpEncoder::new();
        enc.encode_u64(0);
        assert_eq!(enc.as_bytes(), &[0x80]);

        enc.clear();
        enc.encode_u64(127);
        assert_eq!(enc.as_bytes(), &[127]);

        enc.clear();
        enc.encode_u64(256);
        assert_eq!(enc.as_bytes(), &[0x82, 0x01, 0x00]);
    }

    #[test]
    fn test_encode_nibbles_leaf_odd() {
        let mut enc = RlpEncoder::new();
        enc.encode_nibbles(&[1, 2, 3], true);
        // Leaf + odd = 0x3, combined with first nibble: 0x31, then 0x23
        assert_eq!(enc.as_bytes(), &[0x82, 0x31, 0x23]);
    }

    #[test]
    fn test_encode_nibbles_extension_even() {
        let mut enc = RlpEncoder::new();
        enc.encode_nibbles(&[1, 2], false);
        // Extension + even = 0x0, then 0x00, 0x12
        assert_eq!(enc.as_bytes(), &[0x82, 0x00, 0x12]);
    }
}
