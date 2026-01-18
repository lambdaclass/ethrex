//! base64 decoder/encoder using safe alphabet according to:
//! https://datatracker.ietf.org/doc/html/rfc4648#section-4
//! https://datatracker.ietf.org/doc/html/rfc4648#section-5
//!
//! Encoding is implementing with padding at the end (add 1 or 2 '=' if necessary to make the data a multiple of 4)
//! Decoding does not require the data to be padded, that is it makes no difference if padding is present or not

pub(crate) fn byte_to_alphabet(byte: u8) -> char {
    match byte {
        0..=25 => (b'A' + byte) as char,         // A-Z
        26..=51 => (b'a' + (byte - 26)) as char, // a-z
        52..=61 => (b'0' + (byte - 52)) as char, // 0-9
        62 => '-',
        63 => '_',
        _ => '\0',
    }
}

pub(crate) fn alphabet_to_byte(byte: u8) -> u8 {
    match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'-' => 62,
        b'_' => 63,
        b'=' => 64,
        _ => 0,
    }
}

pub fn encode(bytes: &[u8]) -> Vec<u8> {
    let mut result: Vec<u8> = vec![];

    let mut bytes_iter = bytes.iter();
    while bytes_iter.len() > 0 {
        // each block is made up of as many as 24 bits (3 bytes)
        let mut block: Vec<u8> = vec![];

        while block.len() < 3 {
            if let Some(next) = bytes_iter.next() {
                block.push(*next);
            } else {
                break;
            }
        }

        let missing_bytes = 3 - block.len();

        // divide each block in a group of 4 concatenated 6 bits
        // and push its alphabet representation
        let mut carry = 0;
        let mut carry_bits: i32 = 0;
        for byte in block {
            let mut chunk = 0;
            let bits_left = 6 - carry_bits;
            if bits_left > 0 {
                chunk = byte >> (8 - bits_left);
            }
            // concatenate bits
            chunk |= carry << bits_left;
            carry_bits = 8 - bits_left;
            carry = byte & ((1 << carry_bits) - 1);
            result.push(byte_to_alphabet(chunk) as u8);
        }
        let chunk = carry << (6 - carry_bits);
        result.push(byte_to_alphabet(chunk) as u8);

        if missing_bytes == 1 {
            result.push(b'=');
        }
        if missing_bytes == 2 {
            result.push(b'=');
            result.push(b'=');
        }
    }

    result
}

pub fn decode(bytes: &[u8]) -> Vec<u8> {
    let mut result = vec![];
    let mut carry_bits: u8 = 0;

    for (i, byte) in bytes.iter().enumerate() {
        let val = alphabet_to_byte(*byte);
        if val == 64 {
            break;
        }

        // this byte has been consumed, continue with the next one
        if carry_bits == 6 {
            carry_bits = 0;
            continue;
        }

        let bit_1 = alphabet_to_byte(*byte) & ((1 << (6 - carry_bits)) - 1);
        carry_bits = 8 - (6 - carry_bits);

        // Check if there's another byte left
        if i + 1 >= bytes.len() {
            break;
        }
        let next_val = alphabet_to_byte(bytes[i + 1]);
        if next_val == 64 {
            break;
        }
        let bit_2 = next_val >> (6 - carry_bits);

        let bits = (bit_1 << (carry_bits)) | bit_2;
        result.push(bits);
    }

    result
}
