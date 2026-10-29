import argparse
import json
import os
import subprocess
import tempfile
import time
import unittest
from datetime import datetime, timezone
from pathlib import Path
from unittest import mock

import docker_monitor as dm
from multisync.config import ConfigError, default_block_processing_duration, parse, select


def make_instance(status="waiting", **pair_fields):
    config = parse({"pair": [{"network": "hoodi", "el": "ethrex", "cl": "ethlambda", **pair_fields}]})
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
        self.assertEqual(inst.cl_api.base_url, "http://127.0.0.1:5052")
        self.assertEqual(inst.provider_api.base_url, "https://checkpoint-sync.hoodi.ethpandaops.io")
        self.assertTrue(inst.ethrex_extras)
        self.assertEqual(inst.block_processing_duration, default_block_processing_duration())


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


class WindowAlignmentTest(unittest.TestCase):
    def test_cl_window_restarts_the_el_window(self):
        inst = make_instance("block_processing")
        inst.cl.status = "following"
        inst.block_check_start, inst.initial_block, inst.last_block = 1.0, 100, 150
        with mock.patch.object(dm, "update_instance", return_value=False), \
                mock.patch.object(dm.beacon_checks, "update", return_value=False), \
                mock.patch.object(dm.time, "time", return_value=5000.0):
            dm.update_pair(inst, 480)
        self.assertTrue(inst.cl.in_window)
        self.assertEqual((inst.block_check_start, inst.initial_block), (5000.0, 150))

    def test_window_is_not_restarted_again(self):
        inst = make_instance("block_processing")
        inst.cl.status, inst.cl.in_window = "following", True
        inst.block_check_start, inst.initial_block, inst.last_block = 1.0, 100, 150
        with mock.patch.object(dm, "update_instance", return_value=False), \
                mock.patch.object(dm.beacon_checks, "update", return_value=False):
            dm.update_pair(inst, 480)
        self.assertEqual((inst.block_check_start, inst.initial_block), (1.0, 100))


class EndOfRunTest(unittest.TestCase):
    def test_end_checks_run_once(self):
        inst = make_instance("block_processing")
        with mock.patch.object(dm, "update_instance", set_status("success")), \
                mock.patch.object(dm.beacon_checks, "update", return_value=False), \
                mock.patch.object(dm.beacon_checks, "end_checks", return_value=None) as end_checks:
            dm.update_pair(inst, 480)
            self.assertFalse(dm.update_pair(inst, 480))
        end_checks.assert_called_once()
        self.assertEqual(inst.status, "success")

    def test_el_success_with_a_waiting_cl_fails_the_pair(self):
        inst = make_instance("block_processing")  # CL never left "waiting"
        with mock.patch.object(dm, "update_instance", set_status("success")), \
                mock.patch.object(dm.beacon_checks, "update", return_value=False):
            dm.update_pair(inst, 480)
        self.assertEqual(inst.status, "failed")
        self.assertTrue(inst.error.startswith("CL: "), inst.error)


def rpc(answers):
    """A fake `rpc_call` answering from a method -> result map."""
    return lambda url, method, params=None: answers.get(method)


class UpdateInstanceTest(unittest.TestCase):
    def test_own_sync_timeout_is_honored(self):
        inst = make_instance("syncing", sync_timeout=10)
        inst.start_time = 1000.0
        answers = {"eth_blockNumber": "0x5", "eth_syncing": {"x": 1}}
        with mock.patch.object(dm, "rpc_call", rpc(answers)), \
                mock.patch.object(dm, "check_validation_progress", return_value=None), \
                mock.patch.object(dm.time, "time", return_value=1000.0 + 10 * 60 + 1):
            dm.update_instance(inst, 480)
        self.assertEqual(inst.status, "failed")
        self.assertIn(dm.fmt_time(10 * 60), inst.error)

    def test_own_block_processing_duration_is_honored(self):
        inst = make_instance("block_processing", block_processing_duration=1000)
        inst.block_check_start = inst.last_block_time = 5000.0
        inst.initial_block = inst.last_block = 10
        answers = {"eth_blockNumber": "0xb"}
        with mock.patch.object(dm, "rpc_call", rpc(answers)), mock.patch.object(dm, "BLOCK_STALL_TIMEOUT", 10**6):
            with mock.patch.object(dm.time, "time", return_value=5000.0 + 999):
                dm.update_instance(inst, 480)
            self.assertEqual(inst.status, "block_processing")
            with mock.patch.object(dm.time, "time", return_value=5000.0 + 1001):
                dm.update_instance(inst, 480)
        self.assertEqual(inst.status, "success")

    def test_non_ethrex_pair_never_reads_ethrex_logs(self):
        inst = make_instance("syncing", el="geth", cl="lighthouse")
        inst.start_time = time_now = 1000.0
        answers = {"eth_blockNumber": "0x7", "eth_syncing": False}
        with mock.patch.object(dm, "rpc_call", rpc(answers)), \
                mock.patch.object(dm, "check_validation_progress") as progress, \
                mock.patch.object(dm, "check_validation_failure") as failure, \
                mock.patch.object(dm.time, "time", return_value=time_now + 5):
            dm.update_instance(inst, 480)
        progress.assert_not_called()
        failure.assert_not_called()
        self.assertEqual(inst.status, "synced")

    def test_synced_needs_a_block_above_zero(self):
        inst = make_instance("syncing")
        inst.start_time = 1000.0
        with mock.patch.object(dm, "rpc_call", rpc({"eth_blockNumber": "0x0", "eth_syncing": False})), \
                mock.patch.object(dm, "check_validation_progress", return_value=None), \
                mock.patch.object(dm.time, "time", return_value=1005.0):
            dm.update_instance(inst, 480)
        self.assertEqual(inst.status, "syncing")
        with mock.patch.object(dm, "rpc_call", rpc({"eth_blockNumber": "0x1", "eth_syncing": False})), \
                mock.patch.object(dm, "check_validation_progress", return_value=None), \
                mock.patch.object(dm.time, "time", return_value=1010.0):
            dm.update_instance(inst, 480)
        self.assertEqual(inst.status, "synced")

    def test_el_that_never_answers_fails_after_the_startup_timeout(self):
        inst = make_instance("waiting")
        inst.created_at = 1000.0
        with mock.patch.object(dm, "rpc_call", return_value=None), \
                mock.patch.object(dm, "container_exit_info", return_value=(False, 3)), \
                mock.patch.object(dm.time, "time", return_value=1000.0 + dm.EL_STARTUP_TIMEOUT - 1):
            self.assertFalse(dm.update_instance(inst, 480))
        self.assertEqual(inst.status, "waiting")
        with mock.patch.object(dm, "rpc_call", return_value=None), \
                mock.patch.object(dm, "container_exit_info", return_value=(False, 3)), \
                mock.patch.object(dm.time, "time", return_value=1000.0 + dm.EL_STARTUP_TIMEOUT + 1):
            self.assertTrue(dm.update_instance(inst, 480))
        self.assertEqual(inst.status, "failed")
        self.assertEqual(inst.error, f"no RPC answer after {dm.fmt_time(dm.EL_STARTUP_TIMEOUT)}, container exited with code 3")

    def test_startup_timeout_error_is_prefixed_once(self):
        inst = make_instance("waiting")
        inst.created_at = 0.0
        with mock.patch.object(dm, "rpc_call", return_value=None), \
                mock.patch.object(dm, "container_exit_info", return_value=(True, None)):
            dm.update_pair(inst, 480)
        self.assertEqual(inst.error, f"EL: no RPC answer after {dm.fmt_time(dm.EL_STARTUP_TIMEOUT)}")


class ReexecTest(unittest.TestCase):
    def test_moved_head_re_executes(self):
        with mock.patch.object(dm.os, "execv") as execv:
            dm.reexec_if_updated("aaaa", "bbbb")
        execv.assert_called_once_with(dm.sys.executable, [dm.sys.executable, *dm.sys.argv])

    def test_unchanged_head_does_not(self):
        with mock.patch.object(dm.os, "execv") as execv:
            dm.reexec_if_updated("aaaa", "aaaa")
        execv.assert_not_called()

    def test_unknown_start_commit_does_not_loop(self):
        with mock.patch.object(dm.os, "execv") as execv:
            dm.reexec_if_updated("unknown", "bbbb")
        execv.assert_not_called()


class GitCommitTest(unittest.TestCase):
    def test_full_hash_from_the_given_directory(self):
        with mock.patch.object(dm.subprocess, "check_output", return_value=b"abc123\n") as out:
            self.assertEqual(dm.git_commit("/other"), "abc123")
        self.assertEqual(out.call_args.args[0], ["git", "rev-parse", "HEAD"])
        self.assertEqual(out.call_args.kwargs["cwd"], "/other")

    def test_pull_returns_the_full_hash(self):
        with mock.patch.object(dm.subprocess, "run"), \
                mock.patch.object(dm.subprocess, "check_output", return_value=b"f" * 40 + b"\n") as out:
            self.assertEqual(dm.git_pull_latest("main", "/other"), (True, "f" * 40))
        self.assertEqual(out.call_args.args[0], ["git", "rev-parse", "HEAD"])


class ContainerStartTimeTest(unittest.TestCase):
    def test_started_at_is_utc_whatever_the_local_zone(self):
        stamp = b"2026-10-26T20:52:52.160113046Z\n"
        expected = datetime(2026, 10, 26, 20, 52, 52, 160113, tzinfo=timezone.utc).timestamp()
        old_tz = os.environ.get("TZ")
        os.environ["TZ"] = "America/Argentina/Buenos_Aires"
        time.tzset()
        try:
            with mock.patch.object(dm.subprocess, "check_output", return_value=stamp):
                self.assertEqual(dm.container_start_time("c"), expected)
        finally:
            if old_tz is None:
                os.environ.pop("TZ", None)
            else:
                os.environ["TZ"] = old_tz
            time.tzset()


ETHLAMBDA_REV = "a11df91fee172274ddc1c5cf2aed8e35d86e8476"


def inspect_output(image, image_id, labels=None):
    return json.dumps([{"Image": image_id, "Config": {"Image": image, "Labels": labels}}]).encode()


class ClientBuildTest(unittest.TestCase):
    labeled = inspect_output("ethlambda:local", "sha256:59ed6d5030a5f927a0ef", {
        dm.OCI_REVISION: ETHLAMBDA_REV,
        dm.OCI_SOURCE: "https://github.com/lambdaclass/ethlambda",
    })

    def test_a_labeled_image_names_its_commit_linked_to_its_own_repo(self):
        with mock.patch.object(dm.subprocess, "check_output", return_value=self.labeled):
            build = dm.container_build("ethlambda", "ethlambda-hoodi")
        self.assertEqual(build.describe(slack=False), "ethlambda: ethlambda:local, commit a11df91")
        self.assertEqual(
            build.describe(slack=True),
            f"ethlambda: `ethlambda:local`, commit <https://github.com/lambdaclass/ethlambda/commit/{ETHLAMBDA_REV}|a11df91>",
        )

    def test_an_unlabeled_image_falls_back_to_its_image_id(self):
        out = inspect_output("sigp/lighthouse:v8.3.2", "sha256:5b3e1b7c9a2d4f00aa11", {"org.opencontainers.image.version": "22.04"})
        with mock.patch.object(dm.subprocess, "check_output", return_value=out):
            build = dm.container_build("lighthouse", "lighthouse-hoodi")
        self.assertEqual(build.describe(slack=False), "lighthouse: sigp/lighthouse:v8.3.2, image 5b3e1b7c9a2d")

    def test_a_placeholder_revision_is_not_a_commit(self):
        out = inspect_output("ethlambda:dev", "sha256:0123456789abcdef", {dm.OCI_REVISION: "unknown"})
        with mock.patch.object(dm.subprocess, "check_output", return_value=out):
            build = dm.container_build("ethlambda", "ethlambda-hoodi")
        self.assertEqual(build.describe(slack=False), "ethlambda: ethlambda:dev, image 0123456789ab")

    def test_a_container_docker_cannot_inspect_has_no_image(self):
        with mock.patch.object(dm.subprocess, "check_output", side_effect=subprocess.CalledProcessError(1, "docker")):
            build = dm.container_build("geth", "geth-hoodi")
        self.assertEqual(build.describe(slack=True), "geth: image unknown")

    def test_a_client_shared_by_pairs_is_listed_once(self):
        config = parse({"pair": [
            {"network": "hoodi", "el": "ethrex", "cl": "ethlambda"},
            {"network": "sepolia", "el": "ethrex", "cl": "lighthouse"},
        ]})
        instances = [dm.Instance(p) for p in select(config)]
        outputs = {
            "ethrex-hoodi": inspect_output("ghcr.io/lambdaclass/ethrex:main", "sha256:df03c6c1df97"),
            "ethrex-sepolia": inspect_output("ghcr.io/lambdaclass/ethrex:main", "sha256:df03c6c1df97"),
            "ethlambda-hoodi": self.labeled,
            "lighthouse-sepolia": inspect_output("sigp/lighthouse:v8.3.2", "sha256:5b3e1b7c9a2d"),
        }
        with mock.patch.object(dm.subprocess, "check_output", side_effect=lambda cmd, **_: outputs[cmd[-1]]):
            builds = dm.client_builds(instances)
        self.assertEqual([b.client for b in builds], ["ethrex", "ethlambda", "lighthouse"])

    def test_the_slack_summary_lists_the_clients(self):
        build = dm.ClientBuild("ethlambda", "ethlambda:local", revision=ETHLAMBDA_REV)
        with mock.patch.dict(dm.os.environ, {"SLACK_WEBHOOK_URL_SUCCESS": "http://hook"}), \
                mock.patch.object(dm, "client_builds", return_value=[build]), \
                mock.patch.object(dm.requests, "post") as post:
            dm.slack_notify("20261027_112140", 4, [make_instance("success")], "host", "main", "7a15c34")
        summary = post.call_args.kwargs["json"]["blocks"][1]["text"]["text"]
        self.assertIn("*Clients:*\n• ethlambda: `ethlambda:local`, commit a11df91\n", summary)


def restart_args(directory, **overrides):
    values = dict(config="multisync.toml", networks="", compose_dir=directory,
                  compose_file="compose.json", no_slack=False, branch="main")
    values.update(overrides)
    return argparse.Namespace(**values)


class RestartTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.compose = Path(self.dir.name) / "compose.json"
        self.pairs = [object()]
        patches = [
            mock.patch.object(dm.time, "sleep"),
            mock.patch.object(dm.multisync_compose, "prepare", return_value=(self.pairs, {"services": {}})),
            mock.patch.object(dm, "slack_text"),
        ]
        self.sleep, self.prepare, self.slack = [p.start() for p in patches]
        for p in patches:
            self.addCleanup(p.stop)

    def verbs(self, run):
        return [c.args[0][c.args[0].index("-f") + 2:] for c in run.call_args_list]

    def test_bad_config_touches_nothing(self):
        self.compose.write_text("{}")
        self.prepare.side_effect = ConfigError("pair renamed")
        with mock.patch.object(dm.subprocess, "run") as run:
            self.assertIsNone(dm.restart_containers(restart_args(self.dir.name), {}))
        run.assert_not_called()
        self.assertEqual(self.compose.read_text(), "{}")
        self.slack.assert_called_once()
        self.assertIn("pair renamed", self.slack.call_args.args[0])

    def test_no_slack_flag_silences_the_failure_message(self):
        self.prepare.side_effect = ConfigError("bad")
        with mock.patch.object(dm.subprocess, "run"):
            dm.restart_containers(restart_args(self.dir.name, no_slack=True), {})
        self.slack.assert_not_called()

    def test_existing_file_is_taken_down_then_written_then_started(self):
        self.compose.write_text("old")
        with mock.patch.object(dm.subprocess, "run") as run:
            result = dm.restart_containers(restart_args(self.dir.name), {})
        self.assertIs(result, self.pairs)
        self.assertEqual(self.verbs(run), [["down", "-v", "--remove-orphans"], ["up", "-d", "--remove-orphans"]])
        self.assertEqual(self.compose.read_text().strip(), '{\n  "services": {}\n}')

    def test_first_run_also_cleans_with_the_new_file(self):
        with mock.patch.object(dm.subprocess, "run") as run:
            result = dm.restart_containers(restart_args(self.dir.name), {})
        self.assertIs(result, self.pairs)
        self.assertEqual(self.verbs(run), [["down", "-v", "--remove-orphans"], ["up", "-d", "--remove-orphans"]])
        self.assertTrue(self.compose.exists())

    def test_docker_failure_returns_none_and_reports(self):
        with mock.patch.object(dm.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "docker")):
            self.assertIsNone(dm.restart_containers(restart_args(self.dir.name), {}))
        self.slack.assert_called_once()


class AutoUpdateRetryTest(unittest.TestCase):
    def run_main(self, argv, restart_result):
        patches = {
            "load_instances": mock.patch.object(dm, "load_instances", return_value=[make_instance()]),
            "container_start_time": mock.patch.object(dm, "container_start_time", return_value=None),
            "git_commit": mock.patch.object(dm, "git_commit", return_value="a" * 40),
            "git_branch": mock.patch.object(dm, "git_branch", return_value="main"),
            "ensure_logs_dir": mock.patch.object(dm, "ensure_logs_dir"),
            "get_next_run_count": mock.patch.object(dm, "get_next_run_count", return_value=1),
            "build": mock.patch.object(dm, "build_docker_image", return_value=True),
            "restart": mock.patch.object(dm, "restart_containers", return_value=restart_result),
            "sleep": mock.patch.object(dm.time, "sleep"),
            "reexec": mock.patch.object(dm, "reexec_if_updated"),
            "pull": mock.patch.object(dm, "git_pull_latest", side_effect=[(True, "b" * 40), (False, "")]),
            "argv": mock.patch.object(dm.sys, "argv", ["docker_monitor.py", "--no-slack", *argv]),
        }
        started = {k: v.start() for k, v in patches.items()}
        for v in patches.values():
            self.addCleanup(v.stop)
        with self.assertRaises(SystemExit) as exit_info:
            dm.main()
        return started, exit_info.exception.code

    def test_auto_mode_waits_and_pulls_again_after_a_failed_restart(self):
        m, code = self.run_main(["--auto-update"], None)
        self.assertEqual(code, 1)
        self.assertEqual(m["pull"].call_count, 2)
        self.assertEqual(m["restart"].call_count, 1)
        m["sleep"].assert_any_call(dm.AUTO_UPDATE_RETRY_DELAY)


class SlackTextTest(unittest.TestCase):
    def test_posts_text_to_the_failed_webhook(self):
        with mock.patch.dict(dm.os.environ, {"SLACK_WEBHOOK_URL_FAILED": "http://hook"}), \
                mock.patch.object(dm.requests, "post") as post:
            dm.slack_text("hello")
        self.assertEqual(post.call_args.args[0], "http://hook")
        self.assertEqual(post.call_args.kwargs["json"], {"text": "hello"})

    def test_does_nothing_without_a_webhook(self):
        with mock.patch.dict(dm.os.environ, {}, clear=True), mock.patch.object(dm.requests, "post") as post:
            dm.slack_text("hello")
        post.assert_not_called()


class LoadInstancesTest(unittest.TestCase):
    def test_config_error_is_sent_to_slack_before_exiting(self):
        args = argparse.Namespace(config="x.toml", networks="", no_slack=False, branch="feat")
        with mock.patch.object(dm.multisync_compose, "prepare", side_effect=ConfigError("bad pair")), \
                mock.patch.object(dm, "slack_text") as slack:
            with self.assertRaises(SystemExit):
                dm.load_instances(args)
        text = slack.call_args.args[0]
        self.assertIn("bad pair", text)
        self.assertIn("feat", text)

    def test_no_slack_flag_silences_the_config_error(self):
        args = argparse.Namespace(config="x.toml", networks="", no_slack=True, branch="feat")
        with mock.patch.object(dm.multisync_compose, "prepare", side_effect=ConfigError("bad")), \
                mock.patch.object(dm, "slack_text") as slack:
            with self.assertRaises(SystemExit):
                dm.load_instances(args)
        slack.assert_not_called()

    def test_uses_prepare_and_exits_on_config_error(self):
        args = argparse.Namespace(config="x.toml", networks="", no_slack=True, branch="")
        with mock.patch.object(dm.multisync_compose, "prepare", side_effect=ConfigError("bad")):
            with self.assertRaises(SystemExit):
                dm.load_instances(args)
        config = parse({"pair": [{"network": "hoodi", "el": "ethrex", "cl": "ethlambda"}]})
        with mock.patch.object(dm.multisync_compose, "prepare", return_value=(select(config), {})):
            self.assertEqual([i.name for i in dm.load_instances(args)], ["hoodi"])


if __name__ == "__main__":
    unittest.main()
