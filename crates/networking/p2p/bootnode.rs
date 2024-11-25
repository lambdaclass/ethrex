use ethrex_core::H512;
use std::{net::SocketAddr, num::ParseIntError, str::FromStr};

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub struct BootNode {
    pub node_id: H512,
    pub socket_address: SocketAddr,
}

impl FromStr for BootNode {
    type Err = ParseIntError;
    /// Takes a str with the format "enode://nodeID@IPaddress:port" and
    /// parses it to a BootNode
    // TODO: fix it to support different UDP and TCP ports, according to
    // https://github.com/lambdaclass/ethrex/issues/905
    fn from_str(input: &str) -> Result<BootNode, ParseIntError> {
        // TODO: error handling
        let node_id = H512::from_str(&input[8..136]).expect("Failed to parse node id");
        let socket_address: SocketAddr = input[137..]
            .parse()
            .expect("Failed to parse bootnode address and port");
        Ok(BootNode {
            node_id,
            socket_address,
        })
    }
}

pub fn decode_hex(s: &str) -> Result<Vec<u8>, ParseIntError> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect()
}

#[test]
fn parse_bootnode_from_string() {
    let input = "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303";
    let bootnode = BootNode::from_str(input).unwrap();
    let node_id = H512::from_str(
        "d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666")
        .unwrap();
    let socket_address = SocketAddr::from_str("18.138.108.67:30303").unwrap();
    let expected_bootnode = BootNode {
        node_id,
        socket_address,
    };
    assert_eq!(bootnode, expected_bootnode);
}
