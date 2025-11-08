import subprocess
#23747835
for i in range(23747336, 23747337):
    subprocess.run(
        ["make", "create-single-chain-rlp", "NETWORK=mainnet", f"BLOCK_NUM={i}"],
    )
    subprocess.run(
        ["make", "move-bench-forward", "NETWORK=mainnet", f"BLOCK_NUM={i}"],
    )

    for j in range(0, 3):
        subprocess.run(
            ["make", "run-regenerate-bench", "NETWORK=mainnet", f"BLOCK_NUM={i}", f"BENCH_NUM={j}"],
        )
