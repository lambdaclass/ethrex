"""How to start each client on a network.

One class per client. A template knows its image, its in-container ports and
its command line for a network; `compose.py` wraps that into a service. The
config file only picks clients and overrides image, extra args and env, and
each client runs the same image on every network.
"""

from __future__ import annotations

import shlex
from typing import Optional

from .networks import NETWORK_CONFIG_DIR, Network

DATA_DIR = "/data"
JWT_PATH = "/secrets/jwt.hex"

ALL_NETWORKS = frozenset({"mainnet", "sepolia", "hoodi", "plataberget"})
STANDARD_NETWORKS = frozenset({"mainnet", "sepolia", "hoodi"})


class Client:
    name: str = ""
    role: str = ""
    default_image: str = ""
    # A pinned version tag never changes, so pulling it on every start only adds
    # a registry dependency; clients on a moving tag override this.
    pull_policy: str = "missing"
    metrics_port: int = 0
    # Networks the client knows by name. It starts on any other network from
    # the files that network's setup service downloads.
    builtin_networks: frozenset[str] = frozenset()
    environment: dict[str, str] = {}
    ulimits_nofile: Optional[int] = None
    stop_signal: Optional[str] = None
    stop_grace_period: Optional[str] = None

    def uses_network_files(self, network: Network) -> bool:
        return network.name not in self.builtin_networks

    def can_run(self, network: Network) -> bool:
        return not self.uses_network_files(network) or network.metadata_url is not None

    def entrypoint(self, network: Network) -> Optional[list[str]]:
        return None


class ElClient(Client):
    role = "el"
    rpc_port = 8545
    engine_port = 8551
    # Monitor features that use ethrex-only RPCs and log lines: admin_*
    # diagnostics, phase timings, trie validation.
    ethrex_extras = False

    def flags(self, network: Network) -> list[str]:
        raise NotImplementedError

    def command(self, network: Network, extra_args: list[str]) -> list[str]:
        return [*self.flags(network), *extra_args]


class ClClient(Client):
    role = "cl"
    api_port = 5052

    def flags(self, network: Network, engine_url: str) -> list[str]:
        raise NotImplementedError

    def command(self, network: Network, engine_url: str, extra_args: list[str]) -> list[str]:
        return [*self.flags(network, engine_url), *extra_args]


class Ethrex(ElClient):
    name = "ethrex"
    # Compose interpolation keeps the ETHREX_IMAGE, ETHREX_PULL_POLICY, RUST_LOG
    # and ETHREX_ROCKSDB_BLOCK_CACHE_SIZE environment overrides working.
    default_image = "${ETHREX_IMAGE:-ghcr.io/lambdaclass/ethrex:main}"
    pull_policy = "${ETHREX_PULL_POLICY:-always}"
    metrics_port = 3701
    builtin_networks = ALL_NETWORKS
    environment = {
        "RUST_LOG": "${RUST_LOG:-info,ethrex_p2p::sync=debug}",
        # Bound RocksDB resident memory (default 12 GiB) so parallel snapsyncs fit in host RAM.
        "ETHREX_ROCKSDB_BLOCK_CACHE_SIZE": "${ETHREX_ROCKSDB_BLOCK_CACHE_SIZE:-4294967296}",  # 4 GiB
    }
    ulimits_nofile = 1_000_000
    ethrex_extras = True

    def flags(self, network: Network) -> list[str]:
        return [
            "--http.addr", "0.0.0.0",
            "--http.api", "eth,net,web3,admin",
            "--network", network.name,
            "--authrpc.addr", "0.0.0.0",
            "--authrpc.jwtsecret", JWT_PATH,
            "--syncmode", "snap",
            "--datadir", DATA_DIR,
            "--metrics", "--metrics.addr", "0.0.0.0", "--metrics.port", str(self.metrics_port),
        ]


class Geth(ElClient):
    name = "geth"
    default_image = "ethereum/client-go:v1.17.10"
    metrics_port = 6060
    builtin_networks = STANDARD_NETWORKS

    def flags(self, network: Network) -> list[str]:
        common = [
            "--syncmode", "snap",
            "--datadir", DATA_DIR,
            "--http", "--http.addr", "0.0.0.0", "--http.port", str(self.rpc_port),
            "--http.vhosts", "*", "--http.api", "eth,net,web3",
            "--authrpc.addr", "0.0.0.0", "--authrpc.port", str(self.engine_port),
            "--authrpc.vhosts", "*", "--authrpc.jwtsecret", JWT_PATH,
            "--metrics", "--metrics.addr", "0.0.0.0", "--metrics.port", str(self.metrics_port),
        ]
        if self.uses_network_files(network):
            return ["--networkid", str(network.chain_id), *common]
        return [f"--{network.name}", *common]

    def entrypoint(self, network: Network) -> Optional[list[str]]:
        return ["sh", "-c"] if self.uses_network_files(network) else None

    def command(self, network: Network, extra_args: list[str]) -> list[str]:
        if not self.uses_network_files(network):
            return super().command(network, extra_args)
        # No built-in name: initialise the datadir from the network's genesis,
        # then start with its bootnodes. Re-running init on the same genesis is
        # a no-op, so a container restart is safe. `$$` is compose's `$`.
        start = shlex.join(["geth", *self.flags(network), *extra_args])
        return [
            f"geth init --datadir {DATA_DIR} {NETWORK_CONFIG_DIR}/genesis.json && "
            f'exec {start} --bootnodes "$$(paste -sd, {NETWORK_CONFIG_DIR}/enodes.txt)"'
        ]


class Nethermind(ElClient):
    name = "nethermind"
    default_image = "nethermind/nethermind:2.2.2"
    metrics_port = 6060
    builtin_networks = ALL_NETWORKS

    def flags(self, network: Network) -> list[str]:
        return [
            "--config", network.name,
            "--data-dir", DATA_DIR,
            "--JsonRpc.Enabled=true",
            "--JsonRpc.Host=0.0.0.0",
            f"--JsonRpc.Port={self.rpc_port}",
            "--JsonRpc.EngineHost=0.0.0.0",
            f"--JsonRpc.EnginePort={self.engine_port}",
            f"--JsonRpc.JwtSecretFile={JWT_PATH}",
            "--Sync.SnapSync=true",
            "--Metrics.Enabled=true",
            f"--Metrics.ExposePort={self.metrics_port}",
        ]


class Lighthouse(ClClient):
    name = "lighthouse"
    default_image = "sigp/lighthouse:v8.3.2"
    api_port = 5052
    metrics_port = 5054
    builtin_networks = STANDARD_NETWORKS

    def flags(self, network: Network, engine_url: str) -> list[str]:
        if self.uses_network_files(network):
            # v8.3.2 reads config.yaml, genesis.ssz, bootstrap_nodes.yaml and
            # deposit_contract_block.txt; older releases read boot_enr.yaml and
            # deploy_block.txt, which the setup service also writes.
            chain = ["--testnet-dir", NETWORK_CONFIG_DIR]
        else:
            chain = ["--network", network.name]
        return [
            "lighthouse", "--datadir", DATA_DIR, *chain, "bn",
            "--http", "--http-address", "0.0.0.0", "--http-port", str(self.api_port),
            "--execution-endpoint", engine_url,
            "--execution-jwt", JWT_PATH,
            "--checkpoint-sync-url", network.checkpoint_url,
            "--checkpoint-sync-url-timeout", "600",
            "--metrics", "--metrics-address", "0.0.0.0", "--metrics-port", str(self.metrics_port),
            "--purge-db",
        ]


class Prysm(ClClient):
    name = "prysm"
    default_image = "gcr.io/offchainlabs/prysm/beacon-chain:v7.3.1"
    api_port = 3500
    metrics_port = 8080
    builtin_networks = STANDARD_NETWORKS

    def flags(self, network: Network, engine_url: str) -> list[str]:
        if self.uses_network_files(network):
            chain = [
                f"--chain-config-file={NETWORK_CONFIG_DIR}/config.yaml",
                f"--genesis-state={NETWORK_CONFIG_DIR}/genesis.ssz",
                f"--bootstrap-node={NETWORK_CONFIG_DIR}/bootstrap_nodes.yaml",
            ]
        else:
            chain = [f"--{network.name}"]
        return [
            "--accept-terms-of-use",
            f"--datadir={DATA_DIR}",
            *chain,
            f"--checkpoint-sync-url={network.checkpoint_url}",
            f"--genesis-beacon-api-url={network.checkpoint_url}",
            f"--execution-endpoint={engine_url}",
            f"--jwt-secret={JWT_PATH}",
            "--http-host=0.0.0.0",
            f"--http-port={self.api_port}",
            "--monitoring-host=0.0.0.0",
            f"--monitoring-port={self.metrics_port}",
        ]


class Ethlambda(ClClient):
    name = "ethlambda"
    default_image = "ghcr.io/lambdaclass/ethlambda:beacon"
    pull_policy = "always"  # `:beacon` is a moving tag
    api_port = 5052
    metrics_port = 5054
    builtin_networks = ALL_NETWORKS
    # ethlambda shuts down gracefully on SIGINT only, and as PID 1 it ignores
    # SIGTERM, so a plain `docker stop` would wait out the grace period.
    stop_signal = "SIGINT"
    stop_grace_period = "60s"

    def flags(self, network: Network, engine_url: str) -> list[str]:
        return [
            "beacon",
            "--network", network.name,
            "--checkpoint-sync-url", network.checkpoint_url,
            "--execution-endpoint", engine_url,
            "--execution-jwt-secret", JWT_PATH,
            "--http-address", "0.0.0.0",
            "--api-port", str(self.api_port),
            "--metrics-port", str(self.metrics_port),
            "--data-dir", DATA_DIR,
        ]


EL_CLIENTS: dict[str, ElClient] = {client.name: client for client in (Ethrex(), Geth(), Nethermind())}
CL_CLIENTS: dict[str, ClClient] = {client.name: client for client in (Lighthouse(), Prysm(), Ethlambda())}
