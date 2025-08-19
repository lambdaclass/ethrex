import pandas as pd
import matplotlib.pyplot as plt
import numpy as np
import sys
import os

LOG_FILE = sys.argv[1]
OUTPUT_FILE = sys.argv[2] if len(sys.argv) >= 3 else "count_values.csv"

cols = ["seconds", "label", "value", "value_type", "timestamp"]

df = pd.read_csv(LOG_FILE, sep="|", names=cols, engine="python")
df = df.apply(lambda x: x.str.strip() if x.dtype == "object" else x)
df["seconds"] = pd.to_numeric(df["seconds"], errors="coerce")

df_target = df[(df["value_type"] == "Count")]

counts = (
    df_target.groupby(["timestamp", "label"])
    .agg(count=("label", "size"), seconds=("seconds", "first"))
    .reset_index()
)

counts.to_csv(OUTPUT_FILE, index=False)
print(f"Saved counts to {OUTPUT_FILE}")

labels = counts["label"].unique()
n_labels = len(labels)

fig, axes = plt.subplots(n_labels, 1, figsize=(12, 4 * n_labels), sharex=False)

if n_labels == 1:
    axes = [axes]  # make iterable if only one subplot

for ax, label in zip(axes, labels):
    data = counts[counts["label"] == label]

    ax.scatter(data["seconds"], data["count"], label="Actual counts")

    if len(data) >= 4:
        x = data["seconds"].values
        z = np.polyfit(x, data["count"], 3)
        p = np.poly1d(z)
        ax.plot(data["seconds"], p(x), label="Estimated trend", linestyle="--")

    ax.set_title(f"Label: {label}")
    ax.set_ylabel("Count")
    ax.set_xlabel("Seconds since program start")

    ax.set_xticks(data["seconds"].unique())

    ax.legend()

plt.tight_layout()
plt.savefig("rpc_call_counts.png")
print(f"Saved plots to rpc_call_counts.png")
