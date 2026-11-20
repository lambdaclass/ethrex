import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from multisync.util import load_env


class LoadEnvTest(unittest.TestCase):
    def load(self, text, env=None):
        with tempfile.TemporaryDirectory() as d, mock.patch.dict(os.environ, env or {}, clear=True):
            path = Path(d) / ".env"
            path.write_text(text)
            load_env(path)
            return dict(os.environ)

    def test_reads_values_and_skips_blank_and_comment_lines(self):
        env = self.load("\n# a comment\nFOO=1\n  BAR = two \n")
        self.assertEqual((env["FOO"], env["BAR"]), ("1", "two"))

    def test_inline_comment_is_not_part_of_the_value(self):
        env = self.load("CL_STARTUP_TIMEOUT=900   # fail if slow\nURL=http://h/#frag\n")
        self.assertEqual(env["CL_STARTUP_TIMEOUT"], "900")
        self.assertEqual(env["URL"], "http://h/#frag")

    def test_does_not_override_the_environment(self):
        env = self.load("FOO=file\n", {"FOO": "env"})
        self.assertEqual(env["FOO"], "env")

    def test_missing_file_is_fine(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            load_env("/nonexistent/.env")


if __name__ == "__main__":
    unittest.main()
