// base64 decoder/encoder see: https://datatracker.ietf.org/doc/html/rfc4648#section-4

pub fn encode(bytes: &[u8]) -> Vec<u8> {
    let mut result: Vec<u8> = vec![];
    // safe alphabet thttps://datatracker.ietf.org/doc/html/rfc4648#section-5
    let alphabet = |bits: u8| -> char {
        match bits {
            0..=25 => (b'A' + bits) as char,         // A-Z
            26..=51 => (b'a' + (bits - 26)) as char, // a-z
            52..=61 => (b'0' + (bits - 52)) as char, // 0-9
            62 => '-',
            63 => '_',
            _ => '\0',
        }
    };

    let mut bytes_iter = bytes.iter();
    while bytes_iter.len() > 0 {
        // each block is made of as much as 24 bits (3 bytes)
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
            // parse into alphabet and push to result
            result.push(alphabet(chunk) as u8);
        }
        let chunk = carry << (6 - carry_bits);
        result.push(alphabet(chunk) as u8);

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

    let value = |byte| -> u8 {
        match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            b'=' => 64,
            _ => 0,
        }
    };

    for (i, byte) in bytes.iter().enumerate() {
        let val = value(*byte);
        if val == 64 {
            break;
        }

        // this byte has been consumed, continue with the next one
        if carry_bits == 6 {
            carry_bits = 0;
            continue;
        }

        let bit_1 = value(*byte) & ((1 << (6 - carry_bits)) - 1);
        carry_bits = 8 - (6 - carry_bits);
        let bit_2 = {
            let val = value(bytes[i + 1]);
            if val == 64 {
                break;
            } else {
                val >> (6 - carry_bits)
            }
        };

        let bits = (bit_1 << (carry_bits)) | bit_2;
        result.push(bits);
    }

    result
}

#[cfg(test)]
mod test {
    use super::{decode, encode};

    macro_rules! test_encoding {
        ($input:expr, $expected:expr) => {
            let res = encode($input);
            assert_eq!(res, $expected);
        };
    }

    macro_rules! test_decoding {
        ($input:expr, $expected:expr) => {
            let res = decode($input);
            assert_eq!(res, $expected);
        };
    }

    #[test]
    fn test_encoding() {
        test_encoding!("hola".as_bytes(), "aG9sYQ==".as_bytes());
        test_encoding!("".as_bytes(), "".as_bytes());
        test_encoding!("a".as_bytes(), "YQ==".as_bytes());
        test_encoding!("abc".as_bytes(), "YWJj".as_bytes());
        test_encoding!("你好".as_bytes(), "5L2g5aW9".as_bytes());
        test_encoding!("!@#$%".as_bytes(), "IUAjJCU=".as_bytes());
        test_encoding!(
            "This is a much longer test string.".as_bytes(),
            "VGhpcyBpcyBhIG11Y2ggbG9uZ2VyIHRlc3Qgc3RyaW5nLg==".as_bytes()
        );
        test_encoding!("TeSt".as_bytes(), "VGVTdA==".as_bytes());
        test_encoding!("12345".as_bytes(), "MTIzNDU=".as_bytes());
    }

    #[test]
    fn test_decoding() {
        // Test 1: Decoding "aG9sYQ==" -> "hola".as_bytes()
        test_decoding!("aG9sYQ==".as_bytes(), "hola".as_bytes());

        // Test 2: Decoding an empty string "" -> "".as_bytes()
        test_decoding!("".as_bytes(), "".as_bytes());

        // Test 3: Decoding "YQ==" -> "a".as_bytes()
        test_decoding!("YQ==".as_bytes(), "a".as_bytes());

        // Test 4: Decoding "YWJj" -> "abc".as_bytes()
        test_decoding!("YWJj".as_bytes(), "abc".as_bytes());

        // Test 5: Decoding "5L2g5aW9" -> "你好".as_bytes()
        test_decoding!("5L2g5aW9".as_bytes(), "你好".as_bytes());

        // Test 6: Decoding "IUAjJCU=" -> "!@#$%".as_bytes()
        test_decoding!("IUAjJCU=".as_bytes(), "!@#$%".as_bytes());

        // Test 7: Decoding a long string
        test_decoding!(
            "VGhpcyBpcyBhIG11Y2ggbG9uZ2VyIHRlc3Qgc3RyaW5nLg==".as_bytes(),
            "This is a much longer test string.".as_bytes()
        );
        test_decoding!("VGVTdA==".as_bytes(), "TeSt".as_bytes());
        test_decoding!("MTIzNDU=".as_bytes(), "12345".as_bytes());
    }
}
