import pandas as pd
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
import argparse
import os

parser = argparse.ArgumentParser()
parser.add_argument("file")

file_path = parser.parse_args().file

df = pd.read_csv(file_path, sep="\s*\|\s*", engine="python", header=None, names=["time", "function", "elapsed"])

df["time"] = pd.to_numeric(df["time"], errors="coerce")
df["elapsed"] = pd.to_numeric(df["elapsed"], errors="coerce")

df = df[df["time"] >= 750] 
df = df[df["time"] <= 2250] 

df_grouped = df.groupby(["time", "function"], as_index=False)["elapsed"].sum()

functions = df_grouped["function"].unique()

fig, axes = plt.subplots(len(functions), 1, figsize=(14, 15 * len(functions)))

if len(functions) == 1:
    axes = [axes]

for ax, func in zip(axes, functions):
    func_data = df_grouped[df_grouped["function"] == func]
    ax.plot(func_data["time"], func_data["elapsed"], marker="o", linestyle="-")
    ax.set_title(func)
    ax.set_ylabel("Elapsed time")
    ax.grid(True)
    ax.xaxis.set_major_locator(ticker.MaxNLocator(integer=True))
    ax.tick_params(axis='x', rotation=45)  

axes[-1].set_xlabel("Time since start (s)")

plt.tight_layout()
os.makedirs("perf_logs_plots", exist_ok=True)
output_file = os.path.join("perf_logs_plots", os.path.splitext(os.path.basename(file_path))[0] + "_plot.png")
plt.savefig(output_file)
print(f"Plot saved to {output_file}")
