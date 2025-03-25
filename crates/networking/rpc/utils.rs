use serde_json::Value;

pub fn parse_json_hex(hex: &serde_json::Value) -> Result<u64, String> {
    if let Value::String(maybe_hex) = hex {
        let trimmed = maybe_hex.trim_start_matches("0x");
        let maybe_parsed = u64::from_str_radix(trimmed, 16);
        maybe_parsed.map_err(|_| format!("Could not parse given hex {}", maybe_hex))
    } else {
        Err(format!("Could not parse given hex {}", hex))
    }
}

#[cfg(test)]
pub mod test_utils {
    use std::{net::SocketAddr, str::FromStr, sync::Arc};

    use ethrex_blockchain::Blockchain;
    use ethrex_common::H512;
    use ethrex_p2p::{
        sync::SyncManager,
        types::{Node, NodeRecord},
    };
    use ethrex_storage::{EngineType, Store};
    use k256::ecdsa::SigningKey;

    use crate::server::start_api;
    #[cfg(feature = "based")]
    use crate::{EngineClient, EthClient};
    #[cfg(feature = "based")]
    use bytes::Bytes;
    #[cfg(feature = "l2")]
    use secp256k1::{rand, SecretKey};

    pub const TEST_GENESIS: &str = include_str!("../../../test_data/genesis-l1.json");

    pub fn example_p2p_node() -> Node {
        let node_id_1 = H512::from_str("d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666").unwrap();
        Node {
            ip: "127.0.0.1".parse().unwrap(),
            udp_port: 30303,
            tcp_port: 30303,
            node_id: node_id_1,
        }
    }

    pub fn example_local_node_record() -> NodeRecord {
        let node_id_1 = H512::from_str("d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666").unwrap();
        let node = Node {
            ip: "127.0.0.1".parse().unwrap(),
            udp_port: 30303,
            tcp_port: 30303,
            node_id: node_id_1,
        };
        let signer = SigningKey::random(&mut rand::rngs::OsRng);

        NodeRecord::from_node(node, 0, &signer).unwrap()
    }

    // Util to start an api for testing on ports 8500 and 8501,
    // mostly for when hive is missing some endpoints to test
    // like eth_uninstallFilter.
    // Here's how you would use it:
    // ```
    // let server_handle = tokio::spawn(async move { start_stest_api().await })
    // ...
    // assert!(something_that_needs_the_server)
    // ...
    // server_handle.abort()
    // ```
    pub async fn start_test_api() {
        let http_addr: SocketAddr = "127.0.0.1:8500".parse().unwrap();
        let authrpc_addr: SocketAddr = "127.0.0.1:8501".parse().unwrap();
        let storage =
            Store::new("", EngineType::InMemory).expect("Failed to create in-memory storage");
        storage
            .add_initial_state(serde_json::from_str(TEST_GENESIS).unwrap())
            .expect("Failed to build test genesis");
        let blockchain = Arc::new(Blockchain::default_with_store(storage.clone()));
        let jwt_secret = Default::default();
        let local_p2p_node = example_p2p_node();
        #[cfg(feature = "based")]
        let gateway_eth_client = EthClient::new("");
        #[cfg(feature = "based")]
        let gateway_auth_client = EngineClient::new("", Bytes::default());
        #[cfg(feature = "l2")]
        let valid_delegation_addresses = Vec::new();
        #[cfg(feature = "l2")]
        let sponsor_pk = SecretKey::new(&mut rand::thread_rng());
        start_api(
            http_addr,
            authrpc_addr,
            storage,
            blockchain,
            jwt_secret,
            local_p2p_node,
            example_local_node_record(),
            SyncManager::dummy(),
            #[cfg(feature = "based")]
            gateway_eth_client,
            #[cfg(feature = "based")]
            gateway_auth_client,
            #[cfg(feature = "l2")]
            valid_delegation_addresses,
            #[cfg(feature = "l2")]
            sponsor_pk,
        )
        .await;
    }
}
