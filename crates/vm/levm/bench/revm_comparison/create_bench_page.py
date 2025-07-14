import json
import os

print("""<!DOCTYPE html>
<html lang="en">
 <head>
  <meta charset="utf-8"/>
  <meta content="width=device-width, initial-scale=1.0" name="viewport"/>
  <title>
   Benchmarking Report
  </title>
  <style>
   body {            font-family: Arial, sans-serif;        }        table {            border-collapse: collapse;            margin-bottom: 20px;        }        th, td {            border: 1px solid #ddd;            padding: 8px;            text-align: center;        }        th {            background-color: #f2f2f2;        }        .title {            text-align: left;        }        .preserve-newlines {            white-space: pre-wrap;        }
  </style>
 </head>
<body>""")

print("<h1>Ethrex LEVM benchmarks</h1>")

print("""<a href="https://github.com/lambdaclass/ethrex_benchmarks">https://github.com/lambdaclass/ethrex_benchmarks</a>""")
print("""</br>""")
print("""</br>""")

for root, dirs, files in os.walk("./bench-results"):
    for file in files:
        if file.endswith(".json"):
            file = f"./bench-results/{file}"
            with open(file, "r") as f:
                data = json.load(f)
                print("""
<table id="table_ethrex">
   <thread>
      <tr>
        <th>Name</th>
        <th>Mean</th>
        <th>Median</th>
        <th>Stddev</th>
        <th>Min</th>
        <th>Max</th>
        <th>Difference (mean, as levm base)</th>
      </tr>
    </thread>
    <tbody>
""")
                first_mean = -1
                for row in data["results"]:
                    if first_mean == -1:
                        first_mean = row["mean"]
                    row["change"] = (row["mean"] / first_mean) * 100
                for row in data["results"]:
                    change_text = ""

                    if row["change"] != 100.0:
                      if row["change"] < 100.0:
                          value = (100.0 - row["change"]) / 100
                          change_text = f"""(<span style="color: green;">{value:.2f} times faster</span>)"""
                      else:
                          value = (row["change"] - 100.0) / 100
                          change_text = f"""(<span style="color: red;">{value:.2f} times slower</span>)"""
                    print(f"""
                          <tr>
                            <td class="title">{row["command"]}</td>
                            <td>{row["mean"]  * 1000:.2f} ms</td>
                            <td>{row["median"] * 1000:.2f} ms</td>
                            <td>{row["stddev"] * 1000:.2f} ms</td>
                            <td>{row["min"] * 1000:.2f} ms</td>
                            <td>{row["max"] * 1000:.2f} ms</td>
                            <td>{row["change"]:.2f} % {change_text}</td>
                          </tr>
                        """)
                print("""
                </tbody>
                """)

print("""</body>""")
print("</html>")
