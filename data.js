window.BENCHMARK_DATA = {
  "lastUpdate": 1741985650279,
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
          "distinct": true,
          "id": "40bc3df8f055f0e205e41028ea08d4192351546c",
          "message": "fix(core): fix flamegraph reporter workflow (#2221)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-03-13T18:33:10Z",
          "tree_id": "e1ff157c6435c4ebecf71922737365f437f875a8",
          "url": "https://github.com/lambdaclass/ethrex/commit/40bc3df8f055f0e205e41028ea08d4192351546c"
        },
        "date": 1741894305496,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 228890170082,
            "range": "± 2113501115",
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
          "distinct": true,
          "id": "ace63e070da474cd4fa1dc2943e8d31c01c1aa7f",
          "message": "fix(core): fix flamegraph reporter checking out github pages (#2223)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-03-13T21:31:49Z",
          "tree_id": "a1f4b7b169da04608770d28c10f639ccb85f89e0",
          "url": "https://github.com/lambdaclass/ethrex/commit/ace63e070da474cd4fa1dc2943e8d31c01c1aa7f"
        },
        "date": 1741904999705,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 223601049448,
            "range": "± 621095801",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "72628438+avilagaston9@users.noreply.github.com",
            "name": "Avila Gastón",
            "username": "avilagaston9"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "db6b5129c648c63b2dc54cc03fd807f18d9a27fd",
          "message": "feat(l2): add P256Verify precompile (#2186)\n\n**Motivation**\n\nWe want to support signature verifications using the “secp256r1”\nelliptic curve.\n\n**Description**\n\nImplements\n[RIP-7212](https://github.com/ethereum/RIPs/blob/master/RIPS/rip-7212.md),\nadding a new precompiled contract to levm. The contract is only\nactivated under the \"l2\" feature.\n\nCloses #2148",
          "timestamp": "2025-03-14T00:24:00Z",
          "tree_id": "b0b76c969a46387d059dceee423ec899f9e578b1",
          "url": "https://github.com/lambdaclass/ethrex/commit/db6b5129c648c63b2dc54cc03fd807f18d9a27fd"
        },
        "date": 1741915350936,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 225172082329,
            "range": "± 421731711",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "99273364+fmoletta@users.noreply.github.com",
            "name": "fmoletta",
            "username": "fmoletta"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "b7badba4ccec20d68f722661084c0dc08d92fa44",
          "message": "fix(l1): add prague timestamps to holesky & sepolia genesis (#2215)\n\n**Motivation**\nHolesky and Sepolia testnets have moved on to Prague but we haven't\nregistered this in their preset chain config, causing us to reject all\nnewPayloadV4 requests as we asume the block to be Cancun instead of\nPrague. This PR fixes this by adding the Prague timestamps for both\nnetworks.\nThe timestamps were taken from\n[geth](https://github.com/ethereum/go-ethereum/blob/f3e4866073d4650a7f461315c517333c6407ab5c/params/config.go#L99)",
          "timestamp": "2025-03-14T13:54:23Z",
          "tree_id": "6ced125528fccd262cdc550d3da13e3a68e1bfdc",
          "url": "https://github.com/lambdaclass/ethrex/commit/b7badba4ccec20d68f722661084c0dc08d92fa44"
        },
        "date": 1741963997637,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 225407839111,
            "range": "± 1011564156",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "tomas.orsi@lambdaclass.com",
            "name": "Tomas Fabrizio Orsi",
            "username": "lima-limon-inc"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "ca4dfc05837084100ae8049ce55b20a71fc34a2e",
          "message": "chore(l1): revert conditional docker building logic (#2196)\n\n**Motivation**\n\n#2175 introduced an additional compilation check regarding L1 client for\nthe hive tests. The check was that to avoid building the `ethrex` docker\nimage if it was not being used.\n\nThe check added additional complexity whilst not providing a lot of\nutility, since the ethrex docker image would have to be built\nregardless; since the only point of using a different L1 Client was to\ncompare against `ethrex`.\n\n**Description**\n\nRemove the if statement that provided the conditional compilation of the\n`ethrex` docker image.\n\nAlso remove an additional `HIVE_LOGLEVEL` that was not present in the\nMakefile before.\n\n---------\n\nSigned-off-by: Tomas Fabrizio Orsi <tomas.orsi@lambdaclass.com>",
          "timestamp": "2025-03-14T16:03:06Z",
          "tree_id": "1fb22211975765b37bd1497dce7fb0c486e2cd20",
          "url": "https://github.com/lambdaclass/ethrex/commit/ca4dfc05837084100ae8049ce55b20a71fc34a2e"
        },
        "date": 1741971664932,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 224507493580,
            "range": "± 468683788",
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
          "id": "892d5adb946de77d2be9586e44225ab702622e99",
          "message": "fix(core): fix slack flamegraphs link (#2228)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-03-14T16:08:02Z",
          "tree_id": "b7717f1f6bff8116efcc079adc9e3f1240c95269",
          "url": "https://github.com/lambdaclass/ethrex/commit/892d5adb946de77d2be9586e44225ab702622e99"
        },
        "date": 1741971968928,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 224225831163,
            "range": "± 1129255460",
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
          "id": "d1655468ef758690587d016a2ecd5477d883e465",
          "message": "fix(core): fix benchmark to use the CI genesis file (#2229)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number\n\nCo-authored-by: Francisco Krause Arnim <56402156+fkrause98@users.noreply.github.com>",
          "timestamp": "2025-03-14T18:34:30Z",
          "tree_id": "6435817883da697960c74920cbe8b6021e87b2fc",
          "url": "https://github.com/lambdaclass/ethrex/commit/d1655468ef758690587d016a2ecd5477d883e465"
        },
        "date": 1741980809693,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 229781371145,
            "range": "± 462190827",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "manuel.bilbao@lambdaclass.com",
            "name": "Manuel Iñaki Bilbao",
            "username": "ManuelBilbao"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "0e5bd4b7bf369c9a409324e785801b03d6c997e2",
          "message": "feat(l2): add a blobs saver command to store state diffs (#2194)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nWe need a tool to store state diffs blobs offline so the L2 state is\nreconstructable after 2 weeks, when blobs got deleted on L1.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nThis PR adds a command to the L2 CLI (`ethrex_l2 stack blobs-saver`)\nthat runs a service which continuously looks for new Commit events in\nthe `OnChainProposer` contract and downloads its blobs in the local\nfilesystem.\nIt uses a both EL and CL RPCs\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #1196\n\n---------\n\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-03-14T19:54:25Z",
          "tree_id": "42a295d873e68414c55a2c68d57893933295ee2c",
          "url": "https://github.com/lambdaclass/ethrex/commit/0e5bd4b7bf369c9a409324e785801b03d6c997e2"
        },
        "date": 1741985585803,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 225890114363,
            "range": "± 4382620813",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "67517699+ilitteri@users.noreply.github.com",
            "name": "Ivan Litteri",
            "username": "ilitteri"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "567d32c9a623307a3ed0f513667953a467f7fdde",
          "message": "feat(core): add `p2p.enabled` flag (#2230)\n\n**Motivation**\n\nIn most of the L2 use cases we want to disable de P2P network.\n\n**Description**\n\nAdd a `p2p.enabled` flag for users to explicit whether they want to\nenable the P2P in their node.\n\nIt is enabled by default in the L1 and disabled by default for the L2.",
          "timestamp": "2025-03-14T19:55:10Z",
          "tree_id": "cee66b1dc342d7c8999f4bf6d5c4cdef69f3c46c",
          "url": "https://github.com/lambdaclass/ethrex/commit/567d32c9a623307a3ed0f513667953a467f7fdde"
        },
        "date": 1741985648213,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 227042419601,
            "range": "± 1019661927",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}