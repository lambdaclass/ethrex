window.BENCHMARK_DATA = {
  "lastUpdate": 1742921147302,
  "repoUrl": "https://github.com/lambdaclass/ethrex",
  "entries": {
    "Trie Benchmark": [
      {
        "commit": {
          "author": {
            "name": "lambdaclass",
            "username": "lambdaclass"
          },
          "committer": {
            "name": "lambdaclass",
            "username": "lambdaclass"
          },
          "id": "cfe86bf7a6bb01afc2633a4aca8487d814c3987a",
          "message": "perf(l1,l2): trie benchmark",
          "timestamp": "2025-03-25T13:21:08Z",
          "url": "https://github.com/lambdaclass/ethrex/pull/2272/commits/cfe86bf7a6bb01afc2633a4aca8487d814c3987a"
        },
        "date": 1742921146825,
        "tool": "cargo",
        "benches": [
          {
            "name": "ethrex-trie insert 1k",
            "value": 7953271,
            "range": "± 43322",
            "unit": "ns/iter"
          },
          {
            "name": "ethrex-trie insert 10k",
            "value": 102825719,
            "range": "± 897120",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}