import sys

bench = {}
bench_around = {}

for file_path in sys.argv[1:]:
    bench_around[file_path] = {}

    with open(file_path, "r") as file:
        for line in file:
            if "Finished regenerating state" in line:
                break

        for line in file:
            if "[METRIC]" in line:
                block_num = line.split(")")[0][-7:]
                ggas = line.split(")")[1][2:7]

                if block_num not in bench:
                    bench[block_num] = {}
                bench[block_num][file_path] = float(ggas)
                bench_around[file_path][block_num] = float(ggas)

total = 0
count = 0
for block in bench.values():
    for ggas in block.values():
        total += ggas
        count += 1


print("Blocks tested", len(bench))
print("Mean ggas across multiple runs:", total / count)
for run_count, run in bench_around.items():
    print("Mean ggas in run:", run_count, sum(run.values()) / len(run.values()))

average_difference = []
for block_num, block in bench.items():
    average_difference.append(max(block.values()) - min(block.values()))
    pass
print("Mean ggas spread across blocks:", sum(average_difference) / count)
