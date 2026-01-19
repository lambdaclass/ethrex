//! Fuzz target for transaction decoding.
//!
//! This fuzzer tests that transaction decoding never panics on arbitrary input.
//! Even malformed transaction data should return an error rather than panic.

#![no_main]

use libfuzzer_sys::fuzz_target;

use ethrex_common::types::{
    EIP1559Transaction, EIP2930Transaction, EIP4844Transaction, EIP7702Transaction,
    LegacyTransaction, Transaction,
};
use ethrex_rlp::decode::RLPDecode;

fuzz_target!(|data: &[u8]| {
    // Try to decode as the main Transaction enum
    // This should never panic, only return Ok or Err
    let _ = Transaction::decode(data);

    // Try to decode as specific transaction types
    let _ = LegacyTransaction::decode(data);
    let _ = EIP2930Transaction::decode(data);
    let _ = EIP1559Transaction::decode(data);
    let _ = EIP4844Transaction::decode(data);
    let _ = EIP7702Transaction::decode(data);

    // If data starts with a type byte, try the typed transaction format
    if let Some(&tx_type) = data.first() {
        if let Some(tx_data) = data.get(1..) {
            match tx_type {
                0x01 => {
                    let _ = EIP2930Transaction::decode(tx_data);
                }
                0x02 => {
                    let _ = EIP1559Transaction::decode(tx_data);
                }
                0x03 => {
                    let _ = EIP4844Transaction::decode(tx_data);
                }
                0x04 => {
                    let _ = EIP7702Transaction::decode(tx_data);
                }
                _ => {}
            }
        }
    }
});
