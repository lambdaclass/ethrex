#!/usr/bin/env python3
import subprocess, time, sys, os

KEYFILE = os.getenv("KEYFILE", "./fixtures/keys/private_keys_l1.txt")
N = int(os.getenv("N", 1))
DEST = os.getenv("DEST", "0x8943545177806ed17b9f23f0a21ee5948ecaa776")

# --- load and clean keys ---
with open(KEYFILE) as f:
    keys = [line.strip() for line in f if line.strip()]


def chunks(lst, n):
    """Yield successive n-sized chunks from lst."""
    for i in range(0, len(lst), n):
        yield lst[i : i + n]


while True:
    for batch in chunks(keys, N):
        procs = []
        for key in batch:
            print(f"Sending from {key} to {DEST}")
            # Launch rex command in background
            p = subprocess.Popen(
                ["rex", "send", "--value", "1", DEST],
                env={**os.environ, "PRIVATE_KEY": key},
            )
            procs.append(p)

        time.sleep(1.0)
