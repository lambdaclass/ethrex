"""Networks a multisync pair can run on.

A `Network` holds what belongs to the chain rather than to any client: where
to checkpoint-sync from, its slot timing, and the files a client needs when
it has no built-in name for the network.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional

# Where a network's downloaded files are mounted in every container that reads them.
NETWORK_CONFIG_DIR = "/config"

PLATABERGET_METADATA_URL = (
    "https://raw.githubusercontent.com/ethpandaops/glamsterdam-devnets/"
    "master/network-configs/devnet-8/metadata"
)

# genesis.json and enodes.txt start an EL from a genesis file (geth); the
# rest start a CL from config files (Lighthouse, Prysm).
PLATABERGET_FILES = (
    "config.yaml",
    "genesis.ssz",
    "genesis.json",
    "bootstrap_nodes.yaml",
    "enodes.txt",
    "deposit_contract_block.txt",
    "genesis_validators_root.txt",
)


@dataclass(frozen=True)
class Network:
    name: str
    checkpoint_url: str
    seconds_per_slot: int = 12
    slots_per_epoch: int = 32
    # Set only for networks some client has no built-in name for.
    chain_id: Optional[int] = None
    metadata_url: Optional[str] = None
    metadata_files: tuple[str, ...] = ()

    @property
    def epoch_seconds(self) -> int:
        return self.seconds_per_slot * self.slots_per_epoch

    @property
    def config_volume(self) -> Optional[str]:
        return f"config-{self.name}" if self.metadata_url else None

    @property
    def setup_service_name(self) -> Optional[str]:
        return f"setup-config-{self.name}" if self.metadata_url else None

    def setup_service(self) -> Optional[dict]:
        """Compose service that downloads this network's files into its volume."""
        if not self.metadata_url:
            return None
        files = " ".join(self.metadata_files)
        # `$$` is compose's escape for a literal `$`. Lighthouse v8.3.2 reads the
        # standard bootstrap_nodes.yaml and deposit_contract_block.txt; the copies
        # under boot_enr.yaml and deploy_block.txt are for older releases, which
        # read those names. deploy_block.txt is the completion marker, the last
        # file written, so a partly filled volume fetches again.
        script = (
            "set -e; "
            f"if [ -f {NETWORK_CONFIG_DIR}/deploy_block.txt ]; then echo '{self.name} config present'; exit 0; fi; "
            "apk add --no-cache curl; "
            f"for f in {files}; do curl -fsSL -o {NETWORK_CONFIG_DIR}/$$f {self.metadata_url}/$$f; done; "
            f"cp {NETWORK_CONFIG_DIR}/bootstrap_nodes.yaml {NETWORK_CONFIG_DIR}/boot_enr.yaml; "
            f"cp {NETWORK_CONFIG_DIR}/deposit_contract_block.txt {NETWORK_CONFIG_DIR}/deploy_block.txt; "
            f"echo '{self.name} config downloaded'"
        )
        return {
            "image": "alpine",
            "volumes": [f"{self.config_volume}:{NETWORK_CONFIG_DIR}"],
            "command": ["sh", "-c", script],
        }


NETWORKS = {
    network.name: network
    for network in (
        Network("mainnet", "https://mainnet-checkpoint-sync.attestant.io"),
        Network("sepolia", "https://checkpoint-sync.sepolia.ethpandaops.io"),
        Network("hoodi", "https://checkpoint-sync.hoodi.ethpandaops.io"),
        # Platåberget, the public Glamsterdam testnet (glamsterdam-devnet-8 spec).
        Network(
            "plataberget",
            "https://checkpoint-sync.plataberget.ethpandaops.io",
            chain_id=7091047534,
            metadata_url=PLATABERGET_METADATA_URL,
            metadata_files=PLATABERGET_FILES,
        ),
    )
}
