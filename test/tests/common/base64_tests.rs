use ethrex_common::base64::{decode, encode};

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
    test_decoding!("aG9sYQ==".as_bytes(), "hola".as_bytes());
    test_decoding!("".as_bytes(), "".as_bytes());
    test_decoding!("YQ==".as_bytes(), "a".as_bytes());
    test_decoding!("YWJj".as_bytes(), "abc".as_bytes());
    test_decoding!("5L2g5aW9".as_bytes(), "你好".as_bytes());
    test_decoding!("IUAjJCU=".as_bytes(), "!@#$%".as_bytes());
    test_decoding!(
        "VGhpcyBpcyBhIG11Y2ggbG9uZ2VyIHRlc3Qgc3RyaW5nLg==".as_bytes(),
        "This is a much longer test string.".as_bytes()
    );
    test_decoding!("VGVTdA==".as_bytes(), "TeSt".as_bytes());
    test_decoding!("MTIzNDU=".as_bytes(), "12345".as_bytes());
}
