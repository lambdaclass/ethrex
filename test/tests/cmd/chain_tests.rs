//! A chain defined outside the `ethrex` crate.
//!
//! `Example` is an invented chain. It uses everything a chain can add to the node:
//! named networks, its own flag, an RPC namespace, and a mempool rule. The
//! tests check each one separately, and then all of them on a running node.

use std::{net::Ipv4Addr, sync::Arc};

use clap::CommandFactory;
use ethrex::{
    chain::{Chain, ETHEREUM, NoArgs, NodeComponents, command, parse_args},
    cli::CLI,
    initializers::{get_network, init_node},
};
use ethrex_blockchain::{Blockchain, BlockchainOptions, MempoolPolicy, error::MempoolError};
use ethrex_common::{
    Address, H160, H512, U256,
    types::{BlockHeader, EIP1559Transaction, Genesis, P2PTransaction, Transaction, TxKind},
};
use ethrex_config::networks::{
    LOCAL_DEVNET_GENESIS_CONTENTS, LOCAL_DEVNET_PRIVATE_KEYS, Network, NetworkPreset, PublicNetwork,
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_p2p::{rlpx::eth::transactions::PooledTransactions, types::Node as Peer};
use ethrex_rpc::{
    RpcApiContext, RpcErr, RpcExtension, RpcExtensionFuture, RpcExtensions, RpcStartupError,
    map_http_requests,
    rpc::map_authrpc_requests,
    test_utils::{context_without_p2p, setup_store},
    utils::RpcRequest,
};
use ethrex_storage::{EngineType, Store};
use futures::{SinkExt, StreamExt};
use secp256k1::SecretKey;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

const EXAMPLE_DEV: NetworkPreset = NetworkPreset {
    name: "example-dev",
    genesis: LOCAL_DEVNET_GENESIS_CONTENTS,
    bootnodes: "[]",
};

const EXAMPLE_TEST: NetworkPreset = NetworkPreset {
    name: "example-test",
    genesis: LOCAL_DEVNET_GENESIS_CONTENTS,
    bootnodes: "[]",
};

#[derive(clap::Args)]
struct ExampleArgs {
    /// Highest gas limit a transaction may have to enter the pool.
    #[arg(long = "example.max-tx-gas", default_value_t = 1_000_000)]
    max_tx_gas: u64,
}

const EXAMPLE: Chain<ExampleArgs> = Chain {
    name: "ethrex-example",
    networks: &[EXAMPLE_DEV, EXAMPLE_TEST],
    components: example_components,
};

fn example_components(args: &ExampleArgs, _genesis: &Genesis) -> eyre::Result<NodeComponents> {
    let mut components = NodeComponents::default();
    components.rpc.push(Arc::new(ExampleRpc));
    components.mempool_policy = Some(Arc::new(MaxTxGas(args.max_tx_gas)));
    Ok(components)
}

/// Serves `example_ping`, `example_chainId` by calling the built-in `eth_chainId`, and
/// `example_pingItself` by calling `example_ping` the same way.
struct ExampleRpc;

impl RpcExtension for ExampleRpc {
    fn namespace(&self) -> &'static str {
        "example"
    }

    fn handle<'a>(&'a self, req: &'a RpcRequest, context: RpcApiContext) -> RpcExtensionFuture<'a> {
        Box::pin(async move {
            match req.method.as_str() {
                "example_ping" => Ok(json!("pong")),
                "example_chainId" => {
                    map_http_requests(&RpcRequest::new("eth_chainId", None), context).await
                }
                "example_pingItself" => {
                    map_http_requests(&RpcRequest::new("example_ping", None), context).await
                }
                _ => Err(RpcErr::MethodNotFound(req.method.clone())),
            }
        })
    }
}

#[derive(Debug)]
struct MaxTxGas(u64);

impl MempoolPolicy for MaxTxGas {
    fn admit(&self, tx: &Transaction, _sender: Address, _head: &BlockHeader) -> Result<(), String> {
        if tx.gas_limit() > self.0 {
            return Err(format!(
                "gas limit {} is above the example chain's cap of {}",
                tx.gas_limit(),
                self.0
            ));
        }
        Ok(())
    }
}

fn parse<A: clap::Args>(chain: &Chain<A>, args: &[&str]) -> (CLI, A) {
    let argv = std::iter::once(chain.name).chain(args.iter().copied());
    match parse_args(chain, argv) {
        Ok(parsed) => parsed,
        Err(err) => panic!("arguments {args:?} should parse: {err}"),
    }
}

// ---- command line ----

#[test]
fn ethereum_adds_nothing_to_the_command_line() {
    let stock = CLI::command().render_long_help().to_string();
    let ethereum = command(&ETHEREUM).render_long_help().to_string();
    assert_eq!(stock, ethereum);

    let (cli, _) = parse(&ETHEREUM, &[]);
    assert_eq!(cli.opts.network, None);
    assert_eq!(get_network(&cli.opts), Network::mainnet());
}

#[test]
fn chain_flag_is_parsed_with_the_node_flags() {
    let (cli, args) = parse(
        &EXAMPLE,
        &["--example.max-tx-gas", "5", "--http.port", "1234"],
    );
    assert_eq!(args.max_tx_gas, 5);
    assert_eq!(cli.opts.http_port, "1234");

    let (_, args) = parse(&EXAMPLE, &[]);
    assert_eq!(args.max_tx_gas, 1_000_000);
}

#[test]
fn chain_flag_is_unknown_to_another_binary() {
    let err = parse_args(&ETHEREUM, ["ethrex", "--example.max-tx-gas", "5"])
        .err()
        .expect("the stock binary must reject another chain's flag");
    assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
}

/// clap identifies a flag by its field name and a flags struct by its type name,
/// so a chain that reuses one of the node's would share its value, and one that
/// reuses a `long` or a `short` would shadow it. That is refused, in every build
/// profile.
#[test]
fn chain_flags_cannot_take_a_name_the_node_uses() {
    fn refusal<A: clap::Args>() -> String {
        let chain: Chain<A> = Chain {
            name: "ethrex-example",
            networks: &[],
            components: |_, _| Ok(NodeComponents::default()),
        };
        match parse_args(&chain, ["ethrex-example"]) {
            Ok(_) => panic!("parsed with a name used twice"),
            Err(err) => err.to_string(),
        }
    }

    /// A field named like a node flag.
    #[derive(clap::Args)]
    struct SameField {
        #[arg(long = "example.datadir", default_value = "elsewhere")]
        datadir: String,
    }
    let err = refusal::<SameField>();
    assert!(err.contains("`datadir` is defined twice"), "{err}");

    /// A field named like a flag clap adds itself.
    #[derive(clap::Args)]
    struct BuiltInField {
        #[arg(long = "example.help", default_value = "none")]
        help: String,
    }
    let err = refusal::<BuiltInField>();
    assert!(err.contains("`help` is defined twice"), "{err}");

    /// A struct named like the node's.
    #[derive(clap::Args)]
    struct Options {
        #[arg(long = "example.flag")]
        flag: bool,
    }
    let err = refusal::<Options>();
    assert!(err.contains("`Options` is defined twice"), "{err}");

    /// A flag typed like a node flag, under another field name.
    #[derive(clap::Args)]
    struct SameLong {
        #[arg(long = "network", default_value = "elsewhere")]
        example_network: String,
    }
    let err = refusal::<SameLong>();
    assert!(err.contains("`--network` is defined twice"), "{err}");

    /// A short flag clap adds itself.
    #[derive(clap::Args)]
    struct BuiltInShort {
        #[arg(short = 'h', long = "example.height", default_value_t = 0)]
        example_height: u64,
    }
    let err = refusal::<BuiltInShort>();
    assert!(err.contains("`-h` is defined twice"), "{err}");
}

#[test]
fn preset_name_selects_the_preset() {
    let (cli, _) = parse(&EXAMPLE, &["--network", "example-test"]);
    assert_eq!(cli.opts.network, Some(Network::Preset(EXAMPLE_TEST)));
}

#[test]
fn first_preset_is_the_default_network() {
    let (cli, _) = parse(&EXAMPLE, &[]);
    assert_eq!(cli.opts.network, Some(Network::Preset(EXAMPLE_DEV)));
}

#[test]
fn dev_mode_keeps_its_local_network() {
    let (cli, _) = parse(&EXAMPLE, &["--dev"]);
    assert_eq!(cli.opts.network, None);
    assert_eq!(get_network(&cli.opts), Network::LocalDevnet);
}

#[test]
fn ethereum_networks_and_genesis_paths_are_still_accepted() {
    let (cli, _) = parse(&EXAMPLE, &["--network", "hoodi"]);
    assert_eq!(
        cli.opts.network,
        Some(Network::PublicNetwork(PublicNetwork::Hoodi))
    );

    let (cli, _) = parse(&EXAMPLE, &["--network", "/some/genesis.json"]);
    assert_eq!(
        cli.opts.network,
        Some(Network::GenesisPath("/some/genesis.json".into()))
    );
}

#[test]
fn preset_name_is_a_genesis_path_for_another_binary() {
    let (cli, _) = parse(&ETHEREUM, &["--network", "example-test"]);
    assert_eq!(
        cli.opts.network,
        Some(Network::GenesisPath("example-test".into()))
    );
}

#[test]
fn help_lists_the_presets() {
    let help = command(&EXAMPLE).render_long_help().to_string();
    assert!(help.contains("example-dev, example-test"), "{help}");
    assert!(help.contains("defaults to example-dev"), "{help}");
    // The built-in networks are still accepted, and still listed.
    assert!(
        help.contains("sepolia, hoodi, plataberget and mainnet"),
        "{help}"
    );
    assert!(!help.contains("defaults to mainnet"), "{help}");
    assert!(help.contains("--example.max-tx-gas"), "{help}");
}

#[test]
fn malformed_preset_is_rejected_at_startup() {
    const BROKEN: NetworkPreset = NetworkPreset {
        name: "broken",
        genesis: "{",
        bootnodes: "[]",
    };
    fn chain(networks: &'static [NetworkPreset]) -> Chain<NoArgs> {
        Chain {
            name: "ethrex-broken",
            networks,
            components: |_, _| Ok(NodeComponents::default()),
        }
    }

    let err = parse_args(&chain(&[BROKEN]), ["ethrex-broken"])
        .err()
        .expect("a preset whose genesis does not parse must be rejected");
    assert!(err.to_string().contains("invalid genesis"), "{err}");

    // Also when another network is the one selected.
    let err = parse_args(
        &chain(&[EXAMPLE_DEV, BROKEN]),
        ["ethrex-broken", "--network", "example-dev"],
    )
    .err()
    .expect("a malformed preset must be rejected even when it is not selected");
    assert!(err.to_string().contains("invalid genesis"), "{err}");
}

#[test]
fn preset_names_must_be_distinct_from_each_other_and_from_built_in_networks() {
    fn chain(networks: &'static [NetworkPreset]) -> Chain<NoArgs> {
        Chain {
            name: "ethrex-misnamed",
            networks,
            components: |_, _| Ok(NodeComponents::default()),
        }
    }
    const NAMED_LIKE_ETHEREUM: NetworkPreset = NetworkPreset {
        name: "mainnet",
        ..EXAMPLE_DEV
    };
    const OUTSIDE_THE_DATADIR: NetworkPreset = NetworkPreset {
        name: "../example",
        ..EXAMPLE_DEV
    };

    // Checked whichever network is selected, so a chain cannot ship a preset that
    // `--network` would resolve to something else.
    let err = parse_args(
        &chain(&[EXAMPLE_DEV, NAMED_LIKE_ETHEREUM]),
        ["ethrex-misnamed", "--network", "example-dev"],
    )
    .err()
    .expect("a preset named like a built-in network must be rejected");
    assert!(err.to_string().contains("reserved"), "{err}");

    let err = parse_args(&chain(&[OUTSIDE_THE_DATADIR]), ["ethrex-misnamed"])
        .err()
        .expect("a preset whose name is not a plain directory name must be rejected");
    assert!(err.to_string().contains("lowercase letters"), "{err}");

    let err = parse_args(&chain(&[EXAMPLE_DEV, EXAMPLE_DEV]), ["ethrex-misnamed"])
        .err()
        .expect("two presets with one name must be rejected");
    assert!(err.to_string().contains("more than once"), "{err}");
}

// ---- RPC namespace ----

async fn example_context() -> ethrex_rpc::test_utils::TestContext {
    let mut context = context_without_p2p(setup_store().await);
    context.extensions = RpcExtensions::new(vec![Arc::new(ExampleRpc)]).unwrap();
    context
}

async fn call(context: &RpcApiContext, method: &str) -> Result<Value, RpcErr> {
    map_http_requests(&RpcRequest::new(method, None), context.clone()).await
}

#[tokio::test]
async fn extension_namespace_is_served() {
    let context = example_context().await;
    assert_eq!(call(&context, "example_ping").await.unwrap(), json!("pong"));
}

#[tokio::test]
async fn extension_can_call_a_built_in_method() {
    let context = example_context().await;
    let built_in = call(&context, "eth_chainId").await.unwrap();
    assert_eq!(call(&context, "example_chainId").await.unwrap(), built_in);
}

#[tokio::test]
async fn extension_namespace_is_not_served_on_the_authenticated_port() {
    let context = example_context().await;
    let request = RpcRequest::new("example_ping", None);
    assert!(matches!(
        map_authrpc_requests(&request, context.clone()).await,
        Err(RpcErr::MethodNotFound(method)) if method == "example_ping"
    ));
}

#[tokio::test]
async fn extension_cannot_call_an_extension() {
    let context = example_context().await;
    // The call back into the dispatcher finds no extension, so it cannot recurse.
    assert!(matches!(
        call(&context, "example_pingItself").await,
        Err(RpcErr::MethodNotFound(method)) if method == "example_ping"
    ));
}

#[tokio::test]
async fn unknown_methods_are_not_found() {
    let context = example_context().await;
    // Unknown to the extension.
    assert!(matches!(
        call(&context, "example_missing").await,
        Err(RpcErr::MethodNotFound(method)) if method == "example_missing"
    ));
    // No such namespace.
    assert!(matches!(
        call(&context, "other_ping").await,
        Err(RpcErr::MethodNotFound(method)) if method == "other_ping"
    ));
    // The namespace alone is not a method of the namespace.
    assert!(matches!(
        call(&context, "example").await,
        Err(RpcErr::MethodNotFound(method)) if method == "example"
    ));
}

#[tokio::test]
async fn extension_namespace_is_not_found_without_the_extension() {
    let context = context_without_p2p(setup_store().await);
    assert!(matches!(
        call(&context, "example_ping").await,
        Err(RpcErr::MethodNotFound(_))
    ));
}

#[test]
fn extension_namespaces_are_validated() {
    struct Named(&'static str);
    impl RpcExtension for Named {
        fn namespace(&self) -> &'static str {
            self.0
        }
        fn handle<'a>(&'a self, req: &'a RpcRequest, _: RpcApiContext) -> RpcExtensionFuture<'a> {
            Box::pin(async move { Err(RpcErr::MethodNotFound(req.method.clone())) })
        }
    }
    let extensions = |names: &[&'static str]| -> Vec<Arc<dyn RpcExtension>> {
        names
            .iter()
            .map(|name| Arc::new(Named(name)) as Arc<dyn RpcExtension>)
            .collect()
    };

    assert!(RpcExtensions::new(extensions(&[])).is_ok());
    assert!(RpcExtensions::new(extensions(&["example", "other2"])).is_ok());
    for built_in in [
        "eth", "engine", "debug", "admin", "net", "web3", "txpool", "testing",
    ] {
        let err = RpcExtensions::new(extensions(&[built_in])).err().unwrap();
        assert!(err.contains("built-in"), "{err}");
    }
    let err = RpcExtensions::new(extensions(&["example", "example"]))
        .err()
        .unwrap();
    assert!(err.contains("more than once"), "{err}");
    for malformed in ["", "with_underscore", "Upper"] {
        let err = RpcExtensions::new(extensions(&[malformed])).err().unwrap();
        assert!(err.contains("lowercase letters and digits"), "{err}");
    }
}

// ---- mempool ----

async fn blockchain_with(mempool_policy: Option<Arc<dyn MempoolPolicy>>) -> Blockchain {
    let genesis: Genesis = serde_json::from_str(LOCAL_DEVNET_GENESIS_CONTENTS).unwrap();
    let mut store = Store::new("", EngineType::InMemory).unwrap();
    store.add_initial_state(genesis).await.unwrap();
    Blockchain::new(
        store,
        BlockchainOptions {
            mempool_policy,
            mempool_prewarm_enabled: false,
            ..Default::default()
        },
    )
}

fn transfer(gas_limit: u64) -> Transaction {
    Transaction::EIP1559Transaction(EIP1559Transaction {
        gas_limit,
        to: TxKind::Call(H160::repeat_byte(0xaa)),
        max_fee_per_gas: 10_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        ..Default::default()
    })
}

#[tokio::test]
async fn mempool_policy_rejects_before_the_standard_checks() {
    let blockchain = blockchain_with(Some(Arc::new(MaxTxGas(100_000)))).await;
    let result = blockchain
        .validate_transaction(&transfer(100_001), Address::repeat_byte(1))
        .await;
    match result {
        Err(MempoolError::RejectedByPolicy(reason)) => {
            assert_eq!(
                reason,
                "gas limit 100001 is above the example chain's cap of 100000"
            );
        }
        other => panic!("expected a policy rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn mempool_policy_sees_the_chain_head() {
    /// Rejects everything, naming the block it was shown.
    #[derive(Debug)]
    struct NameTheHead;
    impl MempoolPolicy for NameTheHead {
        fn admit(&self, _: &Transaction, _: Address, head: &BlockHeader) -> Result<(), String> {
            Err(format!(
                "head is block {} ({:#x})",
                head.number,
                head.hash()
            ))
        }
    }

    let blockchain = blockchain_with(Some(Arc::new(NameTheHead))).await;
    let genesis: Genesis = serde_json::from_str(LOCAL_DEVNET_GENESIS_CONTENTS).unwrap();
    let genesis_hash = genesis.get_block().hash();
    let result = blockchain
        .validate_transaction(&transfer(21_000), Address::repeat_byte(1))
        .await;
    match result {
        Err(MempoolError::RejectedByPolicy(reason)) => {
            assert_eq!(reason, format!("head is block 0 ({genesis_hash:#x})"));
        }
        other => panic!("expected a policy rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn mempool_policy_does_not_reject_what_it_admits() {
    let blockchain = blockchain_with(Some(Arc::new(MaxTxGas(100_000)))).await;
    let result = blockchain
        .validate_transaction(&transfer(100_000), Address::repeat_byte(1))
        .await;
    assert!(
        !matches!(result, Err(MempoolError::RejectedByPolicy(_))),
        "{result:?}"
    );
}

/// A transaction the policy rejects is skipped when a peer sends it. Handling the
/// message is not an error, and an error is what a peer is disconnected for.
#[tokio::test]
async fn mempool_policy_rejection_is_not_held_against_the_peer() {
    let genesis: Genesis = serde_json::from_str(LOCAL_DEVNET_GENESIS_CONTENTS).unwrap();
    let tx = signed_transfer_tx(100_001, 0, genesis.config.chain_id).await;
    let hash = tx.hash(&NativeCrypto);
    let Transaction::EIP1559Transaction(sent) = tx else {
        unreachable!("signed_transfer_tx builds a fee-market transaction")
    };
    let peer = Peer::new(Ipv4Addr::LOCALHOST.into(), 30303, 30303, H512::zero());
    let message = PooledTransactions::new(1, vec![P2PTransaction::EIP1559Transaction(sent)]);

    // Without a policy the transaction enters the pool: nothing else is wrong with it.
    let plain = blockchain_with(None).await;
    let handled = message.clone().handle(&peer, &plain, false).await;
    assert!(handled.is_ok(), "{handled:?}");
    assert!(plain.mempool.contains_tx(hash).unwrap());

    // With the policy it is left out, and the message is still handled without error.
    let strict = blockchain_with(Some(Arc::new(MaxTxGas(100_000)))).await;
    let handled = message.handle(&peer, &strict, false).await;
    assert!(handled.is_ok(), "{handled:?}");
    assert!(!strict.mempool.contains_tx(hash).unwrap());
}

#[tokio::test]
async fn no_policy_means_no_policy_rejection() {
    let blockchain = blockchain_with(None).await;
    let result = blockchain
        .validate_transaction(&transfer(100_001), Address::repeat_byte(1))
        .await;
    assert!(
        !matches!(result, Err(MempoolError::RejectedByPolicy(_))),
        "{result:?}"
    );
}

// ---- a running node ----

fn free_port() -> String {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string()
}

async fn rpc(port: &str, method: &str, params: Value) -> Value {
    reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}"))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// A signed transfer as `eth_sendRawTransaction` takes it.
async fn signed_transfer(gas_limit: u64, nonce: u64, chain_id: u64) -> String {
    let tx = signed_transfer_tx(gas_limit, nonce, chain_id).await;
    format!("0x{}", hex::encode(tx.encode_canonical_to_vec()))
}

async fn signed_transfer_tx(gas_limit: u64, nonce: u64, chain_id: u64) -> Transaction {
    let key = LOCAL_DEVNET_PRIVATE_KEYS.lines().next().unwrap();
    let key = SecretKey::from_slice(&hex::decode(key.trim_start_matches("0x")).unwrap()).unwrap();
    let signer: Signer = LocalSigner::new(key).into();
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        gas_limit,
        to: TxKind::Call(H160::repeat_byte(0xaa)),
        value: U256::one(),
        max_fee_per_gas: 10_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        ..Default::default()
    });
    tx.sign_inplace(&signer).await.unwrap();
    tx
}

/// A node started from the example chain's command line, in memory and without p2p.
struct Node {
    http_port: String,
    cancel_token: CancellationToken,
    store: Store,
    /// Holds the JWT secret for as long as the node runs.
    _dir: tempfile::TempDir,
}

impl Node {
    /// `with_components` chooses between the example chain's components and none.
    ///
    /// The ports are found by binding and releasing them, so another process can
    /// take one before the node binds it. The start is retried when that happens.
    async fn start(extra_args: &[&str], with_components: bool) -> Node {
        let dir = tempfile::tempdir().unwrap();
        let jwt = dir.path().join("jwt.hex");
        let mut last_error = None;
        for _ in 0..5 {
            let http_port = free_port();
            let authrpc_port = free_port();
            let mut argv = vec![
                "--network",
                "example-test",
                "--datadir",
                "memory",
                "--p2p.disabled",
                "--http.port",
                &http_port,
                "--authrpc.port",
                &authrpc_port,
                "--authrpc.jwtsecret",
                jwt.to_str().unwrap(),
            ];
            argv.extend(extra_args);
            let (cli, args) = parse(&EXAMPLE, &argv);
            let started = init_node(
                cli.opts,
                |genesis| {
                    if with_components {
                        (EXAMPLE.components)(&args, genesis)
                    } else {
                        Ok(NodeComponents::default())
                    }
                },
                None,
            )
            .await;
            match started {
                Ok((_, cancel_token, _, _, store)) => {
                    return Node {
                        http_port,
                        cancel_token,
                        store,
                        _dir: dir,
                    };
                }
                Err(err) if err.downcast_ref::<RpcStartupError>().is_some() => {
                    last_error = Some(err)
                }
                Err(err) => panic!("the node failed to start: {err}"),
            }
        }
        panic!("no free port after 5 attempts: {last_error:?}");
    }

    async fn stop(self) {
        self.cancel_token.cancel();
        self.store.shutdown().await.unwrap();
    }
}

/// `init_node` is public, so a preset can reach it without having gone through the
/// command line. It is still checked, before anything is created: its name is used
/// as a directory, and with p2p disabled its bootnodes would never be read.
#[tokio::test]
async fn node_refuses_a_preset_that_was_not_checked() {
    let unchecked = [
        (
            NetworkPreset {
                name: "../outside",
                ..EXAMPLE_TEST
            },
            "lowercase letters",
        ),
        (
            NetworkPreset {
                bootnodes: "not a list",
                ..EXAMPLE_TEST
            },
            "invalid bootnodes",
        ),
    ];
    for (preset, reason) in unchecked {
        let dir = tempfile::tempdir().unwrap();
        let datadir = dir.path().join("data");
        let (mut cli, _) = parse(
            &EXAMPLE,
            &["--datadir", datadir.to_str().unwrap(), "--p2p.disabled"],
        );
        cli.opts.network = Some(Network::Preset(preset));

        let err = init_node(cli.opts, |_| Ok(NodeComponents::default()), None)
            .await
            .err()
            .unwrap();

        assert!(err.to_string().contains(reason), "{err}");
        assert!(!datadir.exists(), "nothing is created under the datadir");
        assert!(!dir.path().join("outside").exists());
    }
}

/// Starts a node from the example chain's command line and checks, over HTTP, that
/// everything the chain returned from `components` is in effect.
#[tokio::test(flavor = "multi_thread")]
async fn node_runs_with_the_chain_components() {
    let node = Node::start(&["--example.max-tx-gas", "100000", "--ws.enabled"], true).await;
    let http_port = &node.http_port;
    let chain_id = node.store.get_chain_config().chain_id;
    // The node runs on the preset's genesis, not on a built-in network.
    let preset_genesis: Genesis = serde_json::from_str(EXAMPLE_TEST.genesis).unwrap();
    assert_eq!(chain_id, preset_genesis.config.chain_id);

    // RPC namespace.
    let pong = rpc(http_port, "example_ping", json!([])).await;
    assert_eq!(pong["result"], json!("pong"), "{pong}");
    let chain = rpc(http_port, "example_chainId", json!([])).await;
    assert_eq!(chain["result"], json!(format!("{chain_id:#x}")), "{chain}");

    // The same namespace over WebSocket, which shares the HTTP port here.
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{http_port}"))
        .await
        .unwrap();
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "example_ping", "params": []});
    socket.send(Message::text(ping.to_string())).await.unwrap();
    let pong = socket.next().await.unwrap().unwrap();
    let pong: Value = serde_json::from_str(pong.to_text().unwrap()).unwrap();
    assert_eq!(pong["result"], json!("pong"), "{pong}");

    // Mempool rule: above the cap the submitter gets the chain's reason.
    let above = signed_transfer(100_001, 0, chain_id).await;
    let rejected = rpc(http_port, "eth_sendRawTransaction", json!([above])).await;
    let message = rejected["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("gas limit 100001 is above the example chain's cap of 100000"),
        "{rejected}"
    );

    // At the cap the transaction enters the pool.
    let at_cap = signed_transfer(100_000, 0, chain_id).await;
    let accepted = rpc(http_port, "eth_sendRawTransaction", json!([at_cap])).await;
    assert!(accepted["result"].is_string(), "{accepted}");

    node.stop().await;
}

/// The same start without the chain's components: no extra namespace, no extra mempool
/// rule.
#[tokio::test(flavor = "multi_thread")]
async fn node_without_components_behaves_as_ethereum() {
    let node = Node::start(&[], false).await;
    let http_port = &node.http_port;
    let chain_id = node.store.get_chain_config().chain_id;

    let ping = rpc(http_port, "example_ping", json!([])).await;
    assert_eq!(ping["error"]["code"], json!(-32601), "{ping}");

    let above = signed_transfer(100_001, 0, chain_id).await;
    let accepted = rpc(http_port, "eth_sendRawTransaction", json!([above])).await;
    assert!(accepted["result"].is_string(), "{accepted}");

    node.stop().await;
}
