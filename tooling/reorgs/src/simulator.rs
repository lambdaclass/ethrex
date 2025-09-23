use std::{fs::File, path::PathBuf, process::Stdio, time::Duration};

use ethrex::{cli::Options, initializers::get_network};
use ethrex_common::{
    Bytes, H160, H256, U256,
    types::{
        Block, EIP1559Transaction, Genesis, Transaction, TxKind, requests::compute_requests_hash,
    },
};
use ethrex_config::networks::Network;
use ethrex_l2_rpc::signer::{Signable, Signer};
use ethrex_rpc::{
    EngineClient, EthClient,
    types::{
        block_identifier::{BlockIdentifier, BlockTag},
        fork_choice::{ForkChoiceState, PayloadAttributesV3},
        payload::{ExecutionPayload, PayloadValidationStatus},
    },
};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub struct Simulator {
    cmd_path: PathBuf,
    base_opts: Options,
    jwt_secret: Bytes,
    genesis_path: PathBuf,
    configs: Vec<Options>,
    cancellation_tokens: Vec<CancellationToken>,
}

impl Simulator {
    pub fn new(cmd_path: PathBuf) -> Self {
        let mut opts = Options::default_l1();
        let jwt_secret = generate_jwt_secret();
        std::fs::write("jwt.hex", hex::encode(&jwt_secret)).unwrap();

        let genesis_path = std::path::absolute("../../fixtures/genesis/l1-dev.json")
            .unwrap()
            .canonicalize()
            .unwrap();

        opts.authrpc_jwtsecret = "jwt.hex".to_string();
        opts.dev = false;
        opts.http_addr = "localhost".to_string();
        opts.authrpc_addr = "localhost".to_string();
        opts.network = Some(Network::GenesisPath(genesis_path.clone()));
        Self {
            cmd_path,
            base_opts: opts,
            genesis_path,
            jwt_secret,
            configs: vec![],
            cancellation_tokens: vec![],
        }
    }

    pub fn get_base_chain(&self) -> Chain {
        let network = get_network(&self.base_opts);
        let genesis = network.get_genesis().unwrap();
        Chain::new(genesis)
    }

    pub async fn start_node(&mut self) -> Node {
        let n = self.configs.len();
        let mut opts = self.base_opts.clone();
        opts.http_port = (8545 + n * 2).to_string();
        opts.authrpc_port = (8545 + n * 2 + 1).to_string();
        opts.p2p_port = (30303 + n).to_string();
        opts.discovery_port = (30303 + n).to_string();
        opts.datadir = format!("data/node{n}").into();

        let _ = std::fs::remove_dir_all(&opts.datadir);
        std::fs::create_dir_all(&opts.datadir).expect("Failed to create data directory");

        let logs_file =
            File::create(format!("data/node{n}.log")).expect("Failed to create logs file");

        let cancel = CancellationToken::new();

        self.configs.push(opts.clone());
        self.cancellation_tokens.push(cancel.clone());

        let mut cmd = Command::new(&self.cmd_path);
        cmd.args([
            format!("--http.addr={}", opts.http_addr),
            format!("--http.port={}", opts.http_port),
            format!("--authrpc.addr={}", opts.authrpc_addr),
            format!("--authrpc.port={}", opts.authrpc_port),
            format!("--p2p.port={}", opts.p2p_port),
            format!("--discovery.port={}", opts.discovery_port),
            format!("--datadir={}", opts.datadir.display()),
            format!("--network={}", self.genesis_path.display()),
            "--force".to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(logs_file.try_clone().expect("Failed to clone logs file"))
        .stderr(logs_file);

        let child = cmd.spawn().expect("Failed to start ethrex process");

        tokio::spawn(async move {
            let mut child = child;
            tokio::select! {
                _ = cancel.cancelled() => {
                    if let Some(pid) = child.id() {
                        signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM).unwrap();
                    }
                }
                res = child.wait() => {
                    assert!(res.unwrap().success());
                }
            }
        });

        info!(
            "Started node {n} at http://{}:{}",
            opts.http_addr, opts.http_port
        );

        tokio::time::sleep(Duration::from_millis(200)).await;

        self.get_node(n)
    }

    pub fn stop(&self) {
        for token in &self.cancellation_tokens {
            token.cancel();
        }
    }

    fn get_http_url(&self, index: usize) -> String {
        let opts = &self.configs[index];
        format!("http://{}:{}", opts.http_addr, opts.http_port)
    }

    fn get_auth_url(&self, index: usize) -> String {
        let opts = &self.configs[index];
        format!("http://{}:{}", opts.authrpc_addr, opts.authrpc_port)
    }

    fn get_node(&self, index: usize) -> Node {
        let auth_url = self.get_auth_url(index);
        let engine_client = EngineClient::new(&auth_url, self.jwt_secret.clone());

        let http_url = self.get_http_url(index);
        let rpc_client = EthClient::new(&http_url).unwrap();

        Node {
            index,
            engine_client,
            rpc_client,
        }
    }
}

pub struct Node {
    index: usize,
    engine_client: EngineClient,
    rpc_client: EthClient,
}

impl Node {
    pub async fn update_forkchoice(&self, chain: &Chain) {
        let fork_choice_state = chain.get_fork_choice_state();
        info!(
            node = self.index,
            head = %fork_choice_state.head_block_hash,
            "Updating fork choice"
        );

        let fork_choice_response = self
            .engine_client
            .engine_forkchoice_updated_v3(fork_choice_state, None)
            .await
            .unwrap();

        assert_eq!(
            fork_choice_response.payload_status.status,
            PayloadValidationStatus::Valid,
            "Validation failed with error: {:?}",
            fork_choice_response.payload_status.validation_error
        );
        assert!(fork_choice_response.payload_id.is_none());
    }

    pub async fn build_payload(&self, mut chain: Chain) -> Chain {
        let fork_choice_state = chain.get_fork_choice_state();
        let payload_attributes = chain.get_next_payload_attributes();
        let head = fork_choice_state.head_block_hash;

        let parent_beacon_block_root = payload_attributes.parent_beacon_block_root;

        info!(
            node = self.index,
            %head,
            "Starting payload build"
        );

        let fork_choice_response = self
            .engine_client
            .engine_forkchoice_updated_v3(fork_choice_state, Some(payload_attributes))
            .await
            .unwrap();

        assert_eq!(
            fork_choice_response.payload_status.status,
            PayloadValidationStatus::Valid,
            "Validation failed with error: {:?}",
            fork_choice_response.payload_status.validation_error
        );
        let payload_id = fork_choice_response.payload_id.unwrap();

        let payload_response = self
            .engine_client
            .engine_get_payload_v4(payload_id)
            .await
            .unwrap();

        let requests_hash = compute_requests_hash(&payload_response.execution_requests.unwrap());
        let block = payload_response
            .execution_payload
            .into_block(parent_beacon_block_root, Some(requests_hash))
            .unwrap();

        info!(
            node = self.index,
            %head,
            block = %block.hash(),
            "#txs"=%block.body.transactions.len(),
            "Built payload"
        );
        chain.append_block(block);
        chain
    }

    pub async fn notify_new_payload(&self, chain: &Chain) {
        let head = chain.blocks.last().unwrap();
        let execution_payload = ExecutionPayload::from_block(head.clone());
        // Support blobs
        // let commitments = execution_payload_response
        //     .blobs_bundle
        //     .unwrap_or_default()
        //     .commitments
        //     .iter()
        //     .map(|commitment| {
        //         let mut hash = keccak256(commitment).0;
        //         // https://eips.ethereum.org/EIPS/eip-4844 -> kzg_to_versioned_hash
        //         hash[0] = 0x01;
        //         H256::from_slice(&hash)
        //     })
        //     .collect();
        let commitments = vec![];
        let parent_beacon_block_root = head.header.parent_beacon_block_root.unwrap();
        let payload_status = self
            .engine_client
            .engine_new_payload_v4(execution_payload, commitments, parent_beacon_block_root)
            .await
            .unwrap();

        assert_eq!(
            payload_status.status,
            PayloadValidationStatus::Valid,
            "Validation failed with error: {:?}",
            payload_status.validation_error
        );
    }

    pub async fn send_eth_transfer(&self, signer: &Signer, recipient: H160, amount: u64) {
        info!(node = self.index, sender=%signer.address(), %recipient, amount, "Sending ETH transfer tx");
        let chain_id = self
            .rpc_client
            .get_chain_id()
            .await
            .unwrap()
            .try_into()
            .unwrap();
        let sender_address = signer.address();
        let nonce = self
            .rpc_client
            .get_nonce(sender_address, BlockIdentifier::Tag(BlockTag::Latest))
            .await
            .unwrap();
        let tx = EIP1559Transaction {
            chain_id,
            nonce,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 1_000_000_000,
            gas_limit: 50_000,
            to: TxKind::Call(recipient),
            value: amount.into(),
            ..Default::default()
        };
        let mut tx = Transaction::EIP1559Transaction(tx);
        tx.sign_inplace(signer).await.unwrap();
        let encoded_tx = tx.encode_canonical_to_vec();
        self.rpc_client
            .send_raw_transaction(&encoded_tx)
            .await
            .unwrap();
    }

    pub async fn get_balance(&self, address: H160) -> U256 {
        self.rpc_client
            .get_balance(address, Default::default())
            .await
            .unwrap()
    }
}

pub struct Chain {
    block_hashes: Vec<H256>,
    blocks: Vec<Block>,
    safe_height: usize,
}

impl Chain {
    fn new(genesis: Genesis) -> Self {
        let genesis_block = genesis.get_block();
        Self {
            block_hashes: vec![genesis_block.hash()],
            blocks: vec![genesis_block],
            safe_height: 0,
        }
    }

    fn append_block(&mut self, block: Block) {
        self.block_hashes.push(block.hash());
        self.blocks.push(block);
    }

    pub fn fork(&self) -> Self {
        Self {
            block_hashes: self.block_hashes.clone(),
            blocks: self.blocks.clone(),
            safe_height: self.safe_height,
        }
    }

    fn get_fork_choice_state(&self) -> ForkChoiceState {
        let head_block_hash = *self.block_hashes.last().unwrap();
        let finalized_block_hash = self.block_hashes[self.safe_height];
        ForkChoiceState {
            head_block_hash,
            safe_block_hash: finalized_block_hash,
            finalized_block_hash,
        }
    }

    fn get_next_payload_attributes(&self) -> PayloadAttributesV3 {
        let timestamp = self.blocks.last().unwrap().header.timestamp + 12;
        let head_hash = self.get_fork_choice_state().head_block_hash;
        // Generate dummy values by hashing multiple times
        let parent_beacon_block_root = keccak256(&head_hash.0);
        let prev_randao = keccak256(&parent_beacon_block_root.0);
        // Address of 0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e, a rich account
        let suggested_fee_recipient = H160(
            hex::decode("4417092B70a3E5f10Dc504d0947DD256B965fc62")
                .unwrap()
                .try_into()
                .unwrap(),
        );
        // TODO: add withdrawals
        let withdrawals = vec![];
        PayloadAttributesV3 {
            timestamp,
            prev_randao,
            suggested_fee_recipient,
            parent_beacon_block_root: Some(parent_beacon_block_root),
            withdrawals: Some(withdrawals),
        }
    }
}

fn generate_jwt_secret() -> Bytes {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut secret = [0u8; 32];
    rng.fill(&mut secret);
    Bytes::from(secret.to_vec())
}

fn keccak256(data: &[u8]) -> H256 {
    H256(
        Sha256::new_with_prefix(data)
            .finalize()
            .as_slice()
            .try_into()
            .unwrap(),
    )
}
