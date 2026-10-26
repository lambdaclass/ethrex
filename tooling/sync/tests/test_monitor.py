import unittest
from unittest import mock

import docker_monitor as dm
from multisync.config import parse, select


def make_instance(status="waiting"):
    config = parse({"pair": [{"network": "hoodi", "el": "ethrex", "cl": "ethlambda"}]})
    inst = dm.Instance(select(config)[0])
    inst.status = status
    return inst


def set_status(status, error=""):
    def fake_update_instance(inst, timeout_min):
        inst.status, inst.error = status, error
        return True
    return fake_update_instance


class InstanceTest(unittest.TestCase):
    def test_properties_come_from_the_pair(self):
        inst = make_instance()
        self.assertEqual((inst.name, inst.container, inst.cl_container), ("hoodi", "ethrex-hoodi", "ethlambda-hoodi"))
        self.assertEqual(inst.rpc_url, "http://localhost:8545")
        self.assertEqual(inst.cl_api.base_url, "http://localhost:5052")
        self.assertEqual(inst.provider_api.base_url, "https://checkpoint-sync.hoodi.ethpandaops.io")
        self.assertTrue(inst.ethrex_extras)
        self.assertEqual(inst.block_processing_duration, dm.BLOCK_PROCESSING_DURATION)


class UpdatePairTest(unittest.TestCase):
    def test_el_failures_are_prefixed(self):
        inst = make_instance("syncing")
        with mock.patch.object(dm, "update_instance", set_status("failed", "Sync timeout after 8h")):
            self.assertTrue(dm.update_pair(inst, 480))
        self.assertEqual(inst.error, "EL: Sync timeout after 8h")

    def test_cl_failure_fails_the_pair(self):
        inst = make_instance("syncing")

        def cl_fails(state, *args, **kwargs):
            state.status, state.error = "failed", "CL: head stuck at slot 9 for 10m"
            return True

        with mock.patch.object(dm, "update_instance", return_value=False), \
                mock.patch.object(dm.beacon_checks, "update", cl_fails):
            self.assertTrue(dm.update_pair(inst, 480))
        self.assertEqual((inst.status, inst.error), ("failed", "CL: head stuck at slot 9 for 10m"))

    def test_window_starts_with_block_processing(self):
        inst = make_instance("synced")
        inst.cl.status = "following"
        with mock.patch.object(dm, "update_instance", set_status("block_processing")), \
                mock.patch.object(dm.beacon_checks, "update", return_value=False):
            dm.update_pair(inst, 480)
        self.assertTrue(inst.cl.in_window)

    def test_window_waits_for_the_cl(self):
        inst = make_instance("synced")  # CL still "waiting"
        with mock.patch.object(dm, "update_instance", set_status("block_processing")), \
                mock.patch.object(dm.beacon_checks, "update", return_value=False):
            dm.update_pair(inst, 480)
        self.assertFalse(inst.cl.in_window)

    def test_end_checks_decide_success(self):
        for error, expected in ((None, "success"), ("CL: is_optimistic is true at the end of the window", "failed")):
            inst = make_instance("block_processing")
            with mock.patch.object(dm, "update_instance", set_status("success")), \
                    mock.patch.object(dm.beacon_checks, "update", return_value=False), \
                    mock.patch.object(dm.beacon_checks, "end_checks", return_value=error):
                dm.update_pair(inst, 480)
            self.assertEqual(inst.status, expected)


if __name__ == "__main__":
    unittest.main()
