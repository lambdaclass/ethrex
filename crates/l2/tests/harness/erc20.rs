use std::path::Path;

use crate::harness::{
    find_withdrawal_with_widget, get_contract_dependencies, l1_client, l2_client, rich_pk_1,
    test_balance_of, test_deploy, test_deploy_l1, test_send, wait_for_l2_deposit_receipt,
    wait_for_verified_proof,
};
use ethrex_common::{Address, U256};
use ethrex_l2::monitor::widget::l2_to_l1_messages::{
    L2ToL1MessageKind, L2ToL1MessageRow, L2ToL1MessageStatus,
};
use ethrex_l2_common::calldata::Value;
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use ethrex_l2_sdk::{
    COMMON_BRIDGE_L2_ADDRESS, bridge_address, claim_erc20withdraw, compile_contract, deposit_erc20,
    wait_for_transaction_receipt,
};

pub async fn test_erc20_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let l2_client = l2_client();
    let rich_wallet_private_key = rich_pk_1();
    let token_amount: U256 = U256::from(100);

    let rich_wallet_signer: Signer = LocalSigner::new(rich_wallet_private_key).into();
    let rich_address = rich_wallet_signer.address();

    let init_code_l1 = hex::decode(std::fs::read(
        "../../fixtures/contracts/ERC20/ERC20.bin/TestToken.bin",
    )?)?;

    println!("test_erc20_roundtrip: Deploying ERC20 token on L1");
    let token_l1 = test_deploy_l1(&l1_client, &init_code_l1, &rich_wallet_private_key).await?;

    let contracts_path = Path::new("contracts");

    get_contract_dependencies(contracts_path);
    let remappings = [(
        "@openzeppelin/contracts",
        contracts_path
            .join("lib/openzeppelin-contracts-upgradeable/lib/openzeppelin-contracts/contracts"),
    )];
    compile_contract(
        contracts_path,
        &contracts_path.join("src/example/L2ERC20.sol"),
        false,
        Some(&remappings),
    )?;
    let init_code_l2_inner = hex::decode(String::from_utf8(std::fs::read(
        "contracts/solc_out/TestTokenL2.bin",
    )?)?)?;
    let init_code_l2 = [
        init_code_l2_inner,
        vec![0u8; 12],
        token_l1.to_fixed_bytes().to_vec(),
    ]
    .concat();
    let token_l2 = test_deploy(&l2_client, &init_code_l2, &rich_wallet_private_key).await?;

    println!("test_erc20_roundtrip: token l1={token_l1:x}, l2={token_l2:x}");
    test_send(
        &l1_client,
        &rich_wallet_private_key,
        token_l1,
        "freeMint()",
        &[],
    )
    .await;
    test_send(
        &l1_client,
        &rich_wallet_private_key,
        token_l1,
        "approve(address,uint256)",
        &[Value::Address(bridge_address()?), Value::Uint(token_amount)],
    )
    .await;

    println!("test_erc20_roundtrip: Depositing ERC20 token from L1 to L2");
    let initial_balance = test_balance_of(&l1_client, token_l1, rich_address).await;
    let deposit_tx = deposit_erc20(
        token_l1,
        token_l2,
        token_amount,
        rich_address,
        &rich_wallet_signer,
        &l1_client,
    )
    .await
    .unwrap();

    println!("test_erc20_roundtrip: Waiting for deposit transaction receipt on L1");
    let res = wait_for_transaction_receipt(deposit_tx, &l1_client, 10)
        .await
        .unwrap();

    assert!(res.receipt.status);

    println!("test_erc20_roundtrip: Waiting for deposit transaction receipt on L2");
    wait_for_l2_deposit_receipt(res.block_info.block_number, &l1_client, &l2_client)
        .await
        .unwrap();
    let remaining_l1_balance = test_balance_of(&l1_client, token_l1, rich_address).await;
    let l2_balance = test_balance_of(&l2_client, token_l2, rich_address).await;
    assert_eq!(initial_balance - remaining_l1_balance, token_amount);
    assert_eq!(l2_balance, token_amount);

    println!("test_erc20_roundtrip: Withdrawing ERC20 token from L2 to L1");

    test_send(
        &l2_client,
        &rich_wallet_private_key,
        token_l2,
        "approve(address,uint256)",
        &[
            Value::Address(COMMON_BRIDGE_L2_ADDRESS),
            Value::Uint(token_amount),
        ],
    )
    .await;
    let res = test_send(
        &l2_client,
        &rich_wallet_private_key,
        COMMON_BRIDGE_L2_ADDRESS,
        "withdrawERC20(address,address,address,uint256)",
        &[
            Value::Address(token_l1),
            Value::Address(token_l2),
            Value::Address(rich_address),
            Value::Uint(token_amount),
        ],
    )
    .await;
    let withdrawal_tx_hash = res.tx_info.transaction_hash;
    assert_eq!(
        find_withdrawal_with_widget(
            bridge_address()?,
            withdrawal_tx_hash,
            &l2_client,
            &l1_client
        )
        .await
        .unwrap(),
        L2ToL1MessageRow {
            status: L2ToL1MessageStatus::WithdrawalInitiated,
            kind: L2ToL1MessageKind::ERC20Withdraw,
            receiver: rich_address,
            token_l1,
            token_l2,
            value: token_amount,
            l2_tx_hash: withdrawal_tx_hash
        }
    );

    let proof = wait_for_verified_proof(&l1_client, &l2_client, res.tx_info.transaction_hash).await;

    println!("test_erc20_roundtrip: Claiming withdrawal on L1");

    let withdraw_claim_tx = claim_erc20withdraw(
        token_l1,
        token_l2,
        token_amount,
        &rich_wallet_signer,
        &l1_client,
        &proof,
    )
    .await
    .expect("error while claiming");
    wait_for_transaction_receipt(withdraw_claim_tx, &l1_client, 5).await?;
    assert_eq!(
        find_withdrawal_with_widget(
            bridge_address()?,
            withdrawal_tx_hash,
            &l2_client,
            &l1_client
        )
        .await
        .unwrap(),
        L2ToL1MessageRow {
            status: L2ToL1MessageStatus::WithdrawalClaimed,
            kind: L2ToL1MessageKind::ERC20Withdraw,
            receiver: rich_address,
            token_l1,
            token_l2,
            value: token_amount,
            l2_tx_hash: withdrawal_tx_hash
        }
    );

    let l1_final_balance = test_balance_of(&l1_client, token_l1, rich_address).await;
    let l2_final_balance = test_balance_of(&l2_client, token_l2, rich_address).await;
    assert_eq!(initial_balance, l1_final_balance);
    assert!(l2_final_balance.is_zero());
    Ok(())
}

pub async fn test_erc20_failed_deposit() -> Result<(), Box<dyn std::error::Error>> {
    let l1_client = l1_client();
    let l2_client = l2_client();
    let rich_wallet_private_key = rich_pk_1();
    let token_amount: U256 = U256::from(100);

    let rich_wallet_signer: Signer = LocalSigner::new(rich_wallet_private_key).into();
    let rich_address = rich_wallet_signer.address();

    let init_code_l1 = hex::decode(std::fs::read(
        "../../fixtures/contracts/ERC20/ERC20.bin/TestToken.bin",
    )?)?;

    println!("test_erc20_failed_deposit: Deploying ERC20 token on L1");
    let token_l1 = test_deploy_l1(&l1_client, &init_code_l1, &rich_wallet_private_key).await?;
    let token_l2 = Address::random(); // will cause deposit to fail

    println!("test_erc20_failed_deposit: token l1={token_l1:x}, l2={token_l2:x}");

    test_send(
        &l1_client,
        &rich_wallet_private_key,
        token_l1,
        "freeMint()",
        &[],
    )
    .await;
    test_send(
        &l1_client,
        &rich_wallet_private_key,
        token_l1,
        "approve(address,uint256)",
        &[Value::Address(bridge_address()?), Value::Uint(token_amount)],
    )
    .await;

    println!("test_erc20_failed_deposit: Depositing ERC20 token from L1 to L2");

    let initial_balance = test_balance_of(&l1_client, token_l1, rich_address).await;
    let deposit_tx = deposit_erc20(
        token_l1,
        token_l2,
        token_amount,
        rich_address,
        &rich_wallet_signer,
        &l1_client,
    )
    .await
    .unwrap();

    println!("test_erc20_failed_deposit: Waiting for deposit transaction receipt on L1");

    let res = wait_for_transaction_receipt(deposit_tx, &l1_client, 10)
        .await
        .unwrap();

    assert!(res.receipt.status);

    println!("test_erc20_failed_deposit: Waiting for deposit transaction receipt on L2");

    let res = wait_for_l2_deposit_receipt(res.block_info.block_number, &l1_client, &l2_client)
        .await
        .unwrap();

    let proof = wait_for_verified_proof(&l1_client, &l2_client, res.tx_info.transaction_hash).await;

    println!("test_erc20_failed_deposit: Claiming withdrawal on L1");

    let withdraw_claim_tx = claim_erc20withdraw(
        token_l1,
        token_l2,
        token_amount,
        &rich_wallet_signer,
        &l1_client,
        &proof,
    )
    .await
    .expect("error while claiming");
    wait_for_transaction_receipt(withdraw_claim_tx, &l1_client, 5).await?;
    let l1_final_balance = test_balance_of(&l1_client, token_l1, rich_address).await;
    assert_eq!(initial_balance, l1_final_balance);
    Ok(())
}
