import unittest
from unittest import mock

from multisync import beacon_checks as bc

ROOT_A = "0x" + "aa" * 32
ROOT_B = "0x" + "bb" * 32
HASH = "0x" + "cc" * 32
OTHER_HASH = "0x" + "ee" * 32
T = bc.ClTimeouts(startup=100, unresponsive=50, head_stall=60, provider_interval=10)


class FakeApi:
    def __init__(self, syncing=None, finalized=None, blocks=None):
        self.syncing_data = syncing
        self.finalized_value = finalized
        self.blocks = blocks or {}

    def syncing(self):
        return self.syncing_data

    def finalized_checkpoint(self, state_id="head"):
        return self.finalized_value

    def block(self, block_id):
        return self.blocks.get(block_id)


def synced(head, **flags):
    return {"head_slot": str(head), "sync_distance": "0", "is_syncing": False,
            "is_optimistic": False, "el_offline": False, **flags}


def fulu_block(block_hash):
    return {"data": {"message": {"body": {"execution_payload": {"block_hash": block_hash}}}}}


def gloas_block(parent_hash):
    bid = {"message": {"parent_block_hash": parent_hash, "block_hash": "0x" + "dd" * 32}}
    return {"data": {"message": {"body": {"signed_execution_payload_bid": bid}}}}


class HelpersTest(unittest.TestCase):
    def test_normalize_root(self):
        self.assertEqual(bc.normalize_root("AB" * 32), "0x" + "ab" * 32)
        self.assertEqual(bc.normalize_root(ROOT_A), ROOT_A)

    def test_fcu_hash_before_gloas_is_the_payload_hash(self):
        self.assertEqual(bc.fcu_finalized_hash(fulu_block("0x" + "CC" * 32)), HASH)

    def test_fcu_hash_from_gloas_is_the_bid_parent(self):
        self.assertEqual(bc.fcu_finalized_hash(gloas_block(HASH)), HASH)

    def test_fcu_hash_without_a_payload(self):
        self.assertIsNone(bc.fcu_finalized_hash({"data": {"message": {"body": {}}}}))
        self.assertIsNone(bc.fcu_finalized_hash({}))


class LivenessTest(unittest.TestCase):
    def test_startup_timeout(self):
        state, api, provider = bc.ClState(), FakeApi(), FakeApi()
        self.assertFalse(bc.update(state, api, provider, 1000, T))
        self.assertEqual(state.status, "waiting")
        bc.update(state, api, provider, 1101, T)
        self.assertEqual(state.status, "failed")
        self.assertIn("CL: no Beacon API answer", state.error)

    def test_follows_then_stalls(self):
        state, api, provider = bc.ClState(), FakeApi(syncing=synced(10)), FakeApi()
        self.assertTrue(bc.update(state, api, provider, 0, T))
        self.assertEqual(state.status, "following")
        api.syncing_data = synced(11)
        bc.update(state, api, provider, 30, T)
        bc.update(state, api, provider, 80, T)  # 50 s without progress: under the limit
        self.assertEqual(state.status, "following")
        bc.update(state, api, provider, 91, T)
        self.assertEqual(state.status, "failed")
        self.assertIn("CL: head stuck at slot 11", state.error)

    def test_unresponsive_reports_the_exit_code(self):
        state, api, provider = bc.ClState(), FakeApi(syncing=synced(10)), FakeApi()
        bc.update(state, api, provider, 0, T)
        api.syncing_data = None
        bc.update(state, api, provider, 40, T, exit_code=lambda: 137)
        self.assertEqual(state.status, "following")
        bc.update(state, api, provider, 51, T, exit_code=lambda: 137)
        self.assertIn("CL: unresponsive", state.error)
        self.assertIn("code 137", state.error)


class ProviderTest(unittest.TestCase):
    def setUp(self):
        self.state = bc.ClState()
        self.api = FakeApi(syncing=synced(10), finalized=(100, ROOT_A))
        self.provider = FakeApi()

    def poll(self, now, provider_checkpoint):
        self.provider.finalized_value = provider_checkpoint
        self.api.syncing_data = synced(10 + now)  # keep the head moving
        return bc.update(self.state, self.api, self.provider, now, T)

    def test_match_at_a_shared_epoch(self):
        self.poll(0, (100, ROOT_A))
        self.assertEqual(self.state.provider_match_epoch, 100)

    def test_mismatch_fails(self):
        self.poll(0, (100, ROOT_B))
        self.assertEqual(self.state.status, "failed")
        self.assertIn("finalized root at epoch 100", self.state.error)

    def test_provider_ahead_then_behind(self):
        self.poll(0, (101, ROOT_B))  # newer than anything we saw: skipped
        self.assertEqual(self.state.status, "following")
        self.assertIsNone(self.state.provider_match_epoch)
        self.api.finalized_value = (101, ROOT_B)
        self.poll(10, (100, ROOT_A))  # behind us now, but we saw 100: compared
        self.assertEqual(self.state.provider_match_epoch, 100)

    def test_provider_down_counts_failures(self):
        self.poll(0, None)
        self.poll(10, None)
        self.assertEqual(self.state.provider_failures, 2)
        self.assertEqual(self.state.status, "following")

    def test_polls_respect_the_interval(self):
        self.poll(0, None)
        self.poll(5, None)
        self.assertEqual(self.state.provider_failures, 1)

    def test_matches_count_in_the_window_only_once_it_started(self):
        self.poll(0, (100, ROOT_A))
        self.assertIsNone(self.state.window_match_epoch)
        bc.start_window(self.state)
        self.assertEqual(self.state.window_start_epoch, 100)
        self.poll(10, (100, ROOT_A))
        self.assertEqual(self.state.window_match_epoch, 100)


class EndChecksTest(unittest.TestCase):
    def ready_state(self):
        return bc.ClState(status="following", finalized={100: ROOT_A, 102: ROOT_B}, finalized_epoch=102,
                          in_window=True, window_start_epoch=100, window_match_epoch=101)

    def passing_api(self, block=None, **flags):
        return FakeApi(syncing=synced(5, **flags), finalized=(102, ROOT_B), blocks={ROOT_B: block or fulu_block(HASH)})

    def run_checks(self, state, api, el_hash=HASH):
        return bc.end_checks(state, api, lambda: el_hash, sleep=lambda seconds: None)

    def test_passes(self):
        state = self.ready_state()
        self.assertIsNone(self.run_checks(state, self.passing_api()))
        self.assertIn("EL/CL hash ✓", bc.describe(state))

    def test_optimistic_fails(self):
        error = self.run_checks(self.ready_state(), self.passing_api(is_optimistic=True))
        self.assertIn("CL: is_optimistic is true", error)

    def test_missing_el_offline_counts_as_false(self):
        api = self.passing_api()
        del api.syncing_data["el_offline"]
        self.assertIsNone(self.run_checks(self.ready_state(), api))

    def test_finality_must_advance(self):
        state = self.ready_state()
        state.finalized_epoch = 100
        self.assertIn("did not advance", self.run_checks(state, self.passing_api()))

    def test_needs_a_provider_match_in_the_window(self):
        state = self.ready_state()
        state.window_match_epoch = None
        state.window_provider_failures, state.window_provider_polls = 3, 7
        error = self.run_checks(state, self.passing_api())
        self.assertIn("matched the provider", error)
        self.assertIn("(3 of 7 provider polls failed)", error)

    def test_hash_mismatch_after_retries(self):
        sleeps = []
        error = bc.end_checks(self.ready_state(), self.passing_api(), lambda: OTHER_HASH, sleep=sleeps.append)
        self.assertIn("CL: EL/CL finalized hash mismatch", error)
        self.assertEqual(len(sleeps), 9)
        self.assertIn(OTHER_HASH, error)
        self.assertIn(HASH, error)
        self.assertEqual(bc.ClState().hash_note, "")

    def test_hash_catches_up_on_a_retry(self):
        answers = iter([None, OTHER_HASH, HASH])
        api = self.passing_api(block=gloas_block(HASH))
        self.assertIsNone(bc.end_checks(self.ready_state(), api, lambda: next(answers), sleep=lambda s: None))

    def test_hash_note_shows_a_cross_on_mismatch(self):
        state = self.ready_state()
        self.run_checks(state, self.passing_api(), el_hash=OTHER_HASH)
        self.assertIn("EL/CL hash ✗", bc.describe(state))

    def test_fork_choice_finalized_block_also_matches(self):
        # The head state still names the old checkpoint; fork choice moved on.
        api = self.passing_api(block=fulu_block(OTHER_HASH))
        api.blocks["finalized"] = fulu_block(HASH)
        self.assertIsNone(self.run_checks(self.ready_state(), api))

    def test_neither_candidate_matches(self):
        api = self.passing_api(block=fulu_block(OTHER_HASH))
        api.blocks["finalized"] = fulu_block("0x" + "11" * 32)
        error = self.run_checks(self.ready_state(), api)
        self.assertIn(OTHER_HASH, error)
        self.assertIn("0x" + "11" * 32, error)
        self.assertIn(HASH, error)

    def test_transient_optimistic_is_retried(self):
        api = self.passing_api(is_optimistic=True)
        reads = []

        def sleep(seconds):
            reads.append(seconds)
            api.syncing_data = synced(5)

        self.assertIsNone(bc.end_checks(self.ready_state(), api, lambda: HASH, sleep=sleep))
        self.assertEqual(len(reads), 1)

    def test_transient_api_outage_is_retried(self):
        api = self.passing_api()
        api.syncing_data = None

        def sleep(seconds):
            api.syncing_data = synced(5)

        self.assertIsNone(bc.end_checks(self.ready_state(), api, lambda: HASH, sleep=sleep))

    def test_persistent_api_outage_reports_it(self):
        api = self.passing_api()
        api.syncing_data = None
        self.assertIn("did not answer", self.run_checks(self.ready_state(), api))

    def test_no_finality_observed_before_the_window_still_needs_an_advance(self):
        state = self.ready_state()
        state.window_start_epoch = None
        self.assertIn("no finalized checkpoint observed", self.run_checks(state, self.passing_api()))

    def test_not_following(self):
        self.assertIn("CL:", self.run_checks(bc.ClState(), FakeApi()))


class WindowTest(unittest.TestCase):
    def test_start_window_without_finality_leaves_the_start_unset(self):
        state = bc.ClState(status="following")
        bc.start_window(state)
        self.assertIsNone(state.window_start_epoch)

    def test_first_observation_in_the_window_is_the_baseline(self):
        state, api = bc.ClState(), FakeApi(syncing=synced(10), finalized=(100, ROOT_A))
        bc.update(state, api, FakeApi(), 0, T)
        state = bc.ClState()
        bc.start_window(state)
        bc.update(state, api, FakeApi(), 0, T)
        self.assertEqual(state.window_start_epoch, 100)
        api.finalized_value, api.syncing_data = (101, ROOT_B), synced(11)
        bc.update(state, api, FakeApi(), 1, T)
        self.assertEqual(state.window_start_epoch, 100)
        self.assertEqual(state.finalized_epoch, 101)

    def test_provider_failures_are_counted_per_window(self):
        state, api, provider = bc.ClState(), FakeApi(syncing=synced(10), finalized=(100, ROOT_A)), FakeApi()
        bc.update(state, api, provider, 0, T)  # one failure before the window
        bc.start_window(state)
        api.syncing_data = synced(11)
        bc.update(state, api, provider, 10, T)
        self.assertEqual((state.window_provider_polls, state.window_provider_failures), (1, 1))
        self.assertEqual(state.provider_failures, 2)
        bc.start_window(state)
        self.assertEqual((state.window_provider_polls, state.window_provider_failures), (0, 0))

    def test_container_exit_during_startup_fails_at_once(self):
        state = bc.ClState()
        bc.update(state, FakeApi(), FakeApi(), 0, T, exit_code=lambda: 1)
        self.assertEqual(state.status, "failed")
        self.assertIn("exited with code 1 before its Beacon API answered", state.error)

    def test_running_container_keeps_waiting(self):
        state = bc.ClState()
        bc.update(state, FakeApi(), FakeApi(), 0, T, exit_code=lambda: None)
        self.assertEqual(state.status, "waiting")


def fake_response(status=200, body=None):
    response = mock.Mock(status_code=status)
    response.json.return_value = body
    return response


class BeaconApiTest(unittest.TestCase):
    def call(self, response, method, *args):
        with mock.patch("multisync.beacon_checks.requests.get", return_value=response):
            return getattr(bc.BeaconApi("http://cl:5052/"), method)(*args)

    def test_finalized_checkpoint_parses_strings_and_normalizes_the_root(self):
        body = {"data": {"finalized": {"epoch": "123", "root": "AB" * 32}}}
        self.assertEqual(self.call(fake_response(body=body), "finalized_checkpoint"), (123, "0x" + "ab" * 32))

    def test_non_200_is_none(self):
        self.assertIsNone(self.call(fake_response(status=404, body={"data": {}}), "finalized_checkpoint"))

    def test_body_without_data_is_none(self):
        self.assertIsNone(self.call(fake_response(body={"message": "x"}), "finalized_checkpoint"))
        self.assertIsNone(self.call(fake_response(body={"message": "x"}), "syncing"))

    def test_syncing_returns_the_data_dict(self):
        self.assertEqual(self.call(fake_response(body={"data": synced(5)}), "syncing"), synced(5))

    def test_request_error_is_none(self):
        with mock.patch("multisync.beacon_checks.requests.get", side_effect=OSError):
            self.assertIsNone(bc.BeaconApi("http://cl:5052").syncing())


class DescribeTest(unittest.TestCase):
    def test_waiting_and_following(self):
        self.assertEqual(bc.describe(bc.ClState()), "waiting for its Beacon API")
        state = bc.ClState(status="following", head_slot=7, finalized_epoch=3, window_start_epoch=1,
                           provider_match_epoch=3)
        self.assertEqual(bc.describe(state), "following, head 7, finalized 3 (+2), provider ✓ @3")


if __name__ == "__main__":
    unittest.main()
