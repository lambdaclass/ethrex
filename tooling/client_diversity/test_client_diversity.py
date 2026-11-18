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
            ("validating_from_DAppNodeGE96PM96", "GE"),  # glued to a full user graffiti
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
            ("a long graffiti, then: EXLS GE", "GE"),
        ])

    def test_short_forms_need_a_nearly_full_graffiti(self):
        self.check([
            ("ELITE", None),  # ends in TE, but it is part of a word
            ("x" * 27 + " TE", "TE"),  # 30 bytes
            ("x" * 26 + " TE", None),  # a client would have used a longer tag
            ("GE", None),
            ("EXLS fan GE", "EX"),
            ("NM706aLH176c TE", "NM"),
            ("I LOVE ANGELS", None),  # glued GE+LS in a short graffiti
            ("HOTELS", None),
            ("x" * 26 + "GELH", "GE"),  # glued to a 30-byte graffiti
            ("you are now breathing manuallyGE", "GE"),  # 30 bytes + glued EL code (Prysm)
            ("x" * 26 + "CHANGE", None),  # 32 bytes, but GE ends a capitalized word
            ("x" * 29 + "GE", None),  # 31 bytes: Prysm would have added a space
            ("x" * 27 + " GE\n", None),  # the code must be the last thing
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
        truncated = [FakeResponse(body[:len(body) // 2], content_length=len(body)) for _ in range(3)]
        with self.assertRaises(cd.IncompleteResponse):
            cd.total_active_stake(FakeBeacon(truncated))
        ok = FakeResponse(body)
        self.assertEqual(cd.total_active_stake(FakeBeacon([truncated[0], ok])), (320_000_000_000, 10))

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

    def test_positive_int(self):
        self.assertEqual(cd.positive_int("8"), 8)
        for bad in ("0", "-3", "x"):
            with self.subTest(bad=bad), self.assertRaises(Exception):
                cd.positive_int(bad)

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
        with self.assertRaises(ValueError):
            cd.BeaconClient("ftp://node")


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
        self.assertIn("Estimated stake running on ethrex: 200,000 ETH (lower bound; 2 of 10 blocks)", out)
        self.assertIn("(285,714 ETH if untagged blocks", out)

    def test_buckets_align_to_the_end(self):
        # 5 slots, 2-slot buckets: [EX] [GE GE] [EX EX]; the oldest bucket is partial.
        out = self.run_report(["EX", "GE", "GE", "EX", "EX"], bucket=24)
        self.assertIn("(first bucket (partial) 100.00%, last bucket 100.00%)", out)
        chart = [line for line in out.splitlines() if "|" in line]
        self.assertEqual(chart[-1].split("|")[1], "#####       #####")  # 6-char columns

    def test_absent_client(self):
        out = self.run_report(["GE", ""] * 10, stake=None)
        self.assertIn("No ethrex blocks in this range.", out)
        self.assertIn("- ethrex: 0.00%", out)
        self.assertIn("Estimated stake running on ethrex: 0 ETH (no ethrex blocks)", out)
        self.assertNotIn("Total active stake", out)

    def test_no_stake(self):
        out = self.run_report(["EX", "GE"] * 10, stake=None, show_stake=False)
        self.assertNotIn("stake", out)


class FakeBeaconNode(BaseHTTPRequestHandler):
    """Serves a tiny chain: SECONDS_PER_SLOT=12, genesis at 0."""
    protocol_version = "HTTP/1.1"
    finalized = 0
    blocks = {}  # slot -> graffiti text; other slots are empty (404)
    failing = set()  # slots that answer HTTP 500
    wrong_slot = set()  # slots that answer with the previous block
    blinded_status = 200  # status of every blinded block request, when not 200
    seen_query = set()

    def do_GET(self):
        path, _, query = self.path.partition("?")
        FakeBeaconNode.seen_query.add(query)
        m = re.fullmatch(r"/eth/(v1/beacon/blinded_blocks|v2/beacon/blocks)/(\d+)", path)
        if path == "/eth/v1/config/spec":
            self.reply(200, {"data": {"SECONDS_PER_SLOT": "12"}})
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
            elif slot in self.blocks:
                served = slot - 1 if slot in self.wrong_slot else slot
                self.reply(200, {"data": {"message": {"slot": str(served),
                                                      "body": {"graffiti": graffiti(self.blocks[slot])}}}})
            else:
                self.reply(404, {"message": "not found"})
        else:
            self.reply(404, {"message": "not found"})

    def reply(self, status, payload):
        body = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
        self.send_response(status)
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
        FakeBeaconNode.failing, FakeBeaconNode.wrong_slot, FakeBeaconNode.seen_query = set(), set(), set()
        FakeBeaconNode.blocks = chain(900)

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()

    def run_main(self, *args):
        out, err = io.StringIO(), io.StringIO()
        argv = ["client_diversity.py", "--beacon-url", self.url, "--concurrency", "4", *args]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(cd, "RETRIES", 1), \
                mock.patch.object(cd.time, "sleep"), contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            cd.main()
        return out.getvalue(), err.getvalue()

    def test_truncates_missing_history_and_counts_blocks(self):
        out, err = self.run_main("--range", "1h", "--bucket", "1h")
        # 1h = 300 slots (701..1000); the node only has blocks from slot 900.
        self.assertIn("no blocks before slot 900", err)
        self.assertIn("(slots 900..1,000, 10 empty)", out)
        self.assertIn("Total blocks: 91\n", out)
        self.assertIn("Tagged blocks: 46 (50.5%)", out)
        self.assertIn("- ethrex: 28.57%", out)  # 26 of 91
        self.assertIn("Estimated stake running on ethrex: 37 ETH (lower bound; 26 of 91 blocks)", out)  # 128 * 26/91
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
        FakeBeaconNode.failing = set(range(701, 761)) | set(range(900, 1000, 7))
        out, err = self.run_main("--range", "1h")
        expected = len(set(FakeBeaconNode.blocks) - FakeBeaconNode.failing)
        skipped = len(FakeBeaconNode.failing)
        self.assertIn(f"skipped {skipped} slots", err)
        self.assertNotIn("no blocks before", err)  # leading failures are not missing history
        self.assertIn(f"{skipped} skipped", out)
        self.assertIn(f"Total blocks: {expected}\n", out)

    def test_aborts_when_the_node_stops_answering(self):
        FakeBeaconNode.blocks = chain(701)
        FakeBeaconNode.failing = set(range(800, 1000))  # the finalized block still answers
        with self.assertRaises(SystemExit) as ctx:
            self.run_main("--range", "1h")
        self.assertIn("stopped answering", str(ctx.exception.code))

    def test_mismatched_slot_is_an_error(self):
        FakeBeaconNode.wrong_slot = {950, 960}
        out, err = self.run_main("--range", "1h", "--no-stake")
        self.assertIn("asked for slot 950, got slot 949", err)
        self.assertIn("2 skipped", out)

    def test_wrong_url_prefix(self):
        self.url += "/not/the/api"
        with self.assertRaises(SystemExit) as ctx:
            self.run_main("--range", "1h")
        self.assertIn("is this a beacon node API URL?", str(ctx.exception.code))

    def test_query_string_is_forwarded(self):
        self.url += "/?apikey=secret"
        self.run_main("--range", "5m")
        self.assertEqual(FakeBeaconNode.seen_query, {"apikey=secret", "status=active&apikey=secret"})


if __name__ == "__main__":
    unittest.main()
