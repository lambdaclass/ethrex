# polygon-package/main.star
#
# Minimal Polygon PoS devnet:
#   1. heimdall  — Heimdall node (REST :1317)
#   2. bor       — Bor validator mining blocks (JSON-RPC :8545, P2P :30303)
#   3. ethrex    — ethrex-polygon node syncing from Bor (JSON-RPC :8545)
#
# Usage:
#   kurtosis run --enclave polygon-devnet polygon-package \
#       --args-file fixtures/networks/polygon.yaml

DEFAULT_ARGS = {
    "heimdall_image": "0xpolygon/heimdall:latest",
    "bor_image":      "0xpolygon/bor:latest",
    "ethrex_image":   "ethrex:local",
    "bor_chain":      "mainnet",
    "bor_http_api":   "eth,net,web3,bor,admin",
    "ethrex_network": "polygon",
    "ethrex_log_level": "info",
}

# Pre-generated devnet validator address (empty password, keystore in static/).
DEVNET_VALIDATOR = "0x575d0bDDcFD26e4f457d175E3C3CD4Dfe4Fb598E"

HEIMDALL_REST_PORT = 1317
BOR_RPC_PORT  = 8545
BOR_P2P_PORT  = 30303
ETHREX_RPC_PORT     = 8545
ETHREX_P2P_PORT     = 30303
ETHREX_METRICS_PORT = 9090

def run(plan, args = {}):
    cfg = dict(DEFAULT_ARGS)
    cfg.update(args)

    heimdall = _start_heimdall(plan, cfg)
    bor = _start_bor(plan, cfg, heimdall)
    ethrex = _start_ethrex(plan, cfg, heimdall, bor)

    plan.print("Polygon devnet ready:")
    plan.print("  Heimdall REST: http://{}:{}".format(heimdall.ip_address, HEIMDALL_REST_PORT))
    plan.print("  Bor RPC:       http://{}:{}".format(bor.ip_address, BOR_RPC_PORT))
    plan.print("  ethrex RPC:    http://{}:{}".format(ethrex.ip_address, ETHREX_RPC_PORT))

    return struct(heimdall = heimdall, bor = bor, ethrex = ethrex)


def _start_heimdall(plan, cfg):
    return plan.add_service(
        name = "heimdall",
        config = ServiceConfig(
            image = cfg["heimdall_image"],
            cmd = ["start", "--home=/heimdall-data", "--rest-server"],
            ports = {
                "rest": PortSpec(number = HEIMDALL_REST_PORT, transport_protocol = "TCP"),
            },
            ready_conditions = ReadyCondition(
                recipe       = GetHttpRequestRecipe(port_id = "rest", endpoint = "/staking/validator-set"),
                field        = "code",
                assertion    = "==",
                target_value = 200,
                interval     = "5s",
                timeout      = "3m",
            ),
        ),
    )


def _start_bor(plan, cfg, heimdall):
    heimdall_url = "http://{}:{}".format(heimdall.ip_address, HEIMDALL_REST_PORT)

    # Upload the pre-generated keystore and password file.
    keystore = plan.upload_files(src = "./static/keystore", name = "bor-keystore")
    password = plan.upload_files(src = "./static/password.txt", name = "bor-password")

    # A fresh devnet Heimdall has no span data, so Bor must run with
    # --bor.withoutheimdall to produce blocks independently.
    return plan.add_service(
        name = "bor",
        config = ServiceConfig(
            image = cfg["bor_image"],
            cmd = [
                "server",
                "--datadir=/bor-data",
                "--chain={}".format(cfg["bor_chain"]),
                "--http",
                "--http.addr=0.0.0.0",
                "--http.port={}".format(BOR_RPC_PORT),
                "--http.api={}".format(cfg["bor_http_api"]),
                "--http.corsdomain=*",
                "--http.vhosts=*",
                "--mine",
                "--miner.etherbase={}".format(DEVNET_VALIDATOR),
                "--unlock={}".format(DEVNET_VALIDATOR),
                "--password=/secrets/password.txt",
                "--keystore=/keystore",
                "--allow-insecure-unlock",
                "--bor.withoutheimdall",
                "--bor.heimdall={}".format(heimdall_url),
            ],
            ports = {
                "rpc": PortSpec(number = BOR_RPC_PORT, transport_protocol = "TCP"),
                "p2p": PortSpec(number = BOR_P2P_PORT, transport_protocol = "TCP"),
            },
            files = {
                "/keystore": keystore,
                "/secrets":  password,
            },
            ready_conditions = ReadyCondition(
                recipe = PostHttpRequestRecipe(
                    port_id      = "rpc",
                    endpoint     = "/",
                    content_type = "application/json",
                    body         = '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}',
                ),
                field        = "code",
                assertion    = "==",
                target_value = 200,
                interval     = "5s",
                timeout      = "5m",
            ),
        ),
    )


def _start_ethrex(plan, cfg, heimdall, bor):
    heimdall_url = "http://{}:{}".format(heimdall.ip_address, HEIMDALL_REST_PORT)

    # Fetch Bor's enode URL via admin_nodeInfo RPC.
    bor_node_info = plan.request(
        service_name = "bor",
        recipe = PostHttpRequestRecipe(
            port_id      = "rpc",
            endpoint     = "/",
            content_type = "application/json",
            body         = '{"jsonrpc":"2.0","method":"admin_nodeInfo","params":[],"id":1}',
            extract      = {"enode": ".result.enode"},
        ),
    )

    bor_enode = bor_node_info["extract.enode"]

    cmd = [
        "--network={}".format(cfg["ethrex_network"]),
        "--bor.heimdall={}".format(heimdall_url),
        "--bootnodes={}".format(bor_enode),
        "--http.addr=0.0.0.0",
        "--http.port={}".format(ETHREX_RPC_PORT),
        "--p2p.port={}".format(ETHREX_P2P_PORT),
        "--log.level={}".format(cfg["ethrex_log_level"]),
        "--log.color=never",
        "--metrics",
        "--metrics.addr=0.0.0.0",
        "--metrics.port={}".format(ETHREX_METRICS_PORT),
    ]

    return plan.add_service(
        name = "ethrex",
        config = ServiceConfig(
            image = cfg["ethrex_image"],
            cmd   = cmd,
            ports = {
                "rpc":     PortSpec(number = ETHREX_RPC_PORT,     transport_protocol = "TCP"),
                "p2p":     PortSpec(number = ETHREX_P2P_PORT,     transport_protocol = "TCP"),
                "p2p-udp": PortSpec(number = ETHREX_P2P_PORT,     transport_protocol = "UDP"),
                "metrics": PortSpec(number = ETHREX_METRICS_PORT, transport_protocol = "TCP"),
            },
            env_vars = {"RUST_LOG": cfg["ethrex_log_level"]},
            ready_conditions = ReadyCondition(
                recipe = PostHttpRequestRecipe(
                    port_id      = "rpc",
                    endpoint     = "/",
                    content_type = "application/json",
                    body         = '{"jsonrpc":"2.0","method":"eth_syncing","params":[],"id":1}',
                ),
                field        = "code",
                assertion    = "==",
                target_value = 200,
                interval     = "5s",
                timeout      = "5m",
            ),
        ),
    )
