import numpy as np

bench = {}
bench_around = {}

for i in range(1,4):
    bench_around[i] = {}

    with open(f"../../bench-{i}.log", "r+") as file:
        for line in file:
            if "Finished regenerating state":
                break
        
        for line in file:
            if "[METRIC]" in line:
                block_num = line.split(")")[0][-7:]
                ggas = line.split(")")[1][2:7]
                
                if block_num not in bench:
                    bench[block_num] = {}
                bench[block_num][i] = float(ggas)
                bench_around[i][block_num] = float(ggas)

total = 0
count = 0
for block in bench.values():
    for ggas in block.values():
        total += ggas
        count += 1
        
    
print(len(bench))
print("Mean ggas accross multiple runs:", total/count)
for run in bench_around.values():
    print(sum(run.values())/ len(run.values()))
