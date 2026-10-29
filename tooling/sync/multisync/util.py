"""Helpers shared by the monitor and the package."""

import os
import re
from pathlib import Path


def fmt_time(secs: float) -> str:
    secs = int(abs(secs))
    h, m, s = secs // 3600, (secs % 3600) // 60, secs % 60
    return " ".join(f"{v}{u}" for v, u in [(h, "h"), (m, "m"), (s, "s")] if v or (u == "s" and not h and not m))


def load_env(path) -> None:
    """Load `KEY=value` lines from `path` into the environment.

    A variable already set in the environment wins. Blank lines, full-line
    comments and an inline ` # comment` after a value are ignored. A missing
    file is not an error.
    """
    try:
        text = Path(path).read_text()
    except OSError:
        return
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        key, sep, value = line.partition("=")
        key = key.strip()
        if not sep or not key:
            continue
        value = re.split(r"\s#", value, maxsplit=1)[0].strip()
        os.environ.setdefault(key, value)
