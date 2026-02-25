//! Integration test for native rollup deposit (L1→L2) and withdrawal (L2→L1) roundtrip.
//!
//! Requires the full 3-terminal setup already running:
//!   1. Start L1:      `NATIVE_ROLLUPS=1 make -C crates/l2 rm-db-l1 init-l1`
//!   2. Deploy:        `NATIVE_ROLLUPS=1 make -C crates/l2 deploy-l1`
//!   3. Start L2:      `NATIVE_ROLLUPS=1 make -C crates/l2 rm-db-l2 init-l2`
//!   4. Run this test: `cargo test -p ethrex-test --features native-rollups -- l2::native_rollup_bridge --nocapture`

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::arithmetic_side_effects
)]

use bytes::Bytes;
use ethrex_common::types::TxType;
use ethrex_common::{Address, H256, U256};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use ethrex_l2_sdk::{
    build_generic_tx, calldata::encode_calldata, send_generic_transaction,
    wait_for_transaction_receipt,
};
use ethrex_rpc::clients::eth::EthClient;
use ethrex_rpc::clients::Overrides;
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockTag};
use reqwest::Url;
use secp256k1::SecretKey;
use std::time::Duration;

use super::utils::read_env_file_by_config;

const L1_RPC_URL: &str = "http://localhost:8545";
const L2_RPC_URL: &str = "http://localhost:1729";

/// Private key from crates/l2/Makefile (pre-funded on L1 with 1M ETH in genesis).
const L1_PRIVATE_KEY: &str = "385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924";

/// L2Bridge predeploy at 0x...fffd.
const L2_BRIDGE_ADDRESS: &str = "0x000000000000000000000000000000000000fffd";

/// A fresh test account (not pre-funded on L2) — used to verify deposit lands.
const TEST_PRIVATE_KEY: &str = "0000000000000000000000000000000000000000000000000000000000000042";

const ONE_ETH: u64 = 1_000_000_000_000_000_000;

/// Deposit ETH from L1 to L2, then withdraw a portion back to L1 with proof.
#[tokio::test]
async fn native_rollup_bridge_roundtrip() {
    read_env_file_by_config();
    let contract_address: Address = std::env::var("ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS")
        .expect("ETHREX_NATIVE_ROLLUP_CONTRACT_ADDRESS not set — run `NATIVE_ROLLUPS=1 make -C crates/l2 deploy-l1` first")
        .parse()
        .expect("Invalid contract address");

    let l1_client = EthClient::new(Url::parse(L1_RPC_URL).unwrap()).unwrap();
    let l2_client = EthClient::new(Url::parse(L2_RPC_URL).unwrap()).unwrap();

    let l1_pk = SecretKey::from_slice(&hex::decode(L1_PRIVATE_KEY).unwrap()).unwrap();
    let l1_signer: Signer = LocalSigner::new(l1_pk).into();

    let test_pk = SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).unwrap();
    let test_signer: Signer = LocalSigner::new(test_pk).into();
    let test_address = test_signer.address();

    let l2_bridge: Address = L2_BRIDGE_ADDRESS.parse().unwrap();

    // ── Phase 1: Deposit 1 ETH from L1 to test account on L2 ───────────

    let deposit_amount = U256::from(ONE_ETH);

    let deposit_calldata = Bytes::from(
        encode_calldata(
            "sendL1Message(address,uint256,bytes)",
            &[
                Value::Address(test_address),
                Value::Uint(U256::from(105_000u64)),
                Value::Bytes(Bytes::new()),
            ],
        )
        .expect("encode sendL1Message failed"),
    );

    let deposit_tx = build_generic_tx(
        &l1_client,
        TxType::EIP1559,
        contract_address,
        l1_signer.address(),
        deposit_calldata,
        Overrides {
            value: Some(deposit_amount),
            gas_limit: Some(500_000),
            ..Default::default()
        },
    )
    .await
    .expect("Failed to build deposit tx");

    let deposit_tx_hash = send_generic_transaction(&l1_client, deposit_tx, &l1_signer)
        .await
        .expect("Failed to send deposit tx");

    let deposit_receipt = wait_for_transaction_receipt(deposit_tx_hash, &l1_client, 30)
        .await
        .expect("Deposit receipt not found");
    assert!(deposit_receipt.receipt.status, "Deposit tx reverted on L1");
    println!(
        "Deposit tx {deposit_tx_hash:#x} confirmed on L1 at block {}",
        deposit_receipt.block_info.block_number
    );

    // Poll L2 balance until deposit arrives (timeout ~120s)
    let initial_balance = l2_client
        .get_balance(test_address, BlockIdentifier::Tag(BlockTag::Latest))
        .await
        .unwrap();
    println!("Test account {test_address:#x} initial L2 balance: {initial_balance}");

    let mut deposited = false;
    for i in 0..60 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let balance = l2_client
            .get_balance(test_address, BlockIdentifier::Tag(BlockTag::Latest))
            .await
            .unwrap();
        if balance >= initial_balance + deposit_amount {
            println!(
                "Deposit arrived on L2 after ~{}s, balance: {balance}",
                (i + 1) * 2
            );
            deposited = true;
            break;
        }
        if i % 10 == 0 {
            println!("Waiting for deposit... ({i}/60), current balance: {balance}");
        }
    }
    assert!(deposited, "Deposit did not arrive on L2 within timeout");

    // ── Phase 2: Withdraw 0.5 ETH from L2 back to L1 ───────────────────

    let l1_receiver = l1_signer.address();
    let l1_receiver_balance_before = l1_client
        .get_balance(l1_receiver, BlockIdentifier::Tag(BlockTag::Latest))
        .await
        .unwrap();

    let withdraw_amount = U256::from(ONE_ETH / 2);
    let withdraw_calldata = Bytes::from(
        encode_calldata("withdraw(address)", &[Value::Address(l1_receiver)])
            .expect("encode withdraw failed"),
    );

    let withdraw_tx = build_generic_tx(
        &l2_client,
        TxType::EIP1559,
        l2_bridge,
        test_address,
        withdraw_calldata,
        Overrides {
            value: Some(withdraw_amount),
            gas_limit: Some(200_000),
            ..Default::default()
        },
    )
    .await
    .expect("Failed to build withdraw tx");

    let withdraw_tx_hash = send_generic_transaction(&l2_client, withdraw_tx, &test_signer)
        .await
        .expect("Failed to send withdraw tx");
    println!("Withdrawal tx sent on L2: {withdraw_tx_hash:#x}");

    let withdraw_receipt = wait_for_transaction_receipt(withdraw_tx_hash, &l2_client, 30)
        .await
        .expect("Withdrawal receipt not found on L2");
    assert!(
        withdraw_receipt.receipt.status,
        "Withdrawal tx reverted on L2"
    );
    let withdraw_l2_block = withdraw_receipt.block_info.block_number;
    println!("Withdrawal confirmed on L2 at block {withdraw_l2_block}");

    // ── Phase 3: Wait for L2 block to be committed to L1 ───────────────

    let block_number_calldata =
        Bytes::from(encode_calldata("blockNumber()", &[]).expect("encode blockNumber failed"));

    let mut committed = false;
    for i in 0..180 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let result = l1_client
            .call(
                contract_address,
                block_number_calldata.clone(),
                Overrides::default(),
            )
            .await
            .unwrap();
        let result_bytes = hex::decode(result.trim_start_matches("0x")).unwrap();
        let committed_block = U256::from_big_endian(&result_bytes);
        if committed_block >= U256::from(withdraw_l2_block) {
            println!(
                "L2 block {withdraw_l2_block} committed to L1 after ~{}s (L1 reports block {committed_block})",
                (i + 1) * 2
            );
            committed = true;
            break;
        }
        if i % 15 == 0 {
            println!(
                "Waiting for L2 block {withdraw_l2_block} to be committed... ({i}/180), L1 at block {committed_block}"
            );
        }
    }
    assert!(committed, "L2 block was not committed to L1 within timeout");

    // ── Phase 4: Get withdrawal proof from L2 RPC ───────────────────────

    let proof_response = reqwest::Client::new()
        .post(L2_RPC_URL)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "ethrex_getNativeWithdrawalProof",
            "params": [format!("{withdraw_tx_hash:#x}")],
            "id": 1
        }))
        .send()
        .await
        .expect("Failed to call ethrex_getNativeWithdrawalProof");

    let proof_json: serde_json::Value = proof_response.json().await.unwrap();
    println!("Withdrawal proof response: {proof_json}");

    let proof_result = &proof_json["result"];
    assert!(
        !proof_result.is_null(),
        "ethrex_getNativeWithdrawalProof returned null"
    );

    // Parse proof fields
    let proof_from: Address = proof_result["from"].as_str().unwrap().parse().unwrap();
    let proof_receiver: Address = proof_result["receiver"].as_str().unwrap().parse().unwrap();
    let proof_amount = U256::from_str_radix(
        proof_result["amount"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let proof_message_id = U256::from_str_radix(
        proof_result["messageId"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let proof_block_number = u64::from_str_radix(
        proof_result["blockNumber"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let account_proof: Vec<Bytes> = proof_result["accountProof"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Bytes::from(hex::decode(v.as_str().unwrap().trim_start_matches("0x")).unwrap()))
        .collect();
    let storage_proof: Vec<Bytes> = proof_result["storageProof"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Bytes::from(hex::decode(v.as_str().unwrap().trim_start_matches("0x")).unwrap()))
        .collect();

    println!(
        "Proof: from={proof_from:#x}, receiver={proof_receiver:#x}, amount={proof_amount}, \
         messageId={proof_message_id}, blockNumber={proof_block_number}"
    );

    // ── Phase 5: Claim withdrawal on L1 ─────────────────────────────────

    let claim_calldata = Bytes::from(
        encode_calldata(
            "claimWithdrawal(address,address,uint256,uint256,uint256,bytes[],bytes[])",
            &[
                Value::Address(proof_from),
                Value::Address(proof_receiver),
                Value::Uint(proof_amount),
                Value::Uint(proof_message_id),
                Value::Uint(U256::from(proof_block_number)),
                Value::Array(
                    account_proof
                        .iter()
                        .map(|b| Value::Bytes(b.clone()))
                        .collect(),
                ),
                Value::Array(
                    storage_proof
                        .iter()
                        .map(|b| Value::Bytes(b.clone()))
                        .collect(),
                ),
            ],
        )
        .expect("encode claimWithdrawal failed"),
    );

    let claim_tx = build_generic_tx(
        &l1_client,
        TxType::EIP1559,
        contract_address,
        l1_signer.address(),
        claim_calldata,
        Overrides {
            gas_limit: Some(1_000_000),
            ..Default::default()
        },
    )
    .await
    .expect("Failed to build claim tx");

    let claim_tx_hash = send_generic_transaction(&l1_client, claim_tx, &l1_signer)
        .await
        .expect("Failed to send claim tx");

    let claim_receipt = wait_for_transaction_receipt(claim_tx_hash, &l1_client, 30)
        .await
        .expect("Claim receipt not found");
    assert!(
        claim_receipt.receipt.status,
        "claimWithdrawal reverted on L1"
    );
    println!("Withdrawal claimed on L1! tx: {claim_tx_hash:#x}");

    // Verify L1 receiver got the ETH
    let l1_receiver_balance_after = l1_client
        .get_balance(l1_receiver, BlockIdentifier::Tag(BlockTag::Latest))
        .await
        .unwrap();
    println!(
        "L1 receiver balance: before={l1_receiver_balance_before}, after={l1_receiver_balance_after}"
    );

    // The balance should have increased by approximately withdraw_amount minus gas spent
    // on deposit + claim txs. We check a loose bound: balance_after > balance_before - max_gas_cost.
    let l1_gas_price = l1_client.get_gas_price().await.unwrap().as_u64();
    let max_gas_cost = U256::from(2_000_000u64) * U256::from(l1_gas_price) * 2;
    assert!(
        l1_receiver_balance_after + max_gas_cost + deposit_amount
            >= l1_receiver_balance_before + withdraw_amount,
        "L1 receiver did not receive withdrawal amount. Before: {l1_receiver_balance_before}, \
         After: {l1_receiver_balance_after}, Expected gain: ~{withdraw_amount}"
    );
}
