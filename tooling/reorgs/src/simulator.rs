use std::{
    fs::File,
    io::Read,
    path::PathBuf,
    process::Stdio,
    sync::atomic::AtomicU16,
    time::{Duration, SystemTime},
};

use ethrex::{cli::Options, initializers::get_network};
use ethrex_common::{
    Address, Bytes, H160, H256, U256,
    evm::calculate_create_address,
    types::{
        Block, EIP1559Transaction, Genesis, Transaction, TxKind, requests::compute_requests_hash,
    },
};
use ethrex_config::networks::Network;
use ethrex_l2_rpc::signer::{Signable, Signer};
use ethrex_p2p::sync::SyncMode;
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
use tracing::{error, info};
use url::Url;

pub struct Simulator {
    cmd_path: PathBuf,
    test_name: String,

    base_opts: Options,
    jwt_secret: Bytes,
    genesis_path: PathBuf,
    configs: Vec<Options>,
    enodes: Vec<String>,
    cancellation_tokens: Vec<(CancellationToken, tokio::task::JoinHandle<()>)>,
}

impl Simulator {
    pub fn new(cmd_path: PathBuf, test_name: String) -> Self {
        let mut opts = Options::default_l1();
        let jwt_secret = generate_jwt_secret();
        std::fs::write("jwt.hex", hex::encode(&jwt_secret)).unwrap();

        let genesis_path = std::path::absolute("../../fixtures/genesis/l1.json")
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
            test_name,
            base_opts: opts,
            genesis_path,
            jwt_secret,
            configs: vec![],
            cancellation_tokens: vec![],
            enodes: vec![],
        }
    }

    pub fn get_base_chain(&self) -> Chain {
        let network = get_network(&self.base_opts);
        let genesis = network.get_genesis().unwrap();
        Chain::new(genesis)
    }

    pub async fn start_node(&mut self) -> Node {
        let n = self.configs.len();
        let test_name = &self.test_name;
        info!(node = n, "Starting node");
        let mut opts = self.base_opts.clone();
        opts.datadir = format!("data/{test_name}/node{n}").into();

        opts.http_port = get_next_port().to_string();
        opts.authrpc_port = get_next_port().to_string();

        // These are one TCP and one UDP
        let p2p_port = get_next_port();
        opts.p2p_port = p2p_port.to_string();
        opts.discovery_port = p2p_port.to_string();

        opts.syncmode = SyncMode::Full;

        if opts.datadir.exists() {
            std::fs::remove_dir_all(&opts.datadir)
                .expect("Failed to remove existing data directory");
        }
        std::fs::create_dir_all(&opts.datadir).expect("Failed to create data directory");

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let logs_file_path = format!("data/{test_name}/node{n}_{now}.log");
        let logs_file = File::create(&logs_file_path).expect("Failed to create logs file");

        let cancel = CancellationToken::new();

        self.configs.push(opts.clone());

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
            format!("--syncmode={:?}", opts.syncmode).to_lowercase(),
            "--force".to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(logs_file.try_clone().unwrap())
        .stderr(logs_file);

        if !self.enodes.is_empty() {
            cmd.arg(format!("--bootnodes={}", self.enodes.join(",")));
        }

        let child = cmd.spawn().expect("Failed to start ethrex process");

        let logs_file = File::open(&logs_file_path).expect("Failed to open logs file");
        let enode =
            tokio::time::timeout(Duration::from_secs(5), wait_for_initialization(logs_file))
                .await
                .expect("node initialization timed out");
        self.enodes.push(enode);

        let waiter = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                let mut child = child;
                tokio::select! {
                    _ = cancel.cancelled() => {
                        if let Some(pid) = child.id() {
                            // NOTE: we use SIGTERM instead of child.kill() so sockets are closed
                            signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM).unwrap();
                        }
                    }
                    res = child.wait() => {
                        assert!(res.unwrap().success());
                    }
                }
                // Ignore any errors on shutdown
                let _ = child.wait().await.unwrap();
            }
        });
        self.cancellation_tokens.push((cancel, waiter));

        info!(
            "Started node {n} at http://{}:{}",
            opts.http_addr, opts.http_port
        );

        self.get_node(n)
    }

    pub async fn stop(&mut self) {
        for (token, waiter) in self.cancellation_tokens.drain(..) {
            token.cancel();
            waiter.await.unwrap();
        }
        self.enodes.clear();
        self.configs.clear();
    }

    fn get_http_url(&self, index: usize) -> Url {
        let opts = &self.configs[index];
        Url::parse(&format!("http://{}:{}", opts.http_addr, opts.http_port))
            .expect("Error parsing RPC URL")
    }

    fn get_auth_url(&self, index: usize) -> String {
        let opts = &self.configs[index];
        format!("http://{}:{}", opts.authrpc_addr, opts.authrpc_port)
    }

    fn get_node(&self, index: usize) -> Node {
        let auth_url = self.get_auth_url(index);
        let engine_client = EngineClient::new(&auth_url, self.jwt_secret.clone());

        let http_url = self.get_http_url(index);
        let rpc_client = EthClient::new(http_url).unwrap();

        Node {
            index,
            engine_client,
            rpc_client,
        }
    }
}

/// Waits until the node is initialized by reading its logs.
/// Returns the enode URL of the node.
async fn wait_for_initialization(mut logs_file: File) -> String {
    const NODE_STARTED_LOG: &str = "Starting Auth-RPC server at";

    let mut file_contents = String::new();

    // Wait a bit until the node starts
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;

        logs_file.read_to_string(&mut file_contents).unwrap();

        if file_contents.contains(NODE_STARTED_LOG) {
            break;
        }
    }
    let node_enode_log = file_contents
        .lines()
        .find(|line| line.contains("Local node initialized"))
        .unwrap();
    // Look for the "enode://node_id@host:port" part
    let prefix = "enode://";
    let node_enode = node_enode_log.split_once(prefix).unwrap().1;
    format!("{prefix}{}", node_enode.trim_end())
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
        let syncing_fut = wait_until_synced(&self.engine_client, fork_choice_state);

        tokio::time::timeout(Duration::from_secs(5), syncing_fut)
            .await
            .inspect_err(|_| {
                error!(node = self.index, "Timed out waiting for node to sync");
            })
            .expect("timed out waiting for node to sync");
    }

    pub async fn build_payload(&self, mut chain: Chain) -> Chain {
        let fork_choice_state = chain.get_fork_choice_state();
        let mut payload_attributes = chain.get_next_payload_attributes();
        // Set index as fee recipient to differentiate between nodes
        payload_attributes.suggested_fee_recipient = H160::from_low_u64_be(self.index as u64);
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
            .engine_get_payload_v5(payload_id)
            .await
            .unwrap();

        let requests_hash = compute_requests_hash(&payload_response.execution_requests.unwrap());
        let block_access_list_hash = payload_response
            .execution_payload
            .block_access_list
            .as_ref()
            .map(|bal| bal.compute_hash());
        let block = payload_response
            .execution_payload
            .into_block(
                parent_beacon_block_root,
                Some(requests_hash),
                block_access_list_hash,
            )
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

    pub async fn extend_chain(&self, mut chain: Chain, num_blocks: usize) -> Chain {
        for _ in 0..num_blocks {
            chain = self.build_payload(chain).await;
            self.notify_new_payload(&chain).await;
        }
        self.update_forkchoice(&chain).await;
        chain
    }

    pub async fn notify_new_payload(&self, chain: &Chain) {
        let head = chain.blocks.last().unwrap();
        let execution_payload = ExecutionPayload::from_block(head.clone(), None);
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
        let _payload_status = self
            .engine_client
            .engine_new_payload_v4(execution_payload, commitments, parent_beacon_block_root)
            .await
            .unwrap();
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

    pub async fn send_call(&self, signer: &Signer, contract: H160, data: Bytes) {
        info!(node = self.index, sender=%signer.address(), %contract, "Sending contract call");
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
            to: TxKind::Call(contract),
            data,
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

    pub async fn send_contract_deploy(
        &self,
        signer: &Signer,
        contract_deploy_bytecode: Bytes,
    ) -> Address {
        info!(node = self.index, sender=%signer.address(), "Deploying contract");
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
            gas_limit: 100_000,
            to: TxKind::Create,
            data: contract_deploy_bytecode,
            ..Default::default()
        };
        let mut tx = Transaction::EIP1559Transaction(tx);
        tx.sign_inplace(signer).await.unwrap();
        let encoded_tx = tx.encode_canonical_to_vec();
        self.rpc_client
            .send_raw_transaction(&encoded_tx)
            .await
            .unwrap();

        calculate_create_address(sender_address, nonce)
    }

    pub async fn get_balance(&self, address: H160) -> U256 {
        self.rpc_client
            .get_balance(address, Default::default())
            .await
            .unwrap()
    }

    pub async fn get_storage_at(&self, address: H160, key: U256) -> U256 {
        self.rpc_client
            .get_storage_at(address, key, Default::default())
            .await
            .unwrap()
    }
}

#[derive(Debug)]
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
        let suggested_fee_recipient = Default::default();
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
    let digest = Sha256::new_with_prefix(data).finalize();
    H256::from_slice(digest.as_ref())
}

async fn wait_until_synced(engine_client: &EngineClient, fork_choice_state: ForkChoiceState) {
    loop {
        let fork_choice_response = engine_client
            .engine_forkchoice_updated_v3(fork_choice_state, None)
            .await
            .unwrap();

        let status = fork_choice_response.payload_status.status;
        if status == PayloadValidationStatus::Valid {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn get_next_port() -> u16 {
    static NEXT_PORT: AtomicU16 = AtomicU16::new(8560);
    NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
