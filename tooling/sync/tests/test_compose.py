import json
import tempfile
import unittest
from pathlib import Path

from multisync import compose
from multisync.config import load, parse, select

COMPLETED = {"condition": "service_completed_successfully"}


class BuildTest(unittest.TestCase):
    def setUp(self):
        self.config = parse({
            "pair": [
                {"network": "hoodi", "el": "ethrex", "cl": "ethlambda", "el_metrics_port": 3702},
                {"network": "plataberget", "el": "geth", "cl": "prysm"},
            ],
            "clients": {"ethlambda": {"image": "eth:dev", "env": {"RUST_LOG": "debug"}, "extra_args": ["--x"]}},
        })
        self.doc = compose.build(self.config, select(self.config))
        self.services = self.doc["services"]

    def test_service_names(self):
        self.assertEqual(sorted(self.services), [
            "ethlambda-hoodi", "ethrex-hoodi", "geth-plataberget", "prysm-plataberget",
            "setup-config-plataberget", "setup-jwt-hoodi", "setup-jwt-plataberget",
        ])

    def test_el_service(self):
        svc = self.services["ethrex-hoodi"]
        self.assertEqual(svc["container_name"], "ethrex-hoodi")
        self.assertEqual(svc["ports"], ["8545:8545", "3702:3701"])
        self.assertEqual(svc["volumes"], ["secrets-hoodi:/secrets", "ethrex-hoodi:/data"])
        self.assertEqual(svc["depends_on"], {"setup-jwt-hoodi": COMPLETED})
        self.assertEqual(svc["ulimits"], {"nofile": 1000000})
        self.assertEqual(svc["restart"], "unless-stopped")

    def test_cl_service_points_at_its_el(self):
        svc = self.services["ethlambda-hoodi"]
        command = svc["command"]
        self.assertEqual(command[command.index("--execution-endpoint") + 1], "http://ethrex-hoodi:8551")
        self.assertEqual(svc["ports"], ["5052:5052"])
        self.assertEqual((svc["stop_signal"], svc["stop_grace_period"]), ("SIGINT", "60s"))

    def test_client_override(self):
        svc = self.services["ethlambda-hoodi"]
        self.assertEqual((svc["image"], svc["pull_policy"]), ("eth:dev", "always"))
        self.assertEqual(svc["environment"], {"RUST_LOG": "debug"})
        self.assertEqual(svc["command"][-1], "--x")

    def test_network_files_are_mounted_where_needed(self):
        for name in ("geth-plataberget", "prysm-plataberget"):
            svc = self.services[name]
            self.assertIn("config-plataberget:/config:ro", svc["volumes"])
            self.assertEqual(svc["depends_on"]["setup-config-plataberget"], COMPLETED)
        self.assertEqual(self.services["geth-plataberget"]["entrypoint"], ["sh", "-c"])

    def test_every_named_volume_is_declared(self):
        declared = set(self.doc["volumes"])
        for svc in self.services.values():
            for volume in svc.get("volumes", []):
                self.assertIn(volume.split(":")[0], declared)

    def test_image_override_never_pulls(self):
        doc = compose.build(self.config, select(self.config), {"ethrex": "ethrex-local:multisync"})
        svc = doc["services"]["ethrex-hoodi"]
        self.assertEqual((svc["image"], svc["pull_policy"]), ("ethrex-local:multisync", "never"))


class CliTest(unittest.TestCase):
    def test_generate_writes_json_for_the_selection(self):
        with tempfile.TemporaryDirectory() as d:
            config_path = Path(d) / "m.toml"
            config_path.write_text(
                '[[pair]]\nnetwork = "hoodi"\nel = "ethrex"\ncl = "lighthouse"\n'
                '[[pair]]\nname = "spare"\nnetwork = "hoodi"\nel = "geth"\ncl = "ethlambda"\nenabled = false\n'
            )
            out = Path(d) / "c.json"
            self.assertEqual(compose.main(["generate", "--config", str(config_path), "--out", str(out)]), 0)
            self.assertEqual(sorted(json.loads(out.read_text())["services"]),
                             ["ethrex-hoodi", "lighthouse-hoodi", "setup-jwt-hoodi"])
            self.assertEqual(compose.main(["generate", "--config", str(config_path), "--out", str(out), "--only", "spare"]), 0)
            self.assertEqual(sorted(json.loads(out.read_text())["services"]),
                             ["ethlambda-spare", "geth-spare", "setup-jwt-spare"])
            self.assertEqual(compose.services(config_path, "spare"), ["geth-spare", "ethlambda-spare"])
            self.assertEqual(compose.services(config_path, "spare", "cl"), ["ethlambda-spare"])
            self.assertEqual(compose.main(["services", "--config", str(config_path), "--pair", "nope"]), 1)
            self.assertEqual(
                compose.main(["generate", "--config", str(config_path), "--out", str(out), "--image-override", "ethrex=local:x"]), 0
            )
            self.assertEqual(json.loads(out.read_text())["services"]["ethrex-hoodi"]["image"], "local:x")


SYNC_DIR = Path(__file__).resolve().parent.parent
COMMITTED = SYNC_DIR / "multisync.toml"


class CommittedConfigTest(unittest.TestCase):
    """The committed config reproduces the matrix the hand-written compose file had."""

    def setUp(self):
        self.config = load(COMMITTED)
        every_pair = ",".join(p.name for p in self.config.pairs)
        self.doc = compose.build(self.config, select(self.config, every_pair))
        self.services = self.doc["services"]

    def test_default_selection_is_todays(self):
        self.assertEqual([p.label for p in select(self.config)], [
            "hoodi: ethrex+lighthouse",
            "sepolia: ethrex+lighthouse",
            "mainnet: ethrex+lighthouse",
        ])

    def test_host_ports_are_todays(self):
        expected = {
            "ethrex-hoodi": ["8545:8545", "3702:3701"],
            "ethrex-sepolia": ["8546:8545", "3703:3701"],
            "ethrex-mainnet": ["8547:8545", "3701:3701"],
            "ethrex-hoodi-2": ["8548:8545"],
            "ethrex-plataberget": ["8549:8545"],
            "lighthouse-hoodi": ["5052:5052"],
            "lighthouse-sepolia": ["5053:5052"],
            "lighthouse-mainnet": ["5054:5052"],
            "lighthouse-hoodi-2": ["5055:5052"],
            "prysm-plataberget": ["5056:3500"],
        }
        for name, ports in expected.items():
            self.assertEqual(self.services[name]["ports"], ports, name)

    def test_ethrex_keeps_its_environment_overrides(self):
        svc = self.services["ethrex-mainnet"]
        self.assertEqual(svc["image"], "${ETHREX_IMAGE:-ghcr.io/lambdaclass/ethrex:main}")
        self.assertEqual(svc["pull_policy"], "${ETHREX_PULL_POLICY:-always}")
        self.assertEqual(svc["environment"], {
            "RUST_LOG": "${RUST_LOG:-info,ethrex_p2p::sync=debug}",
            "ETHREX_ROCKSDB_BLOCK_CACHE_SIZE": "${ETHREX_ROCKSDB_BLOCK_CACHE_SIZE:-4294967296}",
        })

    def test_checkpoint_providers_and_engine_urls_are_todays(self):
        providers = {
            "hoodi": "https://checkpoint-sync.hoodi.ethpandaops.io",
            "sepolia": "https://checkpoint-sync.sepolia.ethpandaops.io",
            "mainnet": "https://mainnet-checkpoint-sync.attestant.io",
        }
        for network, url in providers.items():
            command = self.services[f"lighthouse-{network}"]["command"]
            self.assertEqual(command[command.index("--checkpoint-sync-url") + 1], url)
            self.assertEqual(command[command.index("--execution-endpoint") + 1], f"http://ethrex-{network}:8551")

    def test_plataberget_prysm_reads_the_downloaded_files(self):
        self.assertIn("config-plataberget:/config:ro", self.services["prysm-plataberget"]["volumes"])
        self.assertIn("setup-config-plataberget", self.services)
        self.assertNotIn("config-plataberget:/config:ro", self.services["ethrex-plataberget"]["volumes"])


if __name__ == "__main__":
    unittest.main()
