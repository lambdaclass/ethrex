use std::str::FromStr;

use ethrex_net::bootnode::BootNode;
use lazy_static::lazy_static;

pub const HOLESKY_GENESIS_PATH: &str = "cmd/ethrex/genesis_holesky.json";

lazy_static! {
    pub static ref HOLESKY_NODES: Vec<BootNode> = vec![
        BootNode::from_str("enode://ac906289e4b7f12df423d654c5a962b6ebe5b3a74cc9e06292a85221f9a64a6f1cfdd6b714ed6dacef51578f92b34c60ee91e9ede9c7f8fadc4d347326d95e2b@146.190.13.128:30303").unwrap(),
        BootNode::from_str("enode://a3435a0155a3e837c02f5e7f5662a2f1fbc25b48e4dc232016e1c51b544cb5b4510ef633ea3278c0e970fa8ad8141e2d4d0f9f95456c537ff05fdf9b31c15072@178.128.136.233:30303").unwrap(),
        BootNode::from_str("enode://7fa09f1e8bb179ab5e73f45d3a7169a946e7b3de5ef5cea3a0d4546677e4435ee38baea4dd10b3ddfdc1f1c5e869052932af8b8aeb6f9738598ec4590d0b11a6@65.109.94.124:30303").unwrap(),
        BootNode::from_str("enode://3524632a412f42dee4b9cc899b946912359bb20103d7596bddb9c8009e7683b7bff39ea20040b7ab64d23105d4eac932d86b930a605e632357504df800dba100@172.174.35.249:30303").unwrap(),
    ];
}