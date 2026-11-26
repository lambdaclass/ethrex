//! Checks every alternative passes before it is measured. This file is the
//! same on every branch under comparison; what differs is what `adapter.rs`
//! builds.
//!
//! - Every workload goes through the whole lifecycle.
//! - An externally owned account controlled by a secp256k1 key migrates to the
//!   alternative's authorization and then sends a transaction authorized by its
//!   new key.
//! - A signature that does not authorize a transaction is rejected, at pool
//!   admission and at block import, for each of five ways it can fail.

use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::{
    Address, H160, H256, U256,
    types::{AuthorizationTuple, EIP1559Transaction, EIP7702Transaction, Transaction, TxKind},
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use secp256k1::{Message, SECP256K1, SecretKey};

use super::adapter;
use super::lifecycle::{
    AccountKind, Chain, Fault, FaultySubmission, Harness, ImportPath, KeyPair, Layer,
    MAX_FEE_PER_GAS, MAX_PRIORITY_FEE_PER_GAS, Prepared, RECIPIENT, TxRequest, VERIFY_GAS_LIMIT,
    Workload, funded_account, key, pass, prepare, sender_keys, workload_call,
};

/// A harness whose nodes share one state-root thread pool.
macro_rules! harness {
    ($name:ident) => {
        let pool = Blockchain::build_merkle_pool();
        let new_blockchain =
            |store: Store| Blockchain::for_test_harness_with_pool(store, pool.clone());
        let $name = Harness::new(&new_blockchain).await;
    };
}

#[tokio::test]
async fn every_supported_workload_goes_from_submission_to_import() {
    harness!(harness);
    for workload in Workload::ALL {
        if let Err(reason) = workload.supported() {
            println!(
                "pqx-check | {} | {} | not supported: {reason}",
                adapter::ALTERNATIVE,
                workload.name()
            );
            continue;
        }
        let prepared = prepare(workload, 2).await;
        let outcome = pass(&harness, &prepared, true).await;
        let block = &outcome.built.block;

        // The block holds exactly the transactions that were submitted.
        let mut included: Vec<H256> = block
            .body
            .transactions
            .iter()
            .map(|tx| tx.hash(&NativeCrypto))
            .collect();
        let mut submitted: Vec<H256> = prepared
            .submissions
            .iter()
            .map(|submission| submission.transaction.hash(&NativeCrypto))
            .collect();
        included.sort_unstable();
        submitted.sort_unstable();
        assert_eq!(included, submitted, "{}", workload.name());

        // What the adapter reports is what the block shows.
        if let Some(kind) = workload.account_kind() {
            let extras = adapter::block_extras(block);
            let beside: usize = prepared
                .submissions
                .iter()
                .map(|submission| submission.witness_bytes)
                .sum();
            for submission in &prepared.submissions {
                assert_eq!(submission.gas.signatures, kind.keys() as u64);
                assert!(
                    submission.gas.signature_bytes_in_transaction
                        + submission.gas.signature_bytes_beside_transaction
                        > 0,
                    "a signed transaction carries its signature somewhere"
                );
            }
            if adapter::MILESTONE == 1 {
                assert_eq!(
                    extras.witness_bytes, beside,
                    "without aggregation the block carries every witness submitted"
                );
                assert_eq!(extras.aggregate_proof_bytes, 0);
            }
        }
        println!(
            "pqx-check | {} | {} | 2 transactions admitted, built and imported on both paths, \
             block gas {}",
            adapter::ALTERNATIVE,
            workload.name(),
            block.header.gas_used
        );
    }
}

// ---------------------------------------------------------------------------
// Migration of an existing externally owned account
// ---------------------------------------------------------------------------

/// Holds the validation code the account adopts.
const TEMPLATE: Address = H160([0x90; 20]);
/// The account delegates to this contract for the one transaction that
/// migrates it.
const MIGRATOR: Address = H160([0x91; 20]);
const EIP_7702_MAGIC: u8 = 0x05;
const MIGRATION_GAS_LIMIT: u64 = 1_000_000;

/// Code the account runs once, through an EIP-7702 delegation, to migrate: it
/// stores the new key in slot 0 and replaces the account's code with the
/// template's (EIP-8298 `SETCODEFROM`). Only the account itself may run it.
fn migrator_code() -> Vec<u8> {
    let mut code = vec![
        0x33, 0x30, 0x14, // CALLER, ADDRESS, EQ
        0x60, 0x09, 0x57, // PUSH1 9, JUMPI
        0x5f, 0x5f, 0xfd, // PUSH0, PUSH0, REVERT
        0x5b, // JUMPDEST (9)
        0x5f, 0x35, 0x5f, 0x55, // SSTORE(0, CALLDATALOAD(0))
        0x73, // PUSH20 template
    ];
    code.extend_from_slice(TEMPLATE.as_bytes());
    code.extend_from_slice(&[
        0xa7, // SETCODEFROM
        0x60, 0x2a, 0x57, // PUSH1 42, JUMPI
        0x5f, 0x5f, 0xfd, // PUSH0, PUSH0, REVERT
        0x5b, 0x00, // JUMPDEST (42), STOP
    ]);
    assert_eq!(code[42], 0x5b);
    code
}

/// An EIP-7702 authorization of `delegate` signed by `secret`.
fn authorization(
    chain_id: u64,
    delegate: Address,
    nonce: u64,
    secret: &SecretKey,
) -> AuthorizationTuple {
    let mut payload = vec![EIP_7702_MAGIC];
    (U256::from(chain_id), delegate, nonce).encode(&mut payload);
    let digest = keccak(&payload);
    let (recovery_id, signature) = SECP256K1
        .sign_ecdsa_recoverable(&Message::from_digest(digest.0), secret)
        .serialize_compact();
    AuthorizationTuple {
        chain_id: U256::from(chain_id),
        address: delegate,
        nonce,
        y_parity: U256::from(Into::<i32>::into(recovery_id) as u64),
        r_signature: U256::from_big_endian(&signature[..32]),
        s_signature: U256::from_big_endian(&signature[32..64]),
    }
}

#[tokio::test]
async fn externally_owned_account_migrates_and_sends_an_authorized_transaction() {
    if let Err(reason) = adapter::supports(AccountKind::KeyInStorage) {
        println!(
            "pqx-migration | {} | not applicable: {reason}",
            adapter::ALTERNATIVE
        );
        return;
    }
    harness!(harness);

    let secret = SecretKey::from_slice(&[0x42; 32]).expect("a valid secp256k1 key");
    let signer = LocalSigner::new(secret);
    let account = signer.address;
    let signer: Signer = signer.into();
    let new_key = key(900_000);
    let other_key = key(900_001);
    let template_code = adapter::account_code(AccountKind::KeyInStorage, &[]);

    let mut template = funded_account(template_code.clone());
    template.balance = U256::zero();
    template.nonce = 1;
    let mut migrator = funded_account(migrator_code());
    migrator.balance = U256::zero();
    migrator.nonce = 1;
    let chain = Chain::new(vec![
        (account, funded_account(Vec::new())),
        (TEMPLATE, template),
        (MIGRATOR, migrator),
    ]);

    // Block 1: the account, still controlled by its secp256k1 key, delegates
    // to the migrator and calls itself with its new key.
    let builder = harness.node(&chain).await;
    let mut migration = Transaction::EIP7702Transaction(EIP7702Transaction {
        chain_id: chain.chain_id,
        nonce: 0,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        gas_limit: MIGRATION_GAS_LIMIT,
        to: account,
        value: U256::zero(),
        data: Bytes::from(new_key.public.clone()),
        access_list: Vec::new(),
        // The sender's nonce is bumped before its own authorization is read.
        authorization_list: vec![authorization(chain.chain_id, MIGRATOR, 1, &secret)],
        ..Default::default()
    });
    migration
        .sign_inplace(&signer)
        .await
        .expect("sign the migration");
    builder
        .submit(&migration.encode_canonical_to_vec())
        .await
        .0
        .expect("the migration enters the pool");
    let (first, _) = builder.build();
    assert_eq!(first.block.body.transactions.len(), 1);
    assert!(
        first.receipts[0].succeeded,
        "the migration must succeed: {:?}",
        first.receipts
    );
    builder
        .import(&first, ImportPath::AddBlock)
        .0
        .expect("the builder imports its block");
    builder.set_head(&first.block).await;
    builder
        .blockchain
        .remove_transaction_from_pool(&migration.hash(&NativeCrypto))
        .expect("drop the included transaction");

    // The account now has the template's code and its new key in slot 0.
    let info = builder
        .store
        .get_account_info(1, account)
        .await
        .expect("read the account")
        .expect("the account exists");
    assert_eq!(info.code_hash, keccak(&template_code));
    assert_eq!(info.nonce, 2);
    let stored = builder
        .store
        .get_storage_at(1, account, H256::zero())
        .expect("read slot 0")
        .expect("slot 0 is set");
    assert_eq!(stored, U256::from_big_endian(&new_key.public));

    // Its secp256k1 key no longer authorizes anything.
    let mut legacy = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: chain.chain_id,
        nonce: 2,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        gas_limit: 100_000,
        to: TxKind::Call(RECIPIENT),
        value: U256::one(),
        ..Default::default()
    });
    legacy.sign_inplace(&signer).await.expect("sign");
    let refused = builder
        .submit(&legacy.encode_canonical_to_vec())
        .await
        .0
        .expect_err("a transaction signed with the old key is refused");

    // Nor does another key of the new scheme.
    let request = |keys: &'static [KeyPair]| TxRequest {
        chain_id: chain.chain_id,
        sender: account,
        nonce: 2,
        account: AccountKind::KeyInStorage,
        keys,
        call: workload_call(Workload::Transfer, chain.gas_limit),
        verify_gas_limit: VERIFY_GAS_LIMIT,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        max_fee_per_gas: MAX_FEE_PER_GAS,
    };
    let other_keys: &'static [KeyPair] = Box::leak(vec![other_key].into_boxed_slice());
    let new_keys: &'static [KeyPair] = Box::leak(vec![new_key].into_boxed_slice());
    let forged = adapter::build(&request(other_keys));
    builder
        .submit(&forged.rpc)
        .await
        .0
        .expect_err("a transaction authorized by another key is refused");

    // Block 2: a transfer authorized by the new key.
    let submission = adapter::build(&request(new_keys));
    builder
        .submit(&submission.rpc)
        .await
        .0
        .expect("the transaction authorized by the new key enters the pool");
    let (second, _) = builder.build();
    let included: Vec<H256> = second
        .block
        .body
        .transactions
        .iter()
        .map(|tx| tx.hash(&NativeCrypto))
        .collect();
    assert_eq!(included, vec![submission.transaction.hash(&NativeCrypto)]);
    assert!(
        second.receipts[0].succeeded,
        "the authorized transaction must succeed: {:?}",
        second.receipts
    );

    // Both blocks import on a node that has seen neither, on each path.
    for path in ImportPath::ALL {
        let node = harness.node(&chain).await;
        node.import(&first, path)
            .0
            .unwrap_or_else(|error| panic!("{}: block 1: {error}", path.name()));
        node.set_head(&first.block).await;
        node.import(&second, path)
            .0
            .unwrap_or_else(|error| panic!("{}: block 2: {error}", path.name()));
    }
    println!(
        "pqx-migration | {} | EIP-7702 delegation to a migrator that stores the key and adopts \
         the validation code with SETCODEFROM | migration gas {} | authorized transaction gas {} \
         | old key refused with: {refused}",
        adapter::ALTERNATIVE,
        first.block.header.gas_used,
        second.block.header.gas_used,
    );
}

// ---------------------------------------------------------------------------
// Signatures that do not authorize
// ---------------------------------------------------------------------------

/// One account controlled by one key, its transfer, and a key that does not
/// control it.
struct Subject {
    chain: Chain,
    sender: Address,
    keys: Vec<KeyPair>,
    other: KeyPair,
}

impl Subject {
    fn new() -> Subject {
        let keys = sender_keys(AccountKind::SingleKey, 0);
        let sender = adapter::account_address(0, &keys);
        let code = adapter::account_code(AccountKind::SingleKey, &keys);
        Subject {
            chain: Chain::new(vec![(sender, funded_account(code))]),
            sender,
            keys,
            other: key(1),
        }
    }

    fn request(&self) -> TxRequest<'_> {
        TxRequest {
            chain_id: self.chain.chain_id,
            sender: self.sender,
            nonce: 0,
            account: AccountKind::SingleKey,
            keys: &self.keys,
            call: workload_call(Workload::Transfer, self.chain.gas_limit),
            verify_gas_limit: VERIFY_GAS_LIMIT,
            max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
            max_fee_per_gas: MAX_FEE_PER_GAS,
        }
    }
}

fn report(fault: Fault, layer: Layer, variant: &str, outcome: &str) {
    println!(
        "pqx-negative | {} | {} | {layer:?} | {variant} | {outcome}",
        adapter::ALTERNATIVE,
        fault.name()
    );
}

/// Whether the adapter has cases for `fault` at `layer`; when it has none it
/// must say why, and that reason is what the test reports.
fn expressible(fault: Fault, layer: Layer, cases: usize) -> bool {
    if cases > 0 {
        return true;
    }
    let reason = adapter::fault_not_expressible(fault, layer)
        .expect("an adapter with no case for a fault states why it cannot be expressed");
    report(fault, layer, "-", &format!("not expressible: {reason}"));
    false
}

/// Every way the adapter knows of producing `fault` is refused at pool
/// admission, nothing of it reaches the pool, and the valid transaction is
/// admitted afterwards by the same node.
async fn pool_must_reject(fault: Fault) {
    if adapter::supports(AccountKind::SingleKey).is_err() {
        return;
    }
    harness!(harness);
    let subject = Subject::new();
    let request = subject.request();
    let valid = adapter::build(&request);
    let cases = adapter::faulty_submissions(fault, &request, &subject.other);
    if !expressible(fault, Layer::Pool, cases.len()) {
        return;
    }
    for case in cases {
        let node = harness.node(&subject.chain).await;
        let result = match &case.input {
            FaultySubmission::Rpc(raw) => node.submit(raw).await.0.map(|_| ()),
            FaultySubmission::Peer(bytes) => node.receive(bytes).await.0.map(|_| ()),
        };
        let error = match result {
            Ok(()) => panic!("{}: {} was admitted", fault.name(), case.variant),
            Err(error) => error,
        };
        assert!(
            error.contains(case.expected_error),
            "{}: {} was refused for another reason: {error}",
            fault.name(),
            case.variant
        );
        let (built, _) = node.build();
        assert!(
            built.block.body.transactions.is_empty(),
            "{}: nothing may reach the pool",
            case.variant
        );
        node.submit(&valid.rpc)
            .await
            .0
            .expect("the valid transaction is admitted after the refusal");
        report(
            fault,
            Layer::Pool,
            case.variant,
            &format!("refused: {error}"),
        );
    }
}

/// Every way the adapter knows of producing `fault` in a block is refused on
/// both import paths, and the valid block imports afterwards on the same node.
async fn import_must_reject(fault: Fault) {
    if adapter::supports(AccountKind::SingleKey).is_err() {
        return;
    }
    harness!(harness);
    let subject = Subject::new();
    let request = subject.request();
    let prepared = Prepared {
        workload: Workload::Transfer,
        chain: subject.chain.clone(),
        submissions: vec![adapter::build(&request)],
    };
    // The valid block: built from the pool and imported on both paths.
    let valid = pass(&harness, &prepared, true).await.built;
    let cases = adapter::faulty_blocks(fault, &valid.block, &request, &subject.other);
    if !expressible(fault, Layer::Block, cases.len()) {
        return;
    }
    for case in cases {
        for path in case.input.paths.iter().copied() {
            let node = harness.node(&subject.chain).await;
            let (result, _) =
                node.import_block(case.input.block.clone(), valid.access_list.clone(), path);
            let error = match result {
                Ok(()) => panic!(
                    "{}: {} was imported by {}",
                    fault.name(),
                    case.variant,
                    path.name()
                ),
                Err(error) => error,
            };
            assert!(
                error.contains(case.expected_error),
                "{}: {} was refused by {} for another reason: {error}",
                fault.name(),
                case.variant,
                path.name()
            );
            node.import(&valid, path)
                .0
                .expect("the valid block imports after the refusal");
            report(
                fault,
                Layer::Block,
                case.variant,
                &format!("refused by {}: {error}", path.name()),
            );
        }
    }
}

#[tokio::test]
async fn pool_rejects_an_invalid_signature() {
    pool_must_reject(Fault::InvalidSignature).await;
}

#[tokio::test]
async fn pool_rejects_a_signature_of_another_key() {
    pool_must_reject(Fault::WrongKey).await;
}

#[tokio::test]
async fn pool_rejects_a_malformed_signature() {
    pool_must_reject(Fault::MalformedSignature).await;
}

#[tokio::test]
async fn pool_rejects_a_missing_signature() {
    pool_must_reject(Fault::MissingSignature).await;
}

#[tokio::test]
async fn pool_rejects_a_declaration_that_does_not_match_what_is_supplied() {
    pool_must_reject(Fault::DeclaredSuppliedMismatch).await;
}

#[tokio::test]
async fn import_rejects_a_block_with_an_invalid_signature() {
    import_must_reject(Fault::InvalidSignature).await;
}

#[tokio::test]
async fn import_rejects_a_block_with_a_signature_of_another_key() {
    import_must_reject(Fault::WrongKey).await;
}

#[tokio::test]
async fn import_rejects_a_block_with_a_malformed_signature() {
    import_must_reject(Fault::MalformedSignature).await;
}

#[tokio::test]
async fn import_rejects_a_block_with_a_missing_signature() {
    import_must_reject(Fault::MissingSignature).await;
}

#[tokio::test]
async fn import_rejects_a_block_whose_declarations_do_not_match_what_it_carries() {
    import_must_reject(Fault::DeclaredSuppliedMismatch).await;
}
