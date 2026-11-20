import unittest

from multisync.networks import NETWORK_CONFIG_DIR, NETWORKS
from multisync.util import fmt_time


class NetworksTest(unittest.TestCase):
    def test_every_network_has_mainnet_timing(self):
        for network in NETWORKS.values():
            self.assertEqual(network.epoch_seconds, 384, network.name)

    def test_only_plataberget_downloads_files(self):
        with_files = {n.name for n in NETWORKS.values() if n.setup_service() is not None}
        self.assertEqual(with_files, {"plataberget"})

    def test_plataberget_setup_service(self):
        network = NETWORKS["plataberget"]
        service = network.setup_service()
        script = service["command"][2]
        # `$$` is compose's escape for a literal `$`.
        self.assertIn("$$f", script)
        # Lighthouse v8.3.2 reads the standard names; older releases read these copies.
        self.assertIn("boot_enr.yaml", script)
        self.assertIn("deploy_block.txt", script)
        self.assertIn("genesis.json", script)
        self.assertEqual(service["volumes"], [f"config-plataberget:{NETWORK_CONFIG_DIR}"])
        self.assertEqual(network.setup_service_name, "setup-config-plataberget")
        self.assertEqual(network.chain_id, 7091047534)

    def test_checkpoint_providers(self):
        self.assertEqual(NETWORKS["mainnet"].checkpoint_url, "https://mainnet-checkpoint-sync.attestant.io")
        self.assertEqual(NETWORKS["hoodi"].checkpoint_url, "https://checkpoint-sync.hoodi.ethpandaops.io")


class FmtTimeTest(unittest.TestCase):
    def test_format(self):
        self.assertEqual(fmt_time(3725), "1h 2m 5s")
        self.assertEqual(fmt_time(42), "42s")


if __name__ == "__main__":
    unittest.main()
