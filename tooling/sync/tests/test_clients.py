import unittest

from multisync.clients import CL_CLIENTS, EL_CLIENTS
from multisync.networks import NETWORKS

HOODI = NETWORKS["hoodi"]
PLATA = NETWORKS["plataberget"]
ENGINE = "http://ethrex-hoodi:8551"


class ElTemplateTest(unittest.TestCase):
    def test_ethrex_keeps_todays_command(self):
        self.assertEqual(EL_CLIENTS["ethrex"].command(HOODI, []), [
            "--http.addr", "0.0.0.0",
            "--http.api", "eth,net,web3,admin",
            "--network", "hoodi",
            "--authrpc.addr", "0.0.0.0",
            "--authrpc.jwtsecret", "/secrets/jwt.hex",
            "--syncmode", "snap",
            "--datadir", "/data",
            "--metrics", "--metrics.addr", "0.0.0.0", "--metrics.port", "3701",
        ])
        self.assertTrue(EL_CLIENTS["ethrex"].ethrex_extras)
        self.assertFalse(EL_CLIENTS["geth"].ethrex_extras)

    def test_geth_on_a_builtin_network(self):
        geth = EL_CLIENTS["geth"]
        self.assertEqual(geth.command(HOODI, [])[0], "--hoodi")
        self.assertIsNone(geth.entrypoint(HOODI))

    def test_geth_on_plataberget_inits_from_genesis(self):
        geth = EL_CLIENTS["geth"]
        self.assertTrue(geth.uses_network_files(PLATA))
        self.assertEqual(geth.entrypoint(PLATA), ["sh", "-c"])
        [script] = geth.command(PLATA, ["--verbosity", "4"])
        self.assertTrue(script.startswith("geth init --datadir /data /config/genesis.json && exec geth "))
        self.assertIn("--networkid 7091047534", script)
        self.assertIn("--http.vhosts '*'", script)
        self.assertIn("--verbosity 4", script)
        self.assertIn('--bootnodes "$$(paste -sd, /config/enodes.txt)"', script)

    def test_nethermind_uses_its_builtin_config_everywhere(self):
        nethermind = EL_CLIENTS["nethermind"]
        self.assertFalse(nethermind.uses_network_files(PLATA))
        command = nethermind.command(PLATA, [])
        self.assertEqual(command[:4], ["--config", "plataberget", "--data-dir", "/data"])
        self.assertIn("--JsonRpc.JwtSecretFile=/secrets/jwt.hex", command)


class ClTemplateTest(unittest.TestCase):
    def test_lighthouse(self):
        lighthouse = CL_CLIENTS["lighthouse"]
        command = lighthouse.command(HOODI, ENGINE, [])
        self.assertEqual(command[:6], ["lighthouse", "--datadir", "/data", "--network", "hoodi", "bn"])
        self.assertIn("--purge-db", command)
        self.assertEqual(command[command.index("--execution-endpoint") + 1], ENGINE)
        self.assertEqual(lighthouse.command(PLATA, ENGINE, [])[3:5], ["--testnet-dir", "/config"])

    def test_prysm(self):
        prysm = CL_CLIENTS["prysm"]
        command = prysm.command(HOODI, ENGINE, [])
        self.assertIn("--hoodi", command)
        self.assertIn("--http-host=0.0.0.0", command)
        self.assertEqual(prysm.api_port, 3500)
        self.assertIn("--genesis-state=/config/genesis.ssz", prysm.command(PLATA, ENGINE, []))

    def test_ethlambda(self):
        ethlambda = CL_CLIENTS["ethlambda"]
        self.assertEqual((ethlambda.stop_signal, ethlambda.stop_grace_period), ("SIGINT", "60s"))
        command = ethlambda.command(PLATA, ENGINE, ["--custody-group-count", "8"])
        self.assertEqual(command[:3], ["beacon", "--network", "plataberget"])
        self.assertEqual(command[command.index("--execution-endpoint") + 1], ENGINE)
        self.assertIn("--execution-jwt-secret", command)
        self.assertEqual(command[-2:], ["--custody-group-count", "8"])


class RosterTest(unittest.TestCase):
    def test_roster(self):
        self.assertEqual(set(EL_CLIENTS), {"ethrex", "geth", "nethermind"})
        self.assertEqual(set(CL_CLIENTS), {"lighthouse", "prysm", "ethlambda"})

    def test_every_client_can_run_every_network(self):
        for client in [*EL_CLIENTS.values(), *CL_CLIENTS.values()]:
            for network in NETWORKS.values():
                self.assertTrue(client.can_run(network), f"{client.name} on {network.name}")


if __name__ == "__main__":
    unittest.main()
