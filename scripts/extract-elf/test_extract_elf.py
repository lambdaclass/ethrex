#!/usr/bin/env python3
"""
Tests for extract-elf.py

Run with:
    python3 -m pytest scripts/extract-elf/test_extract_elf.py -v
    # or
    python3 scripts/extract-elf/test_extract_elf.py
"""

import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

# Path to the script being tested
SCRIPT_PATH = Path(__file__).parent / "extract-elf.py"

# ELF magic bytes
ELF_MAGIC = b"\x7fELF"


def create_ere_program(elf_content: bytes) -> bytes:
    """Create an ere Program format file from ELF content."""
    # ere format: 8 bytes u64 LE length + ELF bytes + optional metadata
    length = struct.pack("<Q", len(elf_content))
    return length + elf_content


def create_valid_elf(size: int = 100) -> bytes:
    """Create a minimal valid ELF-like content (just magic + padding)."""
    return ELF_MAGIC + b"\x00" * (size - 4)


class TestExtractElf(unittest.TestCase):
    """Test cases for extract-elf.py script."""

    def run_script(self, input_path: str, output_path: str) -> subprocess.CompletedProcess:
        """Run the extract-elf.py script and return the result."""
        return subprocess.run(
            [sys.executable, str(SCRIPT_PATH), input_path, output_path],
            capture_output=True,
            text=True,
        )

    def test_valid_elf_extraction(self):
        """Test extraction of a valid ELF from ere Program format."""
        elf_content = create_valid_elf(256)
        ere_data = create_ere_program(elf_content)

        with tempfile.NamedTemporaryFile(delete=False) as input_file:
            input_file.write(ere_data)
            input_path = input_file.name

        with tempfile.NamedTemporaryFile(delete=False) as output_file:
            output_path = output_file.name

        try:
            result = self.run_script(input_path, output_path)

            self.assertEqual(result.returncode, 0, f"Script failed: {result.stderr}")
            self.assertIn("Extracted ELF", result.stdout)
            self.assertIn("256 bytes", result.stdout)

            # Verify output file content
            with open(output_path, "rb") as f:
                extracted = f.read()
            self.assertEqual(extracted, elf_content)
        finally:
            Path(input_path).unlink(missing_ok=True)
            Path(output_path).unlink(missing_ok=True)

    def test_elf_with_metadata(self):
        """Test extraction when ere Program has trailing metadata."""
        elf_content = create_valid_elf(128)
        metadata = b"some metadata that should be ignored" * 10
        ere_data = create_ere_program(elf_content) + metadata

        with tempfile.NamedTemporaryFile(delete=False) as input_file:
            input_file.write(ere_data)
            input_path = input_file.name

        with tempfile.NamedTemporaryFile(delete=False) as output_file:
            output_path = output_file.name

        try:
            result = self.run_script(input_path, output_path)

            self.assertEqual(result.returncode, 0, f"Script failed: {result.stderr}")

            # Verify only ELF was extracted, not metadata
            with open(output_path, "rb") as f:
                extracted = f.read()
            self.assertEqual(extracted, elf_content)
            self.assertEqual(len(extracted), 128)
        finally:
            Path(input_path).unlink(missing_ok=True)
            Path(output_path).unlink(missing_ok=True)

    def test_invalid_elf_magic(self):
        """Test that invalid ELF magic causes failure."""
        # Create content without ELF magic
        invalid_content = b"NOT_ELF_" + b"\x00" * 100
        ere_data = create_ere_program(invalid_content)

        with tempfile.NamedTemporaryFile(delete=False) as input_file:
            input_file.write(ere_data)
            input_path = input_file.name

        with tempfile.NamedTemporaryFile(delete=False) as output_file:
            output_path = output_file.name

        try:
            result = self.run_script(input_path, output_path)

            self.assertEqual(result.returncode, 1)
            self.assertIn("ELF magic", result.stderr)
        finally:
            Path(input_path).unlink(missing_ok=True)
            Path(output_path).unlink(missing_ok=True)

    def test_file_too_small(self):
        """Test that files smaller than 8 bytes cause failure."""
        # Only 5 bytes - too small for the length header
        small_data = b"\x00\x01\x02\x03\x04"

        with tempfile.NamedTemporaryFile(delete=False) as input_file:
            input_file.write(small_data)
            input_path = input_file.name

        with tempfile.NamedTemporaryFile(delete=False) as output_file:
            output_path = output_file.name

        try:
            result = self.run_script(input_path, output_path)

            self.assertEqual(result.returncode, 1)
            self.assertIn("too small", result.stderr)
            self.assertIn("5 bytes", result.stderr)
        finally:
            Path(input_path).unlink(missing_ok=True)
            Path(output_path).unlink(missing_ok=True)

    def test_elf_length_exceeds_data(self):
        """Test that ELF length exceeding available data causes failure."""
        # Header says 1000 bytes, but only 50 bytes of data follow
        bad_length = struct.pack("<Q", 1000)
        bad_data = bad_length + ELF_MAGIC + b"\x00" * 46  # 8 + 50 = 58 total

        with tempfile.NamedTemporaryFile(delete=False) as input_file:
            input_file.write(bad_data)
            input_path = input_file.name

        with tempfile.NamedTemporaryFile(delete=False) as output_file:
            output_path = output_file.name

        try:
            result = self.run_script(input_path, output_path)

            self.assertEqual(result.returncode, 1)
            self.assertIn("exceeds available data", result.stderr)
        finally:
            Path(input_path).unlink(missing_ok=True)
            Path(output_path).unlink(missing_ok=True)

    def test_file_not_found(self):
        """Test that non-existent input file causes failure."""
        with tempfile.NamedTemporaryFile(delete=False) as output_file:
            output_path = output_file.name

        try:
            result = self.run_script("/nonexistent/path/to/file", output_path)

            self.assertEqual(result.returncode, 1)
            self.assertIn("not found", result.stderr)
        finally:
            Path(output_path).unlink(missing_ok=True)

    def test_wrong_number_of_arguments(self):
        """Test that wrong number of arguments shows usage."""
        # No arguments
        result = subprocess.run(
            [sys.executable, str(SCRIPT_PATH)],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("Usage", result.stdout)

        # One argument
        result = subprocess.run(
            [sys.executable, str(SCRIPT_PATH), "input_only"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("Usage", result.stdout)

        # Three arguments
        result = subprocess.run(
            [sys.executable, str(SCRIPT_PATH), "arg1", "arg2", "arg3"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("Usage", result.stdout)

    def test_empty_elf(self):
        """Test that zero-length ELF causes failure (no magic to verify)."""
        # Length header says 0 bytes
        ere_data = struct.pack("<Q", 0)

        with tempfile.NamedTemporaryFile(delete=False) as input_file:
            input_file.write(ere_data)
            input_path = input_file.name

        with tempfile.NamedTemporaryFile(delete=False) as output_file:
            output_path = output_file.name

        try:
            result = self.run_script(input_path, output_path)

            # Empty ELF can't have magic, so should fail
            self.assertEqual(result.returncode, 1)
            self.assertIn("ELF magic", result.stderr)
        finally:
            Path(input_path).unlink(missing_ok=True)
            Path(output_path).unlink(missing_ok=True)

    def test_minimal_valid_elf(self):
        """Test extraction of minimal valid ELF (just 4 magic bytes)."""
        elf_content = ELF_MAGIC  # Just the magic, nothing else
        ere_data = create_ere_program(elf_content)

        with tempfile.NamedTemporaryFile(delete=False) as input_file:
            input_file.write(ere_data)
            input_path = input_file.name

        with tempfile.NamedTemporaryFile(delete=False) as output_file:
            output_path = output_file.name

        try:
            result = self.run_script(input_path, output_path)

            self.assertEqual(result.returncode, 0, f"Script failed: {result.stderr}")
            self.assertIn("4 bytes", result.stdout)

            with open(output_path, "rb") as f:
                extracted = f.read()
            self.assertEqual(extracted, ELF_MAGIC)
        finally:
            Path(input_path).unlink(missing_ok=True)
            Path(output_path).unlink(missing_ok=True)


if __name__ == "__main__":
    unittest.main()
