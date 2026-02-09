use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rlp::structs::{Decoder, Encoder};

#[derive(Debug, PartialEq, Eq)]
struct Simple {
    pub a: u8,
    pub b: u16,
}

#[test]
fn test_decoder_simple_struct() {
    let expected = Simple { a: 61, b: 75 };
    let mut buf = Vec::new();
    (expected.a, expected.b).encode(&mut buf);

    let decoder = Decoder::new(&buf).unwrap();
    let (a, decoder) = decoder.decode_field("a").unwrap();
    let (b, decoder) = decoder.decode_field("b").unwrap();
    let rest = decoder.finish().unwrap();

    assert!(rest.is_empty());
    let got = Simple { a, b };
    assert_eq!(got, expected);

    // Decoding the struct as a tuple should give the same result
    let tuple_decode = <(u8, u16) as RLPDecode>::decode(&buf).unwrap();
    assert_eq!(tuple_decode, (a, b));
}

#[test]
fn test_encoder_simple_struct() {
    let input = Simple { a: 61, b: 75 };
    let mut buf = Vec::new();

    Encoder::new(&mut buf)
        .encode_field(&input.a)
        .encode_field(&input.b)
        .finish();

    assert_eq!(buf, vec![0xc2, 61, 75]);

    // Encoding the struct from a tuple should give the same result
    let mut tuple_encoded = Vec::new();
    (input.a, input.b).encode(&mut tuple_encoded);
    assert_eq!(buf, tuple_encoded);
}
