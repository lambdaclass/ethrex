import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from multisync.config import ConfigError, check_windows, default_block_processing_duration, load, parse, select


def pair(**overrides):
    return {"network": "hoodi", "el": "ethrex", "cl": "lighthouse", **overrides}


class ParseTest(unittest.TestCase):
    def assertConfigError(self, data, fragment):
        with self.assertRaises(ConfigError) as ctx:
            parse(data)
        self.assertIn(fragment, str(ctx.exception))

    def test_defaults(self):
        [p] = parse({"pair": [pair()]}).pairs
        self.assertEqual((p.name, p.el_container, p.cl_container), ("hoodi", "ethrex-hoodi", "lighthouse-hoodi"))
        self.assertEqual((p.el_rpc_port, p.cl_api_port, p.el_metrics_port, p.cl_metrics_port), (8545, 5052, None, None))
        self.assertEqual((p.jwt_service, p.secrets_volume), ("setup-jwt-hoodi", "secrets-hoodi"))
        self.assertEqual(p.label, "hoodi: ethrex+lighthouse")
        self.assertTrue(p.enabled)

    def test_ports_follow_file_position(self):
        config = parse({"pair": [pair(), pair(name="hoodi-2", cl="ethlambda")]})
        self.assertEqual([p.el_rpc_port for p in config.pairs], [8545, 8546])
        self.assertEqual([p.cl_api_port for p in config.pairs], [5052, 5053])

    def test_rejections(self):
        self.assertConfigError({"pair": [pair(network="holesky")]}, "unknown network 'holesky'")
        self.assertConfigError({"pair": [pair(el="lighthouse")]}, "'lighthouse' is a cl client, not an el client")
        self.assertConfigError({"pair": [pair(cl="besu")]}, "unknown cl 'besu'")
        self.assertConfigError({"pair": [pair(), pair()]}, "pair name 'hoodi' is used twice")
        self.assertConfigError({"pair": [pair(name="Hoodi")]}, "must match [a-z0-9-]+")
        self.assertConfigError({"pair": [pair(el_rpc_port=True)]}, "'el_rpc_port' must be of type int")
        self.assertConfigError({"pair": [pair(sync_timeout="8h")]}, "'sync_timeout' must be of type int")
        self.assertConfigError({"pair": [pair(typo=1)]}, "unknown keys: typo")
        self.assertConfigError({"pair": [{"network": "hoodi", "el": "ethrex"}]}, "missing 'cl'")
        self.assertConfigError({"pair": [pair()], "client": {}}, "unknown top-level keys: client")
        self.assertConfigError({"pair": []}, "at least one [[pair]]")
        self.assertConfigError({"pair": [pair()], "clients": {"teku": {}}}, "clients.teku: unknown client")
        self.assertConfigError({"pair": [pair()], "clients": {"ethlambda": {"extra_args": [1]}}}, "list of strings")
        self.assertConfigError({"pair": [pair()], "clients": {"ethlambda": {"env": {"A": 1}}}}, "values must be strings")

    def test_names_must_match_fully(self):
        self.assertConfigError({"pair": [pair(name="hoodi\n")]}, "must match [a-z0-9-]+")

    def test_host_ports_are_in_range(self):
        for key in ("el_rpc_port", "cl_api_port", "el_metrics_port", "cl_metrics_port"):
            for bad in (0, -1, 65536):
                self.assertConfigError({"pair": [pair(**{key: bad})]}, f"pair 1: '{key}' must be between 1 and 65535")
        parse({"pair": [pair(el_rpc_port=65535, cl_api_port=1)]})

    def test_durations_are_positive_integers(self):
        for key in ("sync_timeout", "block_processing_duration"):
            for bad in (0, -5):
                self.assertConfigError({"pair": [pair(**{key: bad})]}, f"'{key}' must be a positive integer")
        self.assertConfigError({"pair": [pair(sync_timeout=True)]}, "'sync_timeout' must be of type int")

    def test_overrides(self):
        config = parse({
            "pair": [pair()],
            "clients": {"ethlambda": {"image": "x:y", "extra_args": ["--a"], "env": {"K": "V"}}},
        })
        override = config.override("ethlambda")
        self.assertEqual((override.image, override.extra_args, override.env), ("x:y", ["--a"], {"K": "V"}))
        self.assertIsNone(config.override("geth").image)


class SelectTest(unittest.TestCase):
    def setUp(self):
        self.config = parse({"pair": [pair(), pair(name="spare", enabled=False, cl="ethlambda")]})

    def test_enabled_pairs_by_default(self):
        self.assertEqual([p.name for p in select(self.config)], ["hoodi"])

    def test_named_pairs_run_even_when_disabled(self):
        self.assertEqual([p.name for p in select(self.config, "spare")], ["spare"])
        self.assertEqual([p.name for p in select(self.config, "spare, hoodi")], ["hoodi", "spare"])

    def test_filtering_keeps_ports(self):
        self.assertEqual(select(self.config, "spare")[0].el_rpc_port, 8546)

    def test_unknown_name(self):
        with self.assertRaisesRegex(ConfigError, "unknown pair names: nope"):
            select(self.config, "nope")

    def test_port_clash(self):
        config = parse({"pair": [pair(), pair(name="b", el_rpc_port=8545)]})
        with self.assertRaisesRegex(ConfigError, "host port 8545 is used by hoodi.el_rpc_port and by b.el_rpc_port"):
            select(config)

    def test_nothing_selected(self):
        with self.assertRaisesRegex(ConfigError, "no pairs selected"):
            select(parse({"pair": [pair(enabled=False)]}))


class WindowTest(unittest.TestCase):
    def test_window_must_cover_two_epochs(self):
        pairs = select(parse({"pair": [pair(block_processing_duration=600)]}))
        with self.assertRaisesRegex(ConfigError, "under two hoodi epochs"):
            check_windows(pairs, 1320)
        check_windows(select(parse({"pair": [pair()]})), 1320)

    def test_explicit_zero_is_not_the_default(self):
        # Bypass parse's positive check: check_windows must not treat 0 as unset.
        [p] = select(parse({"pair": [pair()]}))
        p.block_processing_duration = 0
        with self.assertRaisesRegex(ConfigError, "block_processing_duration 0s"):
            check_windows([p], 1320)

    def test_default_duration_reads_the_environment_at_call_time(self):
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop("BLOCK_PROCESSING_DURATION", None)
            self.assertEqual(default_block_processing_duration(), 22 * 60)
        with mock.patch.dict(os.environ, {"BLOCK_PROCESSING_DURATION": "60"}):
            self.assertEqual(default_block_processing_duration(), 60)

    def test_non_integer_duration_is_a_config_error(self):
        with mock.patch.dict(os.environ, {"BLOCK_PROCESSING_DURATION": "300 # note"}):
            with self.assertRaisesRegex(ConfigError, "BLOCK_PROCESSING_DURATION"):
                default_block_processing_duration()


class LoadTest(unittest.TestCase):
    def test_toml_errors_name_the_file(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "bad.toml"
            path.write_text("[[pair]\n")
            with self.assertRaisesRegex(ConfigError, "bad.toml"):
                load(path)

    def test_missing_file(self):
        with self.assertRaisesRegex(ConfigError, "file not found"):
            load("/nonexistent/multisync.toml")


if __name__ == "__main__":
    unittest.main()
