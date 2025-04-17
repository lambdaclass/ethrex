import re
import plotly.graph_objects as go

LOG_FILE = "logs.txt"

time_spent_regex = re.compile(r"TIME SPENT: (\d+) msecs")
throughput_regex = re.compile(r"BLOCK EXECUTION THROUGHPUT: ([\d.]+) Gigagas/s")

block_times = []
throughputs = []
block_numbers = []

with open(LOG_FILE, "r") as file:
    for line in file:
        time_match = time_spent_regex.search(line)
        throughput_match = throughput_regex.search(line)

        if time_match and throughput_match:
            time_spent = int(time_match.group(1))
            throughput = float(throughput_match.group(1))

            block_times.append(time_spent)
            throughputs.append(throughput)
            block_numbers.append(len(block_numbers) + 1)

fig = go.Figure()

# Add scatter plot for block generation time (left y-axis)
fig.add_trace(go.Scatter(
    x=block_numbers,
    y=block_times,
    mode="markers+lines",
    name="Block Generation Time (ms)",
    marker=dict(color="blue"),
    yaxis="y1"  # Associate with the left y-axis
))

# Add scatter plot for throughput (right y-axis)
fig.add_trace(go.Scatter(
    x=block_numbers,
    y=throughputs,
    mode="markers+lines",
    name="Throughput (Gigagas/s)",
    marker=dict(color="orange"),
    yaxis="y2"  # Associate with the right y-axis
))

# Update layout to include two y-axes
fig.update_layout(
    title="Block Metrics: Generation Time and Throughput",
    xaxis_title="Block Number",
    yaxis=dict(
        title="Time (ms)",
        tickfont=dict(color="blue"),
        side="left"
    ),
    yaxis2=dict(
        title="Throughput (Gigagas/s)",
        tickfont=dict(color="orange"),
        overlaying="y",  # Overlay on the same x-axis
        side="right"
    ),
    template="plotly_white"
)

# Save the interactive chart as an HTML file
output_file = "load_test_analysis.html"
fig.write_html(output_file)

print(f"Interactive chart saved as '{output_file}'. Open this file in your browser to view it.")