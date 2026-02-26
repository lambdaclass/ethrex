"""Tests for rebuild-index.py"""
import json
import os
import tempfile
import unittest
from pathlib import Path

# Import the module under test
from rebuild_index import scan_data_dir, write_index


class TestScanDataDir(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.mkdtemp()

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmpdir)

    def _make_file(self, relpath: str, content: str = "{}"):
        path = Path(self.tmpdir) / relpath
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def test_empty_directory(self):
        runs = scan_data_dir(self.tmpdir)
        self.assertEqual(runs, [])

    def test_single_bench_file(self):
        self._make_file("2026-02-26/abc123def-bench.json")
        runs = scan_data_dir(self.tmpdir)
        self.assertEqual(len(runs), 1)
        self.assertEqual(runs[0]["date"], "2026-02-26")
        self.assertEqual(runs[0]["commit"], "abc123def")
        self.assertEqual(runs[0]["bench"], "2026-02-26/abc123def-bench.json")

    def test_bench_with_jit_and_regression(self):
        self._make_file("2026-02-26/abc123def-bench.json")
        self._make_file("2026-02-26/abc123def-jit-bench.json")
        self._make_file("2026-02-26/abc123def-regression.json")
        runs = scan_data_dir(self.tmpdir)
        self.assertEqual(len(runs), 1)
        self.assertIn("jit_bench", runs[0])
        self.assertIn("regression", runs[0])

    def test_multiple_dates_sorted(self):
        self._make_file("2026-02-25/aaa-bench.json")
        self._make_file("2026-02-26/bbb-bench.json")
        self._make_file("2026-02-24/ccc-bench.json")
        runs = scan_data_dir(self.tmpdir)
        self.assertEqual(len(runs), 3)
        dates = [r["date"] for r in runs]
        self.assertEqual(dates, ["2026-02-24", "2026-02-25", "2026-02-26"])

    def test_multiple_commits_same_date(self):
        self._make_file("2026-02-26/aaa-bench.json")
        self._make_file("2026-02-26/bbb-bench.json")
        runs = scan_data_dir(self.tmpdir)
        self.assertEqual(len(runs), 2)

    def test_ignores_non_bench_files(self):
        self._make_file("2026-02-26/abc123def-bench.json")
        self._make_file("2026-02-26/readme.txt")
        runs = scan_data_dir(self.tmpdir)
        self.assertEqual(len(runs), 1)

    def test_optional_fields_absent(self):
        self._make_file("2026-02-26/abc123def-bench.json")
        runs = scan_data_dir(self.tmpdir)
        self.assertNotIn("jit_bench", runs[0])
        self.assertNotIn("regression", runs[0])


class TestWriteIndex(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.mkdtemp()

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmpdir)

    def test_writes_valid_json(self):
        runs = [{"date": "2026-02-26", "commit": "abc", "bench": "2026-02-26/abc-bench.json"}]
        out_path = os.path.join(self.tmpdir, "index.json")
        write_index(runs, out_path)
        with open(out_path) as f:
            data = json.load(f)
        self.assertIn("runs", data)
        self.assertEqual(len(data["runs"]), 1)

    def test_idempotent(self):
        """Running twice with same data produces identical output."""
        runs = [{"date": "2026-02-26", "commit": "abc", "bench": "2026-02-26/abc-bench.json"}]
        out_path = os.path.join(self.tmpdir, "index.json")
        write_index(runs, out_path)
        with open(out_path) as f:
            first = f.read()
        write_index(runs, out_path)
        with open(out_path) as f:
            second = f.read()
        self.assertEqual(first, second)


if __name__ == "__main__":
    unittest.main()
