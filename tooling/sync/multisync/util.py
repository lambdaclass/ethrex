"""Helpers shared by the monitor and the package."""


def fmt_time(secs: float) -> str:
    secs = int(abs(secs))
    h, m, s = secs // 3600, (secs % 3600) // 60, secs % 60
    return " ".join(f"{v}{u}" for v, u in [(h, "h"), (m, "m"), (s, "s")] if v or (u == "s" and not h and not m))
