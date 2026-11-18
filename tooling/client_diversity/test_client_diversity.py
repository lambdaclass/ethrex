#!/usr/bin/env python3
"""Tests for client_diversity.py. Run with: python3 tooling/client_diversity/test_client_diversity.py"""
import contextlib
import io
import json
import os
import re
import sys
import threading
import unittest
from collections import Counter
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest import mock

sys.dont_write_bytecode = True  # keep __pycache__ out of the repo
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import client_diversity as cd  # noqa: E402


def graffiti(text, raw=None):
    data = raw if raw is not None else text.encode()
    return "0x" + data.ljust(32, b"\x00").hex()


class GraffitiTest(unittest.TestCase):
    def check(self, cases):
        for text, expected in cases:
            with self.subTest(text=text):
                self.assertEqual(cd.el_code_from_graffiti(graffiti(text)), expected)

    def test_mainnet_samples(self):
        self.check([
            ("GE1df3LH57c2", "GE"),
            ("ssv.network RHb9abTK9591", "RH"),
            ("charon/v1.11.1-198ac72 EX80PMd0", "EX"),
            ("Block by Stakely lidoCM001 EXLS", "EX"),
            ("EX2f0bGR70a5 Grandine/2.0.5", "EX"),
            ("EGcf6fCNcf6f", "EG"),
            ("prylabs-lido GE36a7PMd011", "GE"),
            ("validating_from_DAppNodeGE96PM96", "GE"),  # glued: the tag exactly fills the 32 bytes
            ("RP-GP v1.21.0 (RP-GP V1.20.7) GE", "GE"),  # only the EL code fit
            ("GE13d8PMLoca", "GE"),  # local Prysm build
            ("We are all Satoshi GE117ePRLoca", "GE"),
            ("Lighthouse/v8.3.3-57c2abc", None),
            ("Nimbus/v26.10.2-0abc0d-stateofus", None),
            ("bitcoinsuisse.com TK", None),  # a CL code alone says nothing about the EL
            ("Stake Earn Relax stakefish", None),
            ("", None),
        ])

    def test_only_known_shapes(self):
        self.check([
            ("GE10PM", None),  # 6 chars
            ("GE1dLH57c2", None),  # 4 + 2 hex
            ("GE1df3LH57", None),  # 4 + 2 hex
            ("GE1df3LH57c2a", None),  # 13 chars
            ("GE1DF3LH57C2", None),  # commits are lowercase hex
            ("XX12YY34", None),  # unknown codes
            ("GELHx", None),  # not at a word boundary
            ("fooGELHbar", None),
            ("GEPM", "GE"),
            ("NMTK", "NM"),
            ("EXLA", "EX"),
        ])

    def test_tag_closest_to_end_wins(self):
        self.check([
            ("GELH says hi EX12PMd0", "EX"),
            ("NMLH NM12LH34 RH1234LH5678", "RH"),
            ("a longer graffiti, then: EXLS GE", "GE"),  # 32 bytes
        ])

    def test_short_forms_need_a_nearly_full_graffiti(self):
        self.check([
            ("ELITE", None),  # ends in TE, but it is part of a word
            ("x" * 28 + " TE", "TE"),  # 31 bytes: Lighthouse after a 28-byte graffiti
            ("x" * 27 + " TE", None),  # 30 bytes: a client would have used a longer tag
            ("GE", None),
            ("EXLS fan GE", "EX"),
            ("NM706aLH176c TE", "NM"),
            ("I LOVE ANGELS", None),  # glued GE+LS in a short graffiti
            ("HOTELS", None),
            ("x" * 28 + "GETK", "GE"),  # glued: the tag exactly fills the 32 bytes (Teku)
            ("x" * 26 + "GELH", None),  # glued to a 30-byte graffiti: no client does that
            ("you are now breathing manuallyGE", "GE"),  # 30 bytes + glued EL code (Prysm)
            ("x" * 26 + "CHANGE", None),  # 32 bytes, but GE ends a capitalized word
            ("x" * 29 + "GE", None),  # 31 bytes: Prysm would have added a space
            ("x" * 28 + " GE\n", None),  # the code must be the last thing
            ("x" * 28 + "éGE", None),  # 32 bytes, but glued only counts after [a-z0-9]
            ("x" * 29 + "7GE", "GE"),  # 32 bytes, glued after a digit
            ("x" * 28 + "ÜNE", None),  # 32 bytes, but NE ends a capitalized word
            ("GE1df3LH57c2é", None),  # followed by a non-ASCII letter: not a word boundary
            ("x éGELH", None),  # glued after a non-ASCII letter, in a short graffiti
            ("x" * 27 + "ñGE", None),  # 31 bytes: a non-ASCII letter is no separator
        ])

    def test_bad_input(self):
        self.assertIsNone(cd.el_code_from_graffiti("0xzz"))
        self.assertIsNone(cd.el_code_from_graffiti("0x123"))
        self.assertEqual(cd.el_code_from_graffiti(graffiti(None, b"\xff\xfe GE1df3LH57c2")), "GE")


class FakeResponse:
    def __init__(self, body, status=200, content_length="auto", chunk=7):
        self.body, self.status, self.chunk, self.pos = body, status, chunk, 0
        self.content_length = len(body) if content_length == "auto" else content_length
        self.chunked = False

    def getheader(self, name):
        if name == "Content-Length" and self.content_length is not None:
            return str(self.content_length)
        return None

    def read(self, n=-1):
        n = min(n, self.chunk) if n > 0 else len(self.body)
        out = self.body[self.pos:self.pos + n]
        self.pos += len(out)
        return out


class FakeBeacon:
    def __init__(self, responses):
        self.responses = list(responses)

    def request(self, path, timeout=None):
        return self.responses.pop(0)

    def drop_conn(self):
        pass


def validators_json(balances):
    items = ",".join(
        f'{{"index":"{i}","balance":"{b + 7}","status":"active_ongoing",'
        f'"validator":{{"effective_balance":"{b}","slashed":false}}}}'
        for i, b in enumerate(balances))
    return f'{{"execution_optimistic":false,"finalized":true,"data":[{items}]}}'.encode()


class StakeTest(unittest.TestCase):
    def setUp(self):
        patcher = mock.patch.object(cd.time, "sleep")
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_sums_across_chunk_boundaries(self):
        body = validators_json([32_000_000_000, 2_048_000_000_000, 31_000_000_000])
        for chunk in (1, 5, 7, 64, 1 << 20):
            with self.subTest(chunk=chunk):
                beacon = FakeBeacon([FakeResponse(body, chunk=chunk)])
                self.assertEqual(cd.total_active_stake(beacon), (2_111_000_000_000, 3))

    def test_truncated_response_is_retried_then_fails(self):
        body = validators_json([32_000_000_000] * 10)

        def truncated():
            return FakeResponse(body[:len(body) // 2], content_length=len(body))
        with self.assertRaises(cd.IncompleteResponse):
            cd.total_active_stake(FakeBeacon([truncated() for _ in range(3)]))
        beacon = FakeBeacon([truncated(), FakeResponse(body)])
        self.assertEqual(cd.total_active_stake(beacon), (320_000_000_000, 10))
        self.assertEqual(beacon.responses, [])  # the retry happened

    def test_each_completeness_check(self):
        body = validators_json([32_000_000_000] * 10)
        cases = {
            "content-length": FakeResponse(body, content_length=len(body) + 10),  # complete JSON, short read
            "trailing bytes": FakeResponse(body + b"x"),  # balanced brackets, wrong ending
            # No Content-Length and cut right after a validator's "}": only the depth check catches it.
            "depth": FakeResponse(body[:body.index(b'"index":"5"') - 2], content_length=None),
        }
        for name, response in cases.items():
            with self.subTest(name), self.assertRaises(cd.IncompleteResponse):
                cd.total_active_stake(FakeBeacon([response] + [FakeResponse(b"") for _ in range(2)]))

    def test_no_validators_is_an_error(self):
        with self.assertRaises(cd.IncompleteResponse) as ctx:
            cd.total_active_stake(FakeBeacon([FakeResponse(b'{"data":[]}') for _ in range(3)]))
        self.assertIn("no active validators", str(ctx.exception))

    def test_unframed_response_must_end_the_data_array(self):
        body = validators_json([32_000_000_000] * 10)
        cut = body[:body.index(b'"index":"5"') - 2]  # ends right after a validator's "}"
        self.assertTrue(cut.endswith(b"}"))
        with self.assertRaises(cd.IncompleteResponse):
            cd.total_active_stake(FakeBeacon([FakeResponse(cut, content_length=None) for _ in range(3)]))
        whole = FakeResponse(body, content_length=None)
        self.assertEqual(cd.total_active_stake(FakeBeacon([whole])), (320_000_000_000, 10))

    def test_client_error_is_not_retried(self):
        beacon = FakeBeacon([FakeResponse(b"bad request", status=400)])
        with self.assertRaises(cd.HTTPError):
            cd.total_active_stake(beacon)


class HelpersTest(unittest.TestCase):
    def test_parse_duration(self):
        self.assertEqual(cd.parse_duration("30d"), 30 * 86400)
        self.assertEqual(cd.parse_duration("2w"), 14 * 86400)
        self.assertEqual(cd.parse_duration(" 90m "), 5400)
        for bad in ("0d", "3x", "d", "-1h", "1.5d"):
            with self.subTest(bad=bad), self.assertRaises(Exception):
                cd.parse_duration(bad)

    def test_concurrency(self):
        self.assertEqual(cd.concurrency("8"), 8)
        self.assertEqual(cd.concurrency("256"), 256)
        for bad in ("0", "-3", "x", "257"):
            with self.subTest(bad=bad), self.assertRaises(Exception):
                cd.concurrency(bad)

    def test_nice_ceiling(self):
        self.assertAlmostEqual(cd.nice_ceiling(0.0053), 0.006)
        self.assertAlmostEqual(cd.nice_ceiling(0.004), 0.004)
        self.assertAlmostEqual(cd.nice_ceiling(0.31), 0.4)
        self.assertAlmostEqual(cd.nice_ceiling(1.0), 1.0)

    def test_empty_runs(self):
        E = cd.EMPTY
        self.assertEqual(cd.empty_runs([E, E, "", E, E, E, "GE", E], 2), [(0, 2), (3, 3)])
        self.assertEqual(cd.empty_runs(["", "GE"], 1), [])

    def test_fmt_share(self):
        self.assertEqual(cd.fmt_share(0.0030), "0.30%")
        self.assertEqual(cd.fmt_share(0.000048), "0.0048%")
        self.assertEqual(cd.fmt_share(0.3532), "35.32%")
        self.assertEqual(cd.fmt_share(0), "0.00%")
        self.assertEqual(cd.fmt_share(None), "n/a")

    def test_chart_labels_keep_precision(self):
        lines = cd.ascii_chart([0.00005, 0.0001, None], 0, 86400 * 3, "%Y-%m-%d")
        self.assertTrue(lines[0].startswith("0.010%"), lines[0])
        self.assertTrue(lines[5].lstrip().startswith("0.005%"), lines[5])
        self.assertTrue(lines[-2].lstrip().startswith("0.000% +"), lines[-2])
        lines = cd.ascii_chart([0.0021], 0, 86400, "%Y-%m-%d")
        self.assertTrue(lines[0].startswith("0.250%"), lines[0])
        self.assertTrue(lines[5].lstrip().startswith("0.125%"), lines[5])

    def test_beacon_url(self):
        b = cd.BeaconClient("https://user:p%40ss@node.example/api/?apikey=k")
        self.assertEqual((b.host, b.port, b.prefix, b.query), ("node.example", 443, "/api", "apikey=k"))
        self.assertEqual(b.headers["Authorization"], "Basic dXNlcjpwQHNz")  # user:p@ss
        self.assertEqual(cd.BeaconClient("http://[::1]/").port, 80)
        with self.assertRaises(ValueError) as ctx:
            cd.BeaconClient("ftp://user:secret@node")
        self.assertNotIn("secret", str(ctx.exception))


class ReportTest(unittest.TestCase):
    def run_report(self, results, start=1000, bucket=None, stake=(1_000_000 * 10**9, 1000), show_stake=True):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            cd.report(results, start, start + len(results) - 1, 0, 12, "EX", bucket, stake, show_stake)
        return out.getvalue()

    def test_totals_and_stake(self):
        results = ["EX"] * 2 + ["GE"] * 5 + [""] * 3 + [cd.EMPTY] * 2 + [cd.ERROR]
        out = self.run_report(results, bucket=12 * 13)
        self.assertIn("2 empty, 1 skipped", out)
        self.assertIn("Total blocks: 10\n", out)
        self.assertIn("Tagged blocks: 7 (70.0%)", out)
        self.assertIn("- Geth: 50.00%", out)
        self.assertIn("- ethrex: 20.00%", out)
        self.assertIn("- Untagged: 30.00%", out)
        self.assertIn("Estimated stake running on ethrex: 200,000 ETH "
                      "(tagged blocks only, so likely an underestimate; 2 of 10 blocks)", out)
        self.assertIn("(285,714 ETH if untagged blocks", out)

    def test_buckets_align_to_the_end(self):
        # 5 slots, 2-slot buckets: [EX] [GE GE] [EX EX]; the oldest bucket is partial.
        out = self.run_report(["EX", "GE", "GE", "EX", "EX"], bucket=24)
        self.assertIn("(first bucket 100.00%, partial; last bucket 100.00%)", out)
        chart = [line for line in out.splitlines() if "|" in line]
        self.assertEqual(chart[-1].split("|")[1], "#####       #####")  # 6-char columns

    def test_absent_client(self):
        out = self.run_report(["GE", ""] * 10, stake=None)
        self.assertIn("No ethrex-tagged blocks in this range.", out)
        self.assertIn("- ethrex: 0.00%", out)
        self.assertIn("Estimated stake running on ethrex: 0 ETH (no ethrex-tagged blocks)", out)
        self.assertNotIn("Total active stake", out)

    def test_no_stake(self):
        out = self.run_report(["EX", "GE"] * 10, stake=None, show_stake=False)
        self.assertNotIn("stake", out)

    def test_stake_download_failed(self):
        out = self.run_report(["EX", "GE"] * 10, stake=None)
        self.assertIn("Estimated stake running on ethrex: n/a (could not fetch validator balances)", out)

    def test_tiny_partial_bucket_does_not_set_the_scale(self):
        # 2-minute buckets (10 slots): a 1-slot partial bucket at 100%, then three at 10%.
        results = ["EX"] + (["EX"] + ["GE"] * 9) * 3
        out = self.run_report(results, bucket=120)
        chart = [line for line in out.splitlines() if "|" in line or "+" in line]
        self.assertTrue(chart[0].startswith("10.00% |"), chart[0])

    def test_single_bucket_is_not_charted(self):
        out = self.run_report(["EX", "GE"] * 10, bucket=3600)
        self.assertIn("Range too short to chart per 1h", out)
        self.assertNotIn("|", out)


class FakeBeaconNode(BaseHTTPRequestHandler):
    """Serves a tiny chain: SECONDS_PER_SLOT=12, genesis at 0."""
    protocol_version = "HTTP/1.1"
    finalized = 0
    blocks = {}  # slot -> graffiti text; other slots are empty (404)
    failing = set()  # slots that answer HTTP 500
    forbidden = set()  # slots that answer HTTP 403 (not retried)
    retry_after = "2"  # Retry-After sent with 429s
    flaky = {}  # slot -> statuses to answer before serving the block normally
    wrong_slot = set()  # slots that answer with the previous block
    ahead_slot = set()  # slots that answer with the next block
    spec = {"SECONDS_PER_SLOT": "12"}
    blinded_status = 200  # status of every blinded block request, when not 200
    seen_query, seen_auth, hits = set(), set(), Counter()

    def do_GET(self):
        path, _, query = self.path.partition("?")
        FakeBeaconNode.seen_query.add(query)
        FakeBeaconNode.seen_auth.add(self.headers.get("Authorization"))
        FakeBeaconNode.hits[path] += 1
        m = re.fullmatch(r"/eth/(v1/beacon/blinded_blocks|v2/beacon/blocks)/(\d+)", path)
        if path == "/eth/v1/config/spec":
            self.reply(200, {"data": self.spec})
        elif path == "/eth/v1/beacon/genesis":
            self.reply(200, {"data": {"genesis_time": "0"}})
        elif path == "/eth/v1/beacon/headers/finalized":
            self.reply(200, {"data": {"header": {"message": {"slot": str(self.finalized)}}}})
        elif path == "/eth/v1/beacon/states/finalized/validators":
            self.reply(200, validators_json([32_000_000_000] * 4))
        elif m and m.group(1).startswith("v1") and self.blinded_status != 200:
            self.reply(self.blinded_status, {"message": "no blinded blocks here"})
        elif m:
            slot = int(m.group(2))
            if slot in self.failing:
                self.reply(500, {"message": "boom"})
            elif slot in self.forbidden:
                self.reply(403, {"message": "forbidden"})
            elif self.flaky.get(slot):
                status = self.flaky[slot].pop(0)
                self.reply(status, {"message": "try again"},
                           {"Retry-After": self.retry_after} if status == 429 else {})
            elif slot in self.blocks:
                served = slot - 1 if slot in self.wrong_slot else slot + 1 if slot in self.ahead_slot else slot
                self.reply(200, {"data": {"message": {"slot": str(served),
                                                      "body": {"graffiti": graffiti(self.blocks[slot])}}}})
            else:
                self.reply(404, {"message": "not found"})
        else:
            self.reply(404, {"message": "not found"})

    def reply(self, status, payload, headers=None):
        body = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
        self.send_response(status)
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def chain(first_slot):
    """Blocks from first_slot to 1000, except every 10th slot, which is empty."""
    return {s: ("EXLS" if s % 4 == 0 else "GE1df3LH57c2" if s % 4 == 1 else "")
            for s in range(first_slot, 1001) if s % 10 != 5}


class MainTest(unittest.TestCase):
    def setUp(self):
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), FakeBeaconNode)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        FakeBeaconNode.finalized, FakeBeaconNode.blinded_status = 1000, 200
        FakeBeaconNode.failing, FakeBeaconNode.wrong_slot, FakeBeaconNode.flaky = set(), set(), {}
        FakeBeaconNode.ahead_slot, FakeBeaconNode.spec = set(), {"SECONDS_PER_SLOT": "12"}
        FakeBeaconNode.forbidden, FakeBeaconNode.retry_after = set(), "2"
        FakeBeaconNode.seen_query, FakeBeaconNode.seen_auth, FakeBeaconNode.hits = set(), set(), Counter()
        FakeBeaconNode.blocks = chain(900)

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()

    def run_main(self, *args):
        out, err = io.StringIO(), io.StringIO()
        argv = ["client_diversity.py", "--beacon-url", self.url, "--concurrency", "4", *args]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(cd, "RETRIES", 1), \
                mock.patch.object(cd.time, "sleep"), mock.patch.object(cd, "backoff") as self.backoff, \
                contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            cd.main()
        return out.getvalue(), err.getvalue()

    def backoff_delays(self):
        return [c.args[1] for c in self.backoff.call_args_list]

    def test_spec_prefers_seconds_per_slot_and_rejects_zero(self):
        FakeBeaconNode.spec = {"SECONDS_PER_SLOT": "12", "SLOT_DURATION_MS": "6000"}
        out, _ = self.run_main("--range", "1h", "--no-stake")
        self.assertIn("Total blocks: 91\n", out)  # 300 slots of 12 s, not 600 of 6 s
        FakeBeaconNode.spec = {"SECONDS_PER_SLOT": "0"}
        with self.assertRaises(SystemExit) as ctx:
            self.run_main("--range", "1h")
        self.assertIn("unsupported slot length", str(ctx.exception.code))

    def test_default_range_is_30_days(self):
        out, err = self.run_main("--no-stake")  # 30d is more than the 1,001 slots this chain has
        self.assertIn("Fetching slots 0..1,000", err)
        self.assertIn("(slots 900..1,000, 10 empty)", out)

    def test_slot_duration_in_milliseconds(self):
        FakeBeaconNode.spec = {"SLOT_DURATION_MS": "12000"}
        out, _ = self.run_main("--range", "1h", "--no-stake")
        self.assertIn("Total blocks: 91\n", out)

    def test_truncates_missing_history_and_counts_blocks(self):
        out, err = self.run_main("--range", "1h", "--bucket", "1h")
        # 1h = 300 slots (701..1000); the node only has blocks from slot 900.
        self.assertIn("no blocks before slot 900", err)
        self.assertIn("(slots 900..1,000, 10 empty)", out)
        self.assertIn("Total blocks: 91\n", out)
        self.assertIn("Tagged blocks: 46 (50.5%)", out)
        self.assertIn("- ethrex: 28.57%", out)  # 26 of 91
        self.assertIn("Estimated stake running on ethrex: 37 ETH "  # 128 ETH * 26/91
                      "(tagged blocks only, so likely an underestimate; 26 of 91 blocks)", out)
        self.assertIn("(72 ETH if untagged", out)  # 128 ETH * 26/46
        self.assertIn("blinded_blocks", err)

    def test_falls_back_to_full_blocks(self):
        for status in (404, 500, 501):
            with self.subTest(status=status):
                FakeBeaconNode.blinded_status = status
                out, err = self.run_main("--range", "1h", "--no-stake")
                self.assertIn("/eth/v2/beacon/blocks/{slot}", err)
                self.assertIn("Total blocks: 91\n", out)
                self.assertNotIn("stake", out)

    def test_failed_slots_are_skipped(self):
        FakeBeaconNode.blocks = chain(701)
        FakeBeaconNode.failing = set(range(900, 1000, 10))
        out, err = self.run_main("--range", "1h")
        expected = len(set(FakeBeaconNode.blocks) - FakeBeaconNode.failing)
        skipped = len(FakeBeaconNode.failing)
        self.assertIn(f"skipped {skipped} slots", err)
        self.assertIn(f"{skipped} skipped", out)
        self.assertIn(f"Total blocks: {expected}\n", out)

    def test_too_many_skipped_slots_is_an_error(self):
        # 403s aren't retried, so they fail fast and never trip the time-based abort.
        FakeBeaconNode.blocks = chain(701)
        FakeBeaconNode.forbidden = set(range(760, 1000))
        with self.assertRaises(SystemExit) as ctx:
            self.run_main("--range", "1h", "--no-stake")
        self.assertIn("240 of 300 slots could not be fetched", str(ctx.exception.code))

    def test_long_failure_run_aborts_however_fast(self):
        FakeBeaconNode.blocks = chain(701)
        FakeBeaconNode.forbidden = set(range(760, 1000))
        with self.assertRaises(SystemExit) as ctx, mock.patch.object(cd, "LONG_FAILURE_RUN", 50):
            self.run_main("--range", "1h", "--no-stake")
        self.assertIn("stopped answering", str(ctx.exception.code))

    def test_leading_failures_are_not_missing_history(self):
        # Slots 701..800 alternate between failing and empty; blocks start at 801.
        FakeBeaconNode.blocks = chain(801)
        FakeBeaconNode.failing = set(range(701, 801, 2))
        with mock.patch.object(cd, "MAX_SKIPPED_SHARE", 1.0):
            out, err = self.run_main("--range", "1h", "--no-stake")
        self.assertNotIn("no blocks before", err)
        self.assertIn("(slots 701..1,000, 70 empty, 50 skipped)", out)

    def test_transient_errors_are_retried(self):
        FakeBeaconNode.flaky = {950: [500], 960: [429], 970: [501]}
        out, err = self.run_main("--range", "1h", "--no-stake")
        self.assertIn("1 skipped", out)  # only the 501, which is not retried
        self.assertIn("slot 970: HTTPError", err)
        self.assertEqual(FakeBeaconNode.hits["/eth/v1/beacon/blinded_blocks/950"], 2)
        self.assertEqual(FakeBeaconNode.hits["/eth/v1/beacon/blinded_blocks/970"], 1)
        self.assertIn(2, self.backoff_delays())  # the 429's Retry-After

    def test_retry_after_is_capped(self):
        FakeBeaconNode.flaky, FakeBeaconNode.retry_after = {960: [429]}, "100"
        self.run_main("--range", "1h", "--no-stake")
        self.assertIn(cd.MAX_RETRY_AFTER, self.backoff_delays())
        self.assertNotIn(100, self.backoff_delays())

    def test_aborts_when_the_node_stops_answering(self):
        FakeBeaconNode.blocks = chain(701)
        FakeBeaconNode.failing = set(range(800, 1000))  # the finalized block still answers
        with self.assertRaises(SystemExit) as ctx, mock.patch.object(cd, "NODE_DOWN_AFTER", 0):
            self.run_main("--range", "1h")
        self.assertIn("stopped answering", str(ctx.exception.code))

    def test_short_outage_does_not_abort(self):
        FakeBeaconNode.blocks = chain(701)
        FakeBeaconNode.failing = set(range(800, 1000))
        with mock.patch.object(cd, "MAX_SKIPPED_SHARE", 1.0):
            out, err = self.run_main("--range", "1h", "--no-stake")  # NODE_DOWN_AFTER not reached
        self.assertIn("skipped 200 slots", err)

    def test_mismatched_slots(self):
        FakeBeaconNode.wrong_slot = {950, 960}  # answered with the previous block: empty slots
        FakeBeaconNode.ahead_slot = {970}  # answered with a later block: an error
        out, err = self.run_main("--range", "1h", "--no-stake")
        self.assertIn("asked for slot 970, got slot 971", err)
        self.assertIn("12 empty, 1 skipped", out)

    def test_wrong_url_prefix(self):
        self.url += "/not/the/api"
        with self.assertRaises(SystemExit) as ctx:
            self.run_main("--range", "1h")
        self.assertIn("is this a beacon node API URL?", str(ctx.exception.code))

    def test_client_by_alias_or_code(self):
        for client in ("go-ethereum", "GE", "geth"):
            with self.subTest(client=client):
                out, _ = self.run_main("--range", "1h", "--no-stake", "--client", client)
                self.assertIn("Geth share of total blocks", out)

    def test_credentials_and_query_string_are_forwarded_but_not_printed(self):
        self.url = self.url.replace("http://", "http://user:pass@") + "/?apikey=secret"
        FakeBeaconNode.failing = {990}  # so an HTTP error gets printed
        _, err = self.run_main("--range", "5m")
        self.assertEqual(FakeBeaconNode.seen_query, {"apikey=secret", "status=active&apikey=secret"})
        self.assertEqual(FakeBeaconNode.seen_auth, {"Basic dXNlcjpwYXNz"})  # user:pass
        self.assertIn("slot 990: HTTPError: GET /eth/v1/beacon/blinded_blocks/990: HTTP 500", err)
        self.assertNotIn("pass", err)
        self.assertNotIn("secret", err)


if __name__ == "__main__":
    unittest.main()
