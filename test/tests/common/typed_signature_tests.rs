use ethrex_common::U256;
use ethrex_common::types::{
    EIP1559Transaction, EIP2930Transaction, EIP4844Transaction, EIP7702Transaction, Transaction,
};
use ethrex_crypto::{CryptoError, NativeCrypto};

fn typed_transactions(y_parity: U256) -> [Transaction; 4] {
    [
        Transaction::EIP2930Transaction(EIP2930Transaction {
            signature_y_parity: y_parity,
            ..Default::default()
        }),
        Transaction::EIP1559Transaction(EIP1559Transaction {
            signature_y_parity: y_parity,
            ..Default::default()
        }),
        Transaction::EIP4844Transaction(EIP4844Transaction {
            signature_y_parity: y_parity,
            ..Default::default()
        }),
        Transaction::EIP7702Transaction(EIP7702Transaction {
            signature_y_parity: y_parity,
            ..Default::default()
        }),
    ]
}

#[test]
fn non_binary_typed_parity_decodes_then_fails_sender_recovery() {
    for y_parity in [
        U256::from(2u8),
        U256::from(27u8),
        U256::from(34u8),
        U256::MAX,
    ] {
        for transaction in typed_transactions(y_parity) {
            let encoded = transaction.encode_canonical_to_vec();
            let decoded = Transaction::decode_canonical(&encoded)
                .expect("an invalid signature must not make the transaction undecodable");
            assert_eq!(decoded.hash(&NativeCrypto), transaction.hash(&NativeCrypto));
            assert!(matches!(
                decoded.sender(&NativeCrypto),
                Err(CryptoError::InvalidSignature)
            ));
        }
    }
}
