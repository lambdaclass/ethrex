window.BENCHMARK_DATA = {
  "lastUpdate": 1741890049060,
  "repoUrl": "https://github.com/lambdaclass/ethrex",
  "entries": {
    "Benchmark": [
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
          "id": "9cabb0961d7d0e5d1ac96306c40ec16ed1620b3a",
          "message": "feat(core): bench workflow",
          "timestamp": "2025-03-12T18:31:00Z",
          "url": "https://github.com/lambdaclass/ethrex/pull/2190/commits/9cabb0961d7d0e5d1ac96306c40ec16ed1620b3a"
        },
        "date": 1741834445313,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 230999121163,
            "range": "± 404755845",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "49622509+jrchatruc@users.noreply.github.com",
            "name": "Javier Rodríguez Chatruc",
            "username": "jrchatruc"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "cd5ddb710bfb077a0cc442437f7250f60e4897d1",
          "message": "feat(core): bench workflow (#2190)\n\n**Motivation**\n\nThis PR adds a CI workflow that runs a criterion benchmark of importing\n1000 blocks with erc20 transfers, and posts the result to gh pages, to\ntrack the performance by commit (so we can easily identify regressions).\nThis workflow runs only on pushes to `main`.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-03-13T17:22:26Z",
          "tree_id": "c1d7f35814a9ea9a64f3a316a370bc1429959c57",
          "url": "https://github.com/lambdaclass/ethrex/commit/cd5ddb710bfb077a0cc442437f7250f60e4897d1"
        },
        "date": 1741890047540,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222834821209,
            "range": "± 1241048933",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}