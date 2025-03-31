#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
use bytes::Bytes;
use ethrex_common::{Address, H256, U256};
use ethrex_rpc::{
    clients::{eth::BlockByNumber, Overrides},
    EthClient,
};

const ETH_RPC_URL: &str = "http://localhost:1729";
const RICH_ADDRESS: &str = "f39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const ERC20_ADDRESS: &str = "5015Ddf5fc3Dd6d61Ca1a7De348Cb08816237dC5";
const ERC20_RICH_ADDRESS1: &str = "f37C09c0e560b3026F25A495b71E5DC7eB8C8AF9";
const ERC20_RICH_ADDRESS2: &str = "f37C09c0e560b3026F25A495b71E5DC7eB8C8AF8";
const ERC20_SYMBOL_SIGNATURE: &str = "95d89b41";
const ERC20_SYMBOL: &str = "TOK";
const ERC20_NAME_SIGNATURE: &str = "06fdde03";
const ERC20_NAME: &str = "Token";
const ERC20_DECIMALS_SIGNATURE: &str = "313ce567";
const ERC20_DECIMALS: u8 = 18;
const ERC20_BALANCE_SIGNATURE: &str = "70a08231";

#[tokio::test]
async fn test_state_reconstruct_block_0() {
    let client = connect().await;

    let rich_address = Address::from_slice(&hex::decode(RICH_ADDRESS).unwrap());

    // Balance of RICH_ADDRESS should be 0 at block 0
    let balance = client
        .get_balance(rich_address, 0.into())
        .await
        .expect("Error getting balance");
    assert_eq!(balance, U256::zero());
}

#[tokio::test]
async fn test_state_reconstruct_block_1() {
    let client = connect().await;

    let rich_address = Address::from_slice(&hex::decode(RICH_ADDRESS).unwrap());
    let erc20_address = Address::from_slice(&hex::decode(ERC20_ADDRESS).unwrap());

    // Balance of RICH_ADDRESS should be
    // 4722366482869645213696 (0x1000000000000000000) at block 1
    let balance = client
        .get_balance(rich_address, 1.into())
        .await
        .expect("Error getting balance");
    assert_eq!(
        balance,
        U256::from_dec_str("4722366482869645213696").unwrap()
    );

    // Nonce of RICH_ADDRESS should be 0 at block 1
    let nonce = client
        .get_nonce(rich_address, 1.into())
        .await
        .expect("Error getting nonce");
    assert_eq!(nonce, 0);

    // Bytecode of ERC20_ADDRESS should be null at block 1
    let bytecode = client
        .get_code(erc20_address, 1.into())
        .await
        .expect("Error getting code");
    assert!(bytecode.is_empty());
}

#[tokio::test]
async fn test_state_reconstruct_block_2() {
    let client = connect().await;

    let rich_address = Address::from_slice(&hex::decode(RICH_ADDRESS).unwrap());
    let erc20_address = Address::from_slice(&hex::decode(ERC20_ADDRESS).unwrap());
    let erc20_rich_address1 = Address::from_slice(&hex::decode(ERC20_RICH_ADDRESS1).unwrap());
    let erc20_rich_address2 = Address::from_slice(&hex::decode(ERC20_RICH_ADDRESS2).unwrap());
    let erc20_symbol_signature = hex::decode(ERC20_SYMBOL_SIGNATURE).unwrap();
    let erc20_name_signature = hex::decode(ERC20_NAME_SIGNATURE).unwrap();
    let erc20_balance_signature = hex::decode(ERC20_BALANCE_SIGNATURE).unwrap();
    let erc20_decimals_signature = hex::decode(ERC20_DECIMALS_SIGNATURE).unwrap();

    // Nonce of RICH_ADDRESS should be 3 at block 2
    let nonce = client
        .get_nonce(rich_address, 2.into())
        .await
        .expect("Error getting nonce");
    assert_eq!(nonce, 3);

    // Bytecode of ERC20_ADDRESS should be the following ERC20 contract at block 2
    let bytecode = client
        .get_code(erc20_address, 2.into())
        .await
        .expect("Error getting code");
    assert_eq!(bytecode, erc20_bytecode());

    // Token symbol should be ERC20_SYMBOL at block 2
    let token_symbol = client
        .call(
            erc20_address,
            erc20_symbol_signature.into(),
            Overrides {
                from: Some(erc20_rich_address1),
                block: Some(2.into()),
                max_fee_per_gas: Some(10000000),
                ..Default::default()
            },
        )
        .await
        .expect("Error calling contract: symbol()(string)");

    // The result is a hexstring with a leading 0x, so we need to skip the first 2 characters.
    // The next 64 characters (32 bytes) are the offset to the start of the string in the return data.
    // The next 64 characters (32 bytes) are the length of the string in the return data.
    // The rest of the return data is the string itself, that should have length 6 (3 bytes).
    let token_symbol = String::from_utf8(
        hex::decode(token_symbol.get(130..136).expect("Invalid length"))
            .expect("Invalid response: not hex"),
    )
    .expect("Invalid response: not utf8");
    assert_eq!(ERC20_SYMBOL, token_symbol);

    // Token name should be "Token" at block 2
    let token_name = client
        .call(
            erc20_address,
            erc20_name_signature.into(),
            Overrides {
                from: Some(erc20_rich_address1),
                block: Some(2.into()),
                max_fee_per_gas: Some(10000000),
                ..Default::default()
            },
        )
        .await
        .expect("Error calling contract: name()(string)");
    // Same case as above, but with 5 bytes of data
    let token_name = String::from_utf8(
        hex::decode(token_name.get(130..140).expect("Invalid length"))
            .expect("Invalid response: not hex"),
    )
    .expect("Invalid response: not utf8");
    assert_eq!(ERC20_NAME, token_name);

    // Token decimals should be 18 at block 2
    let token_decimals = client
        .call(
            erc20_address,
            erc20_decimals_signature.into(),
            Overrides {
                from: Some(erc20_rich_address1),
                block: Some(2.into()),
                max_fee_per_gas: Some(10000000),
                ..Default::default()
            },
        )
        .await
        .expect("Error calling contract: decimals()(uint8)");
    // The result is a hexstring with a leading 0x, so we need to skip the first 2 characters.
    // The next 64 characters (32 bytes) are left padded uint8.
    let token_decimals = *hex::decode(token_decimals.get(64..).expect("Invalid length"))
        .expect("Invalid response: not hex")
        .first()
        .expect("Invalid length");
    assert_eq!(ERC20_DECIMALS, token_decimals);

    // Token balance of ERC20_RICH_ADDRESS1 should be 1e39 at block 2
    let token_balance = client
        .call(
            erc20_address,
            [
                erc20_balance_signature.as_slice(),
                H256::from(erc20_rich_address1).as_bytes(),
            ]
            .concat()
            .into(),
            Overrides {
                from: Some(erc20_rich_address1),
                block: Some(2.into()),
                max_fee_per_gas: Some(10000000),
                ..Default::default()
            },
        )
        .await
        .expect("Error calling contract: balanceOf(address)(uint256)");
    let token_balance = U256::from_big_endian(
        &hex::decode(token_balance.get(2..).expect("Invalid length"))
            .expect("Invalid response: not hex"),
    );
    assert_eq!(
        token_balance,
        U256::from_dec_str("1000000000000000000000000000000000000000").unwrap()
    );

    // Token balance of ERC20_RICH_ADDRESS2 should be 1.23e20 at block 2
    let token_balance = client
        .call(
            erc20_address,
            [
                erc20_balance_signature.as_slice(),
                H256::from(erc20_rich_address2).as_bytes(),
            ]
            .concat()
            .into(),
            Overrides {
                from: Some(erc20_rich_address1),
                block: Some(2.into()),
                max_fee_per_gas: Some(10000000),
                ..Default::default()
            },
        )
        .await
        .expect("Error calling contract: balanceOf(address)(uint256)");
    let token_balance = U256::from_big_endian(
        &hex::decode(token_balance.get(2..).expect("Invalid length"))
            .expect("Invalid response: not hex"),
    );
    assert_eq!(
        token_balance,
        U256::from_dec_str("123000000000000000000").unwrap()
    );
}

#[tokio::test]
async fn test_state_reconstruct_block_3() {
    let client = connect().await;

    let erc20_address = Address::from_slice(&hex::decode(ERC20_ADDRESS).unwrap());
    let erc20_rich_address1 = Address::from_slice(&hex::decode(ERC20_RICH_ADDRESS1).unwrap());
    let erc20_rich_address2 = Address::from_slice(&hex::decode(ERC20_RICH_ADDRESS2).unwrap());
    let erc20_balance_signature = hex::decode(ERC20_BALANCE_SIGNATURE).unwrap();

    // Token balance of ERC20_RICH_ADDRESS2 should be 1.22e20 at block 3
    let token_balance = client
        .call(
            erc20_address,
            [
                erc20_balance_signature.as_slice(),
                H256::from(erc20_rich_address2).as_bytes(),
            ]
            .concat()
            .into(),
            Overrides {
                from: Some(erc20_rich_address1),
                block: Some(3.into()),
                max_fee_per_gas: Some(10000000),
                ..Default::default()
            },
        )
        .await
        .expect("Error calling contract: balanceOf(address)(uint256)");
    let token_balance = U256::from_big_endian(
        &hex::decode(token_balance.get(2..).expect("Invalid length"))
            .expect("Invalid response: not hex"),
    );
    assert_eq!(
        token_balance,
        U256::from_dec_str("122000000000000000000").unwrap()
    );
}

#[tokio::test]
async fn test_state_reconstruct_latest_block() {
    let client = connect().await;

    let rich_address = Address::from_slice(&hex::decode(RICH_ADDRESS).unwrap());
    let erc20_address = Address::from_slice(&hex::decode(ERC20_ADDRESS).unwrap());
    let erc20_rich_address1 = Address::from_slice(&hex::decode(ERC20_RICH_ADDRESS1).unwrap());
    let erc20_rich_address2 = Address::from_slice(&hex::decode(ERC20_RICH_ADDRESS2).unwrap());
    let erc20_symbol_signature = hex::decode(ERC20_SYMBOL_SIGNATURE).unwrap();
    let erc20_name_signature = hex::decode(ERC20_NAME_SIGNATURE).unwrap();
    let erc20_decimals_signature = hex::decode(ERC20_DECIMALS_SIGNATURE).unwrap();
    let erc20_balance_signature = hex::decode(ERC20_BALANCE_SIGNATURE).unwrap();

    // Balance of RICH_ADDRESS should be
    // 4722366482869645213696 (0x1000000000000000000) at latest block
    let balance = client
        .get_balance(rich_address, BlockByNumber::Latest)
        .await
        .expect("Error getting balance");
    assert_eq!(
        balance,
        U256::from_dec_str("4722366482869645213696").unwrap()
    );

    // Nonce of RICH_ADDRESS should be 3 at latest block
    let nonce = client
        .get_nonce(rich_address, BlockByNumber::Latest)
        .await
        .expect("Error getting nonce");
    assert_eq!(nonce, 3);

    // Bytecode of ERC20_ADDRESS should be the following ERC20 contract at latest block
    let bytecode = client
        .get_code(erc20_address, BlockByNumber::Latest)
        .await
        .expect("Error getting code");
    assert_eq!(bytecode, erc20_bytecode());

    // Token symbol should be ERC20_SYMBOL at latest block
    let token_symbol = client
        .call(
            erc20_address,
            erc20_symbol_signature.into(),
            Overrides {
                from: Some(erc20_rich_address1),
                max_fee_per_gas: Some(10000000),
                ..Default::default()
            },
        )
        .await
        .expect("Error calling contract: symbol()(string)");

    // The result is a hexstring with a leading 0x, so we need to skip the first 2 characters.
    // The next 64 characters (32 bytes) are the offset to the start of the string in the return data.
    // The next 64 characters (32 bytes) are the length of the string in the return data.
    // The rest of the return data is the string itself, that should have length 6 (3 bytes).
    let token_symbol = String::from_utf8(
        hex::decode(token_symbol.get(130..136).expect("Invalid length"))
            .expect("Invalid response: not hex"),
    )
    .expect("Invalid response: not utf8");
    assert_eq!(ERC20_SYMBOL, token_symbol);

    // Token name should be "Token" at latest block
    let token_name = client
        .call(
            erc20_address,
            erc20_name_signature.into(),
            Overrides {
                from: Some(erc20_rich_address1),
                max_fee_per_gas: Some(10000000),
                ..Default::default()
            },
        )
        .await
        .expect("Error calling contract: name()(string)");
    // Same case as above, but with 5 bytes of data
    let token_name = String::from_utf8(
        hex::decode(token_name.get(130..140).expect("Invalid length"))
            .expect("Invalid response: not hex"),
    )
    .expect("Invalid response: not utf8");
    assert_eq!(ERC20_NAME, token_name);

    // Token decimals should be 18 at latest block
    let token_decimals = client
        .call(
            erc20_address,
            erc20_decimals_signature.into(),
            Overrides {
                from: Some(erc20_rich_address1),
                max_fee_per_gas: Some(10000000),
                ..Default::default()
            },
        )
        .await
        .expect("Error calling contract: decimals()(uint8)");
    // The result is a hexstring with a leading 0x, so we need to skip the first 2 characters.
    // The next 64 characters (32 bytes) are left padded uint8.
    let token_decimals = *hex::decode(token_decimals.get(64..).expect("Invalid length"))
        .expect("Invalid response: not hex")
        .first()
        .expect("Invalid length");
    assert_eq!(ERC20_DECIMALS, token_decimals);

    // Token balance of ERC20_RICH_ADDRESS1 should be 1e39 at latest block
    let token_balance = client
        .call(
            erc20_address,
            [
                erc20_balance_signature.as_slice(),
                H256::from(erc20_rich_address1).as_bytes(),
            ]
            .concat()
            .into(),
            Overrides {
                from: Some(erc20_rich_address1),
                max_fee_per_gas: Some(10000000),
                ..Default::default()
            },
        )
        .await
        .expect("Error calling contract: balanceOf(address)(uint256)");
    let token_balance = U256::from_big_endian(
        &hex::decode(token_balance.get(2..).expect("Invalid length"))
            .expect("Invalid response: not hex"),
    );
    assert_eq!(
        token_balance,
        U256::from_dec_str("1000000000000000000000000000000000000000").unwrap()
    );

    // Token balance of ERC20_RICH_ADDRESS2 should be 1.22e20 at latest block
    let token_balance = client
        .call(
            erc20_address,
            [
                erc20_balance_signature.as_slice(),
                H256::from(erc20_rich_address2).as_bytes(),
            ]
            .concat()
            .into(),
            Overrides {
                from: Some(erc20_rich_address1),
                max_fee_per_gas: Some(10000000),
                ..Default::default()
            },
        )
        .await
        .expect("Error calling contract: balanceOf(address)(uint256)");
    let token_balance = U256::from_big_endian(
        &hex::decode(token_balance.get(2..).expect("Invalid length"))
            .expect("Invalid response: not hex"),
    );
    assert_eq!(
        token_balance,
        U256::from_dec_str("122000000000000000000").unwrap()
    );
}

async fn connect() -> EthClient {
    let client = EthClient::new(ETH_RPC_URL);

    let mut retries = 0;
    while retries < 20 {
        match client.get_block_number().await {
            Ok(_) => return client,
            Err(_) => {
                println!("Couldn't get block number. Retries: {retries}");
                retries += 1;
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }

    panic!("Couldn't connect to the RPC server")
}

fn erc20_bytecode() -> Bytes {
    Bytes::copy_from_slice(&hex::decode("608060405234801561000f575f80fd5b5060043610610091575f3560e01c8063313ce56711610064578063313ce5671461013157806370a082311461014f57806395d89b411461017f578063a9059cbb1461019d578063dd62ed3e146101cd57610091565b806306fdde0314610095578063095ea7b3146100b357806318160ddd146100e357806323b872dd14610101575b5f80fd5b61009d6101fd565b6040516100aa9190610a5b565b60405180910390f35b6100cd60048036038101906100c89190610b0c565b61028d565b6040516100da9190610b64565b60405180910390f35b6100eb6102af565b6040516100f89190610b8c565b60405180910390f35b61011b60048036038101906101169190610ba5565b6102b8565b6040516101289190610b64565b60405180910390f35b6101396102e6565b6040516101469190610c10565b60405180910390f35b61016960048036038101906101649190610c29565b6102ee565b6040516101769190610b8c565b60405180910390f35b610187610333565b6040516101949190610a5b565b60405180910390f35b6101b760048036038101906101b29190610b0c565b6103c3565b6040516101c49190610b64565b60405180910390f35b6101e760048036038101906101e29190610c54565b6103e5565b6040516101f49190610b8c565b60405180910390f35b60606003805461020c90610cbf565b80601f016020809104026020016040519081016040528092919081815260200182805461023890610cbf565b80156102835780601f1061025a57610100808354040283529160200191610283565b820191905f5260205f20905b81548152906001019060200180831161026657829003601f168201915b5050505050905090565b5f80610297610467565b90506102a481858561046e565b600191505092915050565b5f600254905090565b5f806102c2610467565b90506102cf858285610480565b6102da858585610513565b60019150509392505050565b5f6012905090565b5f805f8373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20549050919050565b60606004805461034290610cbf565b80601f016020809104026020016040519081016040528092919081815260200182805461036e90610cbf565b80156103b95780601f10610390576101008083540402835291602001916103b9565b820191905f5260205f20905b81548152906001019060200180831161039c57829003601f168201915b5050505050905090565b5f806103cd610467565b90506103da818585610513565b600191505092915050565b5f60015f8473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f8373ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2054905092915050565b5f33905090565b61047b8383836001610603565b505050565b5f61048b84846103e5565b90507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff81101561050d57818110156104fe578281836040517ffb8f41b20000000000000000000000000000000000000000000000000000000081526004016104f593929190610cfe565b60405180910390fd5b61050c84848484035f610603565b5b50505050565b5f73ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff1603610583575f6040517f96c6fd1e00000000000000000000000000000000000000000000000000000000815260040161057a9190610d33565b60405180910390fd5b5f73ffffffffffffffffffffffffffffffffffffffff168273ffffffffffffffffffffffffffffffffffffffff16036105f3575f6040517fec442f050000000000000000000000000000000000000000000000000000000081526004016105ea9190610d33565b60405180910390fd5b6105fe8383836107d2565b505050565b5f73ffffffffffffffffffffffffffffffffffffffff168473ffffffffffffffffffffffffffffffffffffffff1603610673575f6040517fe602df0500000000000000000000000000000000000000000000000000000000815260040161066a9190610d33565b60405180910390fd5b5f73ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff16036106e3575f6040517f94280d620000000000000000000000000000000000000000000000000000000081526004016106da9190610d33565b60405180910390fd5b8160015f8673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f8573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f208190555080156107cc578273ffffffffffffffffffffffffffffffffffffffff168473ffffffffffffffffffffffffffffffffffffffff167f8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925846040516107c39190610b8c565b60405180910390a35b50505050565b5f73ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff1603610822578060025f8282546108169190610d79565b925050819055506108f0565b5f805f8573ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f20549050818110156108ab578381836040517fe450d38c0000000000000000000000000000000000000000000000000000000081526004016108a293929190610cfe565b60405180910390fd5b8181035f808673ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f2081905550505b5f73ffffffffffffffffffffffffffffffffffffffff168273ffffffffffffffffffffffffffffffffffffffff1603610937578060025f8282540392505081905550610981565b805f808473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff1681526020019081526020015f205f82825401925050819055505b8173ffffffffffffffffffffffffffffffffffffffff168373ffffffffffffffffffffffffffffffffffffffff167fddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef836040516109de9190610b8c565b60405180910390a3505050565b5f81519050919050565b5f82825260208201905092915050565b8281835e5f83830152505050565b5f601f19601f8301169050919050565b5f610a2d826109eb565b610a3781856109f5565b9350610a47818560208601610a05565b610a5081610a13565b840191505092915050565b5f6020820190508181035f830152610a738184610a23565b905092915050565b5f80fd5b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f610aa882610a7f565b9050919050565b610ab881610a9e565b8114610ac2575f80fd5b50565b5f81359050610ad381610aaf565b92915050565b5f819050919050565b610aeb81610ad9565b8114610af5575f80fd5b50565b5f81359050610b0681610ae2565b92915050565b5f8060408385031215610b2257610b21610a7b565b5b5f610b2f85828601610ac5565b9250506020610b4085828601610af8565b9150509250929050565b5f8115159050919050565b610b5e81610b4a565b82525050565b5f602082019050610b775f830184610b55565b92915050565b610b8681610ad9565b82525050565b5f602082019050610b9f5f830184610b7d565b92915050565b5f805f60608486031215610bbc57610bbb610a7b565b5b5f610bc986828701610ac5565b9350506020610bda86828701610ac5565b9250506040610beb86828701610af8565b9150509250925092565b5f60ff82169050919050565b610c0a81610bf5565b82525050565b5f602082019050610c235f830184610c01565b92915050565b5f60208284031215610c3e57610c3d610a7b565b5b5f610c4b84828501610ac5565b91505092915050565b5f8060408385031215610c6a57610c69610a7b565b5b5f610c7785828601610ac5565b9250506020610c8885828601610ac5565b9150509250929050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52602260045260245ffd5b5f6002820490506001821680610cd657607f821691505b602082108103610ce957610ce8610c92565b5b50919050565b610cf881610a9e565b82525050565b5f606082019050610d115f830186610cef565b610d1e6020830185610b7d565b610d2b6040830184610b7d565b949350505050565b5f602082019050610d465f830184610cef565b92915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f610d8382610ad9565b9150610d8e83610ad9565b9250828201905080821115610da657610da5610d4c565b5b9291505056fea2646970667358221220ae9dbfc48a1a0a7a0e0ca3eed8cf688efaabc2deee4222f754358b9c171c53a264736f6c634300081a0033").unwrap())
}
