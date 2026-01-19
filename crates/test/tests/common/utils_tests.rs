use ethereum_types::U256;
use ethrex_common::utils::u256_to_big_endian;

#[test]
fn u256_to_big_endian_test() {
    let a = u256_to_big_endian(U256::one());
    let b = U256::one().to_big_endian();
    assert_eq!(a, b);

    let a = u256_to_big_endian(U256::max_value());
    let b = U256::max_value().to_big_endian();
    assert_eq!(a, b);
}
