window.BENCHMARK_DATA = {
  "lastUpdate": 1743697310220,
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
      },
      {
        "commit": {
          "author": {
            "email": "156438142+fborello-lambda@users.noreply.github.com",
            "name": "Federico Borello",
            "username": "fborello-lambda"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "9f0148fba23984175609aee1eb4acdb54b09e390",
          "message": "feat(levm): pectra-devnet6 eftests (#1877)\n\n**Motivation**\n\nThe latest test for pectra has been released. [Pectra Devnet 6\n](https://github.com/ethereum/execution-spec-tests/releases/tag/pectra-devnet-6%40v1.0.0)\n\n**Description**\n\n- Download latest tests\n- Add eip7702 latest changes: https://github.com/ethereum/EIPs/pull/9248\n- Add a new CI rule for EF Tests to be 100% from London to Prague forks\n- Fix tests from `set_code_txs_2` and `refunds` belonging to EIP 7702\nand EIP 7623\n\n---------\n\nCo-authored-by: Javier Chatruc <jrchatruc@gmail.com>\nCo-authored-by: Tomás Paradelo <tomas.paradelo@lambdaclass.com>",
          "timestamp": "2025-03-17T14:25:36Z",
          "tree_id": "c2996d7b99e333fabd413e652fa751a2f65d0af0",
          "url": "https://github.com/lambdaclass/ethrex/commit/9f0148fba23984175609aee1eb4acdb54b09e390"
        },
        "date": 1742225078844,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 225391645830,
            "range": "± 1168069964",
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
          "id": "35d3462d07ec1e0f224280c3a4dd81097e5de66e",
          "message": "feat(l1): enforce deposit contract address (#2118)\n\n**Motivation**\n\nTo avoid issues mixing the `DepositContractAddress` across different\nnetworks.\n\n**Description**\n\n- Enforce setting `deposit_contract_address` from the genesis files.\n- Remove the `MAINNET_DEPOSIT_CONTRACT_ADDRESS` constant.\n- Update unit tests to use a mock address.\n- Update the `network_params` files to include the\n`deposit_contract_address`.\n- Add the correct `deposit_contract_address` for Holesky.\n\nCloses #2082",
          "timestamp": "2025-03-17T15:08:43Z",
          "tree_id": "879e0564990ee0e55493f7a7dac4b069d21a3cd8",
          "url": "https://github.com/lambdaclass/ethrex/commit/35d3462d07ec1e0f224280c3a4dd81097e5de66e"
        },
        "date": 1742227628303,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 225727773861,
            "range": "± 535842569",
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
          "id": "76d3ee9afa428b5cf8869b00f8a2f4b7d5c119ca",
          "message": "feat(l1): remove deprecated mekong testnet (#2243)\n\n**Motivation**\n\nMekong testnet has been [officially\ndeprecated](https://blog.ethereum.org/en/2025/03/06/mekong-devnet)\n\n**Description**\n\nRemove `mekong` as a preset network option and remove associated data\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-03-17T16:01:55Z",
          "tree_id": "61102444d122a2c8c6e8fdd5c9b40242be72c332",
          "url": "https://github.com/lambdaclass/ethrex/commit/76d3ee9afa428b5cf8869b00f8a2f4b7d5c119ca"
        },
        "date": 1742230945110,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 234091245744,
            "range": "± 2223166116",
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
          "id": "39c8e480507d5a5e6f5d12c61e41eba3f1036462",
          "message": "fix(l2): small fixes and refactors (#2241)\n\n**Motivation**\n\nThis PR makes a few fixes and changes to ethrex l2:\n\n- Moves the block building logic to a separate file/task called\n`block_producer`, more in line with our current vocabulary.\n- Fixes an issue where the prover server in dev mode would wait using\n`thread::sleep` instead of `tokio::time::sleep`, sometimes hanging the\nruntime.\n- Adds a `dev_interval_ms` config option to the prover server to\nconfigure, in dev mode, how often it sends (empty) proofs to the L1.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number\n\n---------\n\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-03-17T16:26:33Z",
          "tree_id": "181c54b5ba2d5abaf5d00444fad8294bb43290a8",
          "url": "https://github.com/lambdaclass/ethrex/commit/39c8e480507d5a5e6f5d12c61e41eba3f1036462"
        },
        "date": 1742232347655,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 228869982757,
            "range": "± 840452460",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "martin.c.paulucci@gmail.com",
            "name": "Martin Paulucci",
            "username": "mpaulucci"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "a07f74ffd7086e94c0cf8da04e5fd9eed6bb2450",
          "message": "ci(l1): refine posting daily reports to slack. (#2170)\n\n**Motivation**\nRemove posting of some reports to L1 channel.",
          "timestamp": "2025-03-17T16:29:02Z",
          "tree_id": "76478a9c0cfb8097bb8a9481b087d3c5ff6f558b",
          "url": "https://github.com/lambdaclass/ethrex/commit/a07f74ffd7086e94c0cf8da04e5fd9eed6bb2450"
        },
        "date": 1742232433382,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 224568254546,
            "range": "± 376027366",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "mrugiero@gmail.com",
            "name": "Mario Rugiero",
            "username": "Oppen"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "58443717c6f5dd2b1d434af1c1483ea814f9ef35",
          "message": "refactor(levm): simplify fill_with_zeros (#2226)\n\nReviewing #2186 I noticed we had this helper that returned a `Result`\nfor a logically impossible situation (already covered by an `if` just\nabove it).\nI removed that `Result` and also simplified the logic by just calling\n`resize` in the appropriate case.",
          "timestamp": "2025-03-17T18:56:32Z",
          "tree_id": "8a20738b3cc89eaa2d1dcc8b091ef3315d63b739",
          "url": "https://github.com/lambdaclass/ethrex/commit/58443717c6f5dd2b1d434af1c1483ea814f9ef35"
        },
        "date": 1742241293769,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 226462891870,
            "range": "± 852500902",
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
          "id": "eb0629cb88e754da18dbc279f3b545f6ac0cf047",
          "message": "docs(core): remove milestones and cleanup readme (#2248)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-03-17T18:58:20Z",
          "tree_id": "bdabf9ad88f96c6f410801f6f64b6c70edba2df2",
          "url": "https://github.com/lambdaclass/ethrex/commit/eb0629cb88e754da18dbc279f3b545f6ac0cf047"
        },
        "date": 1742241370039,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 223547007715,
            "range": "± 625519400",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "estefano.bargas@fing.edu.uy",
            "name": "Estéfano Bargas",
            "username": "xqft"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "104ef77ae137b6ee14de6945c6c49d223e735ba6",
          "message": "feat(l2): pico prover (#1922)\n\n**Motivation**\n\nAdds [Pico](https://pico-docs.brevis.network/) as a prover backend. \n\nAlso does a major refactor to remove the need to have multiple zkvm\ndependencies compiling at the same time, this is because Pico doesn't\ncompile while also having Risc0 as dependency; the linker fails with a\n\"duplicated symbols\" error.\n\nAlso removes zkvm dependencies from crates that don't need them by\ndecoupling return types. This is because Pico compiles with nightly only\nand we want to minimize the number of crates that depend on it (now only\n`ethrex-prover` and `zkvm_interface` does)\n\n**Description**\n\n- adds pico as prover backend\n- decouples zkvm dependencies from other L2 crates by doing a major\nrefactor of provers\n- makes it so you can compile the prover client with only one backend at\na time\n- makes the prover client return the proof calldata to send to the L1\nbridge contract instead of the proofs using each custom type of every\nzkvm\n\n---------\n\nCo-authored-by: Mario Rugiero <mrugiero@gmail.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>",
          "timestamp": "2025-03-17T19:18:17Z",
          "tree_id": "0f62ae1a5aeb11269b1501121a2e71fcefa29667",
          "url": "https://github.com/lambdaclass/ethrex/commit/104ef77ae137b6ee14de6945c6c49d223e735ba6"
        },
        "date": 1742242535817,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 227682387813,
            "range": "± 938087967",
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
          "distinct": false,
          "id": "1c3cb981e7770e532330133292d13a5fc657ce61",
          "message": "feat(levm): implement simulate_tx (#2232)\n\n**Motivation**\n\nTo implement the remaining RPC endpoints.\n\n**Description**\n\n- Implements `simulate_tx_from_generic` for LEVM.\n- If `gas_price` is not specified, sets `env.base_fee_per_gas =\nU256::zero()` to avoid base fee checks.\n- Moves `ExecutionResult` to `vm/backends` to be used by both REVM and\nLEVM.\n\nWith this PR, only the `rpc/eth_createAccessList` tests are failing.\n\nCloses #2182",
          "timestamp": "2025-03-18T14:17:30Z",
          "tree_id": "56ae1dbf03a0cfff8919bf8bfd6c98bf6b0104aa",
          "url": "https://github.com/lambdaclass/ethrex/commit/1c3cb981e7770e532330133292d13a5fc657ce61"
        },
        "date": 1742310920611,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 231242084441,
            "range": "± 1377277024",
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
          "id": "87fc76a74cda059d2bcf25172d26d69bd9fcd8e8",
          "message": "chore(core): improve double genesis block run error (#2252)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\nIt is not obvious how to mitigate this error for users who find\nthemselves having this error. Having a better better error comment might\nhelp with this.",
          "timestamp": "2025-03-18T16:24:26Z",
          "tree_id": "7189f240dfe1af7133b042b74fdc45bf61230440",
          "url": "https://github.com/lambdaclass/ethrex/commit/87fc76a74cda059d2bcf25172d26d69bd9fcd8e8"
        },
        "date": 1742318437363,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 228403691215,
            "range": "± 1044381859",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "56402156+fkrause98@users.noreply.github.com",
            "name": "Francisco Krause Arnim",
            "username": "fkrause98"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "8c363aad60f4af75c1756cac6ad42368475b9a56",
          "message": "ci(l1,l2): always compare with main (#2253)\n\n**Motivation**\n\nThis is useful to always compare changes with main, regardless of the\nbranch.",
          "timestamp": "2025-03-18T18:44:46Z",
          "tree_id": "af8acb1009ed004fadaf3f475b70ea1a6e3f29dd",
          "url": "https://github.com/lambdaclass/ethrex/commit/8c363aad60f4af75c1756cac6ad42368475b9a56"
        },
        "date": 1742326816205,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 228981095719,
            "range": "± 1067523608",
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
          "distinct": true,
          "id": "5264f986a96ca89cda8e9436195a008ee50940a9",
          "message": "chore(l2): remove db when restarting (#2257)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n`make restart` should mean \"having initialized the network previously,\nstart over from scratch\".\n\nIn reality, this wasn't happening since both the L1 and L2 databases\nwere not being restarted.\n\n**Description**\n\nRestart L1 and L2 dbs when doing `make restart`.",
          "timestamp": "2025-03-18T23:10:02Z",
          "tree_id": "99d8dee59da3f19c983f3d05c22bfadcf5545dbd",
          "url": "https://github.com/lambdaclass/ethrex/commit/5264f986a96ca89cda8e9436195a008ee50940a9"
        },
        "date": 1742342953217,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 229927631621,
            "range": "± 1713766248",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "46695152+LeanSerra@users.noreply.github.com",
            "name": "LeanSerra",
            "username": "LeanSerra"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "0b51b10a9623159780641a0d1e35a4c4a788b952",
          "message": "feat(l2): sponsored transaction endpoints (#2214)\n\n**Motivation**\n\nWe want to add a new rpc endpoint that sponsors eip-7702 and eip-1559\nthat calls to addresses that are delegated to whitelisted contracts.\n\n**Description**\n\n- Add new namespace `ethrex` to rpc crate\n- Add feature \"l2\" rpc crate\n- Add new flag to ethrex cmd to provide a file with addresses for\ncontracts we want to sponsor txs to\n- Add new endpoint `ethrex_SendTransaction` that sponsor txs that are\n  - EIP-7702 tx with access list delegating to a whitelisted contract\n- EIP-1559 tx that call to an address that is delegated (starts with\n0xef0100) and the address that it delegates to is whitelisted\n  - Create transactions (to=0x0) are not allowed\n\n**Resources**\nhttps://ithaca.xyz/updates/exp-0000\nhttps://github.com/ithacaxyz/odyssey\nhttps://eips.ethereum.org/EIPS/eip-7702\n\n---------\n\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-03-19T15:37:48Z",
          "tree_id": "fd307c9d44d5b6801ac6200f63acb42ff9eb6c27",
          "url": "https://github.com/lambdaclass/ethrex/commit/0b51b10a9623159780641a0d1e35a4c4a788b952"
        },
        "date": 1742402191776,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 229899677477,
            "range": "± 744277514",
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
          "id": "f1693f5490035e9244fac5e365792bf7830daa9c",
          "message": "refactor(core): ethrex cli (#2240)\n\n**Motivation**\n\nTo improve `ethrex`'s CLI readability and extensibility.\n\n**Description**\n\nThis PR refactors de CLI to use clap derive instead of clap builder\napproach. Using the latter suited perfectly for the first version but as\nwe keep adding flags/args and subcommands, using the the first is better\nfor readability and also extensibility.\n\nIn the new design, the CLI is modeled as the struct `CLI` as follows:\n\n```Rust\npub struct CLI {\n    #[clap(flatten)]\n    pub opts: Options,\n    #[cfg(feature = \"based\")]\n    #[clap(flatten)]\n    pub based_opts: BasedOptions,\n    #[command(subcommand)]\n    pub command: Option<Subcommand>,\n}\n```\n\nwhere `opts` are the flags corresponding to `ethrex` common usage,\n`based_opts` are the flags needed when running `ethrex` with the `based`\nfeature, and `command` is an enum containing the subcommands\n(`removedb`, and `import` for now) which is optional.\n\nIf you'd want to add a new subcommand, simply add it to the `Subcommand`\nenum and implement its handler in the `Subcommand::run` `match`.\n\nThe CLI args are contained in `Options` and `BasedOptions`. Adding a new\nflag/arg would mean to add a field on the corresponding struct, and if\nyou want for example to add flags/args for the L2 feature it'd be good\nfor you to create an `L2Options` struct with them. The\n`#[clap(flatten)]` basically \"unpacks\" the struct fields (args and\nflags) for the CLI.\n\nRunning `cargo run --release --bin ethrex -- --help` displays:\n\n```Shell\nUsage: ethrex [OPTIONS] [COMMAND]\n\nCommands:\n  removedb  Remove the database\n  import    Import blocks to the database\n  help      Print this message or the help of the given subcommand(s)\n\nOptions:\n  -h, --help\n          Print help (see a summary with '-h')\n\n  -V, --version\n          Print version\n\nRPC options:\n      --http.addr <ADDRESS>\n          Listening address for the http rpc server.\n\n          [default: localhost]\n\n      --http.port <PORT>\n          Listening port for the http rpc server.\n\n          [default: 8545]\n\n      --authrpc.addr <ADDRESS>\n          Listening address for the authenticated rpc server.\n\n          [default: localhost]\n\n      --authrpc.port <PORT>\n          Listening port for the authenticated rpc server.\n\n          [default: 8551]\n\n      --authrpc.jwtsecret <JWTSECRET_PATH>\n          Receives the jwt secret used for authenticated rpc requests.\n\n          [default: jwt.hex]\n\nNode options:\n      --log.level <LOG_LEVEL>\n          Possible values: info, debug, trace, warn, error\n\n          [default: INFO]\n\n      --network <GENESIS_FILE_PATH>\n          Alternatively, the name of a known network can be provided instead to use its preset genesis file and include its preset bootnodes. The networks currently supported include Holesky, Sepolia and Mekong.\n\n      --datadir <DATABASE_DIRECTORY>\n          If the datadir is the word `memory`, ethrex will use the `InMemory Engine`.\n\n          [default: ethrex]\n\n      --metrics.port <PROMETHEUS_METRICS_PORT>\n\n\n      --dev\n          If set it will be considered as `true`. The Binary has to be built with the `dev` feature enabled.\n\n      --evm <EVM_BACKEND>\n          Has to be `levm` or `revm`\n\n          [default: revm]\n\nP2P options:\n      --p2p.enabled\n\n\n      --p2p.addr <ADDRESS>\n          [default: 0.0.0.0]\n\n      --p2p.port <PORT>\n          [default: 30303]\n\n      --discovery.addr <ADDRESS>\n          UDP address for P2P discovery.\n\n          [default: 0.0.0.0]\n\n      --discovery.port <PORT>\n          UDP port for P2P discovery.\n\n          [default: 30303]\n\n      --bootnodes <BOOTNODE_LIST>...\n          Comma separated enode URLs for P2P discovery bootstrap.\n\n      --syncmode <SYNC_MODE>\n          Can be either \"full\" or \"snap\" with \"full\" as default value.\n\n          [default: full]\n```",
          "timestamp": "2025-03-19T19:11:51Z",
          "tree_id": "26cb3cbe5bc142445ae282bef6c4b2f66bba1f80",
          "url": "https://github.com/lambdaclass/ethrex/commit/f1693f5490035e9244fac5e365792bf7830daa9c"
        },
        "date": 1742414923389,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 226275017717,
            "range": "± 1327541083",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "156438142+fborello-lambda@users.noreply.github.com",
            "name": "Federico Borello",
            "username": "fborello-lambda"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "652ffd357827ba5a390062ef4479f882b1ce4119",
          "message": "chore(l2): fix lint (#2271)\n\n**Motivation**\n\nThe linter was failing\n\n**Description**\n\n- Update the `lint` target\n- Implement the suggestions made by clippy",
          "timestamp": "2025-03-19T19:28:20Z",
          "tree_id": "38a24c5b9957f616967cde9b9c548f6bbf4918e8",
          "url": "https://github.com/lambdaclass/ethrex/commit/652ffd357827ba5a390062ef4479f882b1ce4119"
        },
        "date": 1742415934117,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 227444733037,
            "range": "± 635540582",
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
          "id": "d38ee5932da6c9d667f2267646f81e839b1fe3c3",
          "message": "refactor(l2): add flag for setting sponsor private key (#2281)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\nThe current implementation requires a `.env` file to exist and the\nexecution panics if this file does not exist. Nevertheless, this has a\npurpose of being. As this feature should be used in `l2` it is assumed\nthat there's a `.env` file and that is ok because it should. This PR\nintends to add a second path for setting the sponsor pk without needing\na `.env`.\n\n**Description**\n\nAdd a flag `--sponsor-private-key` as a second option for setting this\nvalue.",
          "timestamp": "2025-03-20T16:48:16Z",
          "tree_id": "d21d99b7d4992673f2d8352fa4c5e49e6c1fd55b",
          "url": "https://github.com/lambdaclass/ethrex/commit/d38ee5932da6c9d667f2267646f81e839b1fe3c3"
        },
        "date": 1742492643359,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 231005160866,
            "range": "± 1266839643",
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
          "id": "d964a2fece5ad19273b02aa5081b6a85609437dc",
          "message": "chore(core): add `rust-toolchain.toml` (#2278)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nWe have a pinned version of Rust in the CI and also in `.tool-versions`\n(for `asdf`) but not for `rustup`. We encountered ourselves running\ndifferent versions of Rust, with different results, specially when\nrunning tools like Clippy\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nAdded a `rust-toolchain.toml` file with the pinned version of Rust so\nit's evaluated by default when using `rustup`. As a side effect, needed\nto change the way Pico CLI is installed in the CI.",
          "timestamp": "2025-03-20T18:21:32Z",
          "tree_id": "bcc88ad5fae1cf708aacd02da9f6c9ae1d647967",
          "url": "https://github.com/lambdaclass/ethrex/commit/d964a2fece5ad19273b02aa5081b6a85609437dc"
        },
        "date": 1742498237074,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 229011647065,
            "range": "± 410351860",
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
          "id": "31dd81a4a8a26640c365a1eb58180f98d4f663c2",
          "message": "fix(l1): enable CORS for rpc endpoints (#2275)\n\n**Motivation**\n\nTo be used with different applications\n\n**Description**\n\nAdds a permissive CORS layer using\n[axum](https://docs.rs/axum/latest/axum/middleware/index.html) +\n[tower-http](https://docs.rs/tower-http/0.6.2/tower_http/cors/index.html).\n- All request headers allowed.\n- All methods allowed.\n- All origins allowed.\n- All headers exposed.\n\nCloses None",
          "timestamp": "2025-03-20T18:25:53Z",
          "tree_id": "149791b9bd1e6254a1f0bf5fa7fc5918a624cf0e",
          "url": "https://github.com/lambdaclass/ethrex/commit/31dd81a4a8a26640c365a1eb58180f98d4f663c2"
        },
        "date": 1742498482468,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 225153168984,
            "range": "± 875558649",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "estefano.bargas@fing.edu.uy",
            "name": "Estéfano Bargas",
            "username": "xqft"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "4f7024cdd9997138bb88ddb94f5721d0343ad95c",
          "message": "fix(l2): make TCP connection async (#2280)\n\n**Motivation**\n\nThe prover server-client TCP connection uses blocking primitive from the\nstandard library, so whenever one of the processes is expecting a\nconnection they don't yield control to the runtime and all other\nprocesses get blocked (because tokio's scheduler is cooperative).\n\nThis PR replaces these primitives with tokio's async ones.\n\nCloses #1983\nCloses #2019",
          "timestamp": "2025-03-20T19:04:14Z",
          "tree_id": "ccfea38803d446965230600c877f88b69ee4e550",
          "url": "https://github.com/lambdaclass/ethrex/commit/4f7024cdd9997138bb88ddb94f5721d0343ad95c"
        },
        "date": 1742500761736,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 226396001375,
            "range": "± 381946602",
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
          "distinct": false,
          "id": "862fb49e6143e5bdc1f3aa8939a95dff4038e5f2",
          "message": "fix(l1): fix unending storage healer process in snap sync (#2287)\n\n**Motivation**\nThere is currently a bug in snap sync. When a state sync becomes stale,\nthe snap sync cycle is aborted but the storage healer process is left\nhanging instead if signaling it to end and waiting for it to finish. The\nloop condition of the storage healer is also not properly set, keeping\nit alive even after the end signal if it still has paths to heal. This\nPR fixes both of this problems\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Fix loop condition in storage healer\n* End storage healer if state sync aborts due to stale pivot\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-03-21T13:40:23Z",
          "tree_id": "53424afc61727988e153fbb7b02a7f2ddc50c7d0",
          "url": "https://github.com/lambdaclass/ethrex/commit/862fb49e6143e5bdc1f3aa8939a95dff4038e5f2"
        },
        "date": 1742567830515,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 232806520727,
            "range": "± 1000563047",
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
          "id": "ab751f0470192a2120b27f9ef207ff5e06c4676f",
          "message": "feat(l1): write multiple account's storage batches in the same db txn (#2270)\n\n**Motivation**\nWhen measuring time taken by each task during snap sync I noticed that a\nlot of time was spent writing the storage ranges obtained from peers to\nthe DB snapshot. It would take anywhere from 3 to over 10 seconds to\nwrite all the ranges to the DB (around 300 storage ranges per request).\nThis PR modifies the insertion logic to write all 300 ranges in the same\nDB transaction, reducing the time taken to write all the ranges to the\nDB to 10 milliseconds or less\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Add `write_storage_snapshot_batches` method to `Store`, which can\nwrite multiple batches from different accounts on the same txn\n* Write all storage ranges received from peers in a single DB txn using\nthe method above on the storage fetcher (snap sync)\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses: None, but helps speed up snap sync",
          "timestamp": "2025-03-21T14:54:55Z",
          "tree_id": "8b597efa81f871d126ab9b85f32aa9034fe83bf5",
          "url": "https://github.com/lambdaclass/ethrex/commit/ab751f0470192a2120b27f9ef207ff5e06c4676f"
        },
        "date": 1742572214466,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 227965982892,
            "range": "± 509390539",
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
          "distinct": true,
          "id": "d481d7f17c8843b51651e5ba46390f5444498998",
          "message": "feat(l2): `restart-testnet` target (#2293)\n\n**Motivation**\n\nHaving a target for restarting the L2 deployment on a testnet.",
          "timestamp": "2025-03-21T19:42:06Z",
          "tree_id": "fdbd75a5f2efcdeb68d8aa47188d17f89626be61",
          "url": "https://github.com/lambdaclass/ethrex/commit/d481d7f17c8843b51651e5ba46390f5444498998"
        },
        "date": 1742589574979,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 227901888050,
            "range": "± 862870744",
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
          "distinct": true,
          "id": "9b0c70f3121eac4dcf86a3fd62220281cfa697cc",
          "message": "feat(l2): add state reconstruction command (#2204)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nWe need a way to reconstruct the chain state in case of a failure or\neven if someone want to _trustlessly_ access the state.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nThis PR introduces a new ethrex_l2 CLI command, `stack reconstruct`,\nthat takes downloaded blobs from L1 and reconstruct the blocks based on\nits info, storing the state in a Libmdbx store. The blobs can be\ndownloaded using the `stack blobs-saver` command.\nAt this stage, the command is able to successfully reconstruct the chain\nstate and continue to produce blocks.\nNote that, as we send state diffs and not transactions to L1, some data\n(i.e., transactions history, receipts) will not be accessible in a\nreconstructed network.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #1103\n\n---------\n\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>",
          "timestamp": "2025-03-21T20:42:37Z",
          "tree_id": "e5e608acebe033aebc9bcc46324c291a5898ee38",
          "url": "https://github.com/lambdaclass/ethrex/commit/9b0c70f3121eac4dcf86a3fd62220281cfa697cc"
        },
        "date": 1742593156757,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 224024128750,
            "range": "± 455656359",
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
          "id": "92cd758fd30447b82b6fffa13351772b50d6a165",
          "message": "fix(l2): use absolute path for `.env` file (#2295)\n\n**Motivation**\n\nRunning the stack outside of `crates/l2` fails because the `.env` file\npath is set to its relative form.\n\n**Description**\n\nUse the `.env` file absolute path.",
          "timestamp": "2025-03-21T21:09:42Z",
          "tree_id": "2b74de1133b3bb608b8d2f7dbca638d55b6d227d",
          "url": "https://github.com/lambdaclass/ethrex/commit/92cd758fd30447b82b6fffa13351772b50d6a165"
        },
        "date": 1742594648536,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222976173527,
            "range": "± 1235507741",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "martin.c.paulucci@gmail.com",
            "name": "Martin Paulucci",
            "username": "mpaulucci"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "7d4b056cd387c0db577b6fabd1485013ad11efeb",
          "message": "refactor(core): do not leak vm specific implementations from vm crate. (#2297)\n\n**Motivation**\nMake progress toward removing abstraction leaks in vm crate. Outside of\nvm, we should not know about revm vs levm.\n\n**Description**\n- Created `internal` feature flag for the crates that still need to\naccess internal apis: state tests and zkvm interfaces. The idea is that\nit will be temporary until we can remove the leaks from those crates.\n- Refactored the code to make the api explicit in `/vm/lib.rs`. Do not\nexpose modules to the outside by default. This is a first step, we're\nstill exposing too much.\n- Encapsulated `SpecId`, which is a internal concept inside vm, from\noutside we use `Fork`\n- Added utility function `create_contract_address` that uses revm. Added\nthat function to vm crate.",
          "timestamp": "2025-03-25T12:49:21Z",
          "tree_id": "80141def374bcf58a68aaa928524962bf375247e",
          "url": "https://github.com/lambdaclass/ethrex/commit/7d4b056cd387c0db577b6fabd1485013ad11efeb"
        },
        "date": 1742910553704,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 230795622445,
            "range": "± 1202144474",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "mrugiero@gmail.com",
            "name": "Mario Rugiero",
            "username": "Oppen"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "14406e2d1984a3398945e9db4e29e8948a079995",
          "message": "fix(l2): remove uses of blocking sleeps from async code (#2296)\n\nThere were still some sleeps blocking the runtime. Found mostly in the\nload test, but in other places as well. Changed them by tokio::sleep\ncalls.",
          "timestamp": "2025-03-25T13:10:21Z",
          "tree_id": "48674ee61d71343d1d8f58623ac22b1399d1d511",
          "url": "https://github.com/lambdaclass/ethrex/commit/14406e2d1984a3398945e9db4e29e8948a079995"
        },
        "date": 1742911720109,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 232681672891,
            "range": "± 591152064",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "156438142+fborello-lambda@users.noreply.github.com",
            "name": "Federico Borello",
            "username": "fborello-lambda"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "a8bb355f7fe072474461b29b8e1f68c7bdc75d75",
          "message": "fix(l2): prover_client with SP1 (#2273)\n\n**Motivation**\n\nWhen we bumped the SP1 version to the latest we didn't test the\n`prover_client` on its own.\nAlso, we had some issues when using CUDA with a `ctrl-c` handler set\ninside the `sp1-cuda` crate.\n \n**Description**\n\n- Fix Makefile Target\n- Bump the contract version\n- Start a single SP1's client with `LazyLock` to fix the CUDA issues\ndescribed above.",
          "timestamp": "2025-03-25T18:14:11Z",
          "tree_id": "bf146f62bf307abc955a0c3593207b587fbdc98f",
          "url": "https://github.com/lambdaclass/ethrex/commit/a8bb355f7fe072474461b29b8e1f68c7bdc75d75"
        },
        "date": 1742929926890,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 231323537827,
            "range": "± 840969873",
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
          "id": "55d8bd520e032323a83e780986d23156161d66d3",
          "message": "refactor(l2): rework gas fee bump (#2277)\n\n**Motivation**\n\nThis PR moves all logic related to handling transaction retries and\nbumping gas fees to a single function\n`send_tx_bump_gas_exponential_backoff` (before it was scattered in a few\ndiferent places, hard to follow and with no exponential backoff).\n\nIt also introduces a small randomness to the intervals with which the\nmain processes (l1 commiter, l1 watcher and prover server) execute their\nmain loop, to avoid possible problems related to things running at\ndeterministic intervals.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number\n\n---------\n\nCo-authored-by: ilitteri <ilitteri@fi.uba.ar>",
          "timestamp": "2025-03-25T19:24:45Z",
          "tree_id": "c671a1aa88bd75ff93d79bc553c7cd90c4d6b73f",
          "url": "https://github.com/lambdaclass/ethrex/commit/55d8bd520e032323a83e780986d23156161d66d3"
        },
        "date": 1742934128302,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 227892360916,
            "range": "± 592695565",
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
          "id": "4c002213e1aaf16a64d09de3d93741103e73bd02",
          "message": "feat(l2): add rpc endpoints for based sequencing (#2274)\n\n> [!NOTE]\n> Original PR: https://github.com/lambdaclass/ethrex/pull/2022\n(squeashed because of unsigned commits).\n\n---------\n\nCo-authored-by: Manuel Iñaki Bilbao <bilbaomanuel98@gmail.com>",
          "timestamp": "2025-03-25T21:47:44Z",
          "tree_id": "bd700c81bc0bf69843c8d7d44d57d1a2c8abac6a",
          "url": "https://github.com/lambdaclass/ethrex/commit/4c002213e1aaf16a64d09de3d93741103e73bd02"
        },
        "date": 1742942718069,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 229725052320,
            "range": "± 1338873796",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "76252340+MarcosNicolau@users.noreply.github.com",
            "name": "Marcos Nicolau",
            "username": "MarcosNicolau"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "cdbfbe904b5742dc6fefb48f2a12c18001264b9d",
          "message": "feat(l1): process blocks in batches when syncing and importing (#2174)\n\n**Motivation**\nAccelerate syncing!\n\n**Description**\nThis PR introduces block batching during full sync:\n1. Instead of storing and computing the state root for each block\nindividually, we now maintain a single state tree for the entire batch,\ncommitting it only at the end. This results in one state trie per `n`\nblocks instead of one per block (we'll need less storage also).\n2. The new full sync process:\n    - Request 1024 headers\n    - Request 1024 block bodies and collect them\n- Once all blocks are received, process them in batches using a single\nstate trie, which is attached to the last block.\n3. Blocks are now stored in a single transaction.\n4. State root, receipts root, and request root validation are only\nrequired for the last block in the batch.\n5. The new add_blocks_in_batch function includes a flag,\n`should_commit_intermediate_tries`. When set to true, it stores the\ntries for each block. This functionality is added to make the hive test\npass. Currently, this is handled by verifying if the block is within the\n`STATE_TRIES_TO_KEEP` range. In a real syncing scenario, my intuition is\nthat it would be better to wait until we are fully synced and then we\nwould start storing the state of the new blocks and pruning when we\nreach `STATE_TRIES_TO_KEEP`.\n6. Throughput when syncing is now measured per batches.\n7. A new command was added to import blocks in batch\n\nConsiderations:\n1. ~Optimize account updates: Instead of inserting updates into the\nstate trie after each block execution, batch them at the end, merging\nrepeated accounts to reduce insertions and improve performance (see\n#2216)~ Closes #2216.\n2. Improve transaction handling: Avoid committing storage tries to the\ndatabase separately. Instead, create a single transaction for storing\nreceipts, storage tries, and blocks. This would require additional\nabstractions for transaction management (see #2217).\n3. This isn't working for `levm` backend we need it to cache the\nexecutions state and persist it between them, as we don't store anything\nuntil the final of the batch (see #2218).\n4. In #2210 a new ci is added to run a bench comparing main and `head`\nbranch using `import-in-batch`\n\nCloses None\n\n---------\n\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>\nCo-authored-by: fmoletta <99273364+fmoletta@users.noreply.github.com>",
          "timestamp": "2025-03-25T21:48:54Z",
          "tree_id": "5ee3b5d1c38da882ce4394e5df4f01dbe40c43bf",
          "url": "https://github.com/lambdaclass/ethrex/commit/cdbfbe904b5742dc6fefb48f2a12c18001264b9d"
        },
        "date": 1742942819671,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 231706874953,
            "range": "± 1486957612",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "mrugiero@gmail.com",
            "name": "Mario Rugiero",
            "username": "Oppen"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "579275c0bc6392b7f7ff25f4cf253579cadb2245",
          "message": "fix(l2): bashism in l2 Makefile (#2301)\n\nThe `[[` builtin is not POSIX, which causes issues in some servers that\ndefault their shell to `sh` (POSIX-compat mode). Specifically, because\nthe builtin does not exist, the L2 always runs in based mode due to the\nerror.\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-03-25T22:20:04Z",
          "tree_id": "4de2af114f2cec936f4ea95e5e169282d4038fb2",
          "url": "https://github.com/lambdaclass/ethrex/commit/579275c0bc6392b7f7ff25f4cf253579cadb2245"
        },
        "date": 1742944672081,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 230469754658,
            "range": "± 700649031",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "martin.c.paulucci@gmail.com",
            "name": "Martin Paulucci",
            "username": "mpaulucci"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "65ac1fdd7fbb86a7b56992dbd6a6a822713b8405",
          "message": "ci(core): disable flamegraph report until it is fixed. (#2312)\n\n**Motivation**\nThis job is broken. Disabling it until it gets fixed.",
          "timestamp": "2025-03-26T13:57:20Z",
          "tree_id": "9569055f5bca151e1d6111556907cd23ff096cd2",
          "url": "https://github.com/lambdaclass/ethrex/commit/65ac1fdd7fbb86a7b56992dbd6a6a822713b8405"
        },
        "date": 1743000927874,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 231415392619,
            "range": "± 1028060620",
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
          "distinct": true,
          "id": "d1faf8b4658bc3f36c35159717303fa3af384fd2",
          "message": "test(l2): add state reconstruction test to the CI (#2255)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nWe want to check that the state diff reconstruction doesn't break on\nPRs.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nAdded some tests that reconstruct the state from 3 blobs, which include\nbalance and nonce diffs, and an ERC20 contract \"deployment\" with balance\ndiffs.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\n---------\n\nCo-authored-by: Federico Borello <156438142+fborello-lambda@users.noreply.github.com>\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>\nCo-authored-by: Javier Chatruc <jrchatruc@gmail.com>",
          "timestamp": "2025-03-26T15:11:43Z",
          "tree_id": "79801ab99fd69dda90bb28100f2f5c991cd76480",
          "url": "https://github.com/lambdaclass/ethrex/commit/d1faf8b4658bc3f36c35159717303fa3af384fd2"
        },
        "date": 1743005326164,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 226755411287,
            "range": "± 357419523",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "76252340+MarcosNicolau@users.noreply.github.com",
            "name": "Marcos Nicolau",
            "username": "MarcosNicolau"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "f8f9552b9c9c8755e8156752e051a347c1feb169",
          "message": "fix(l1): blocking tokio scheduler while syncing (#2314)\n\n**Motivation**\nSyncing.\n\n**Description**\nExecuting blocks is a CPU-intensive task. During syncing, this process\nwas blocking the Tokio runtime, causing other tasks to stop working. A\nmajor issue was that our node stopped responding to p2p requests,\nleading to abrupt disconnections.\n\nThis fix resolves the problem by spawning the block execution with tokio\n`spawn_blocking`, which runs tasks in a separate thread pool optimized\nfor CPU-heavy operations. This prevents the scheduler from being\nblocked, fixing the networking issue.\n\nCloses None",
          "timestamp": "2025-03-26T15:31:19Z",
          "tree_id": "a1e35a5c8219b0f5d0261788d6f5dfa994803e62",
          "url": "https://github.com/lambdaclass/ethrex/commit/f8f9552b9c9c8755e8156752e051a347c1feb169"
        },
        "date": 1743006649776,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 225655939542,
            "range": "± 664288582",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "martin.c.paulucci@gmail.com",
            "name": "Martin Paulucci",
            "username": "mpaulucci"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "206c56e2e02c569b00fd6ce73e3106432e811793",
          "message": "ci(core): remove rust version since it's already specified in toolchain (#2311)\n\n**Motivation**\nYou can see this message in the CI:\n`info: note that the toolchain '1.82.0-x86_64-unknown-linux-gnu' is\ncurrently in use (overridden by\n'/home/runner/work/ethrex/ethrex/rust-toolchain.toml')`",
          "timestamp": "2025-03-26T15:42:13Z",
          "tree_id": "fe462380636577aa82af2df8210b069231cfd8a5",
          "url": "https://github.com/lambdaclass/ethrex/commit/206c56e2e02c569b00fd6ce73e3106432e811793"
        },
        "date": 1743007195511,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 229594303409,
            "range": "± 2536574230",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "martin.c.paulucci@gmail.com",
            "name": "Martin Paulucci",
            "username": "mpaulucci"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "4583997f02f572e6587abbd9239994f0c368080b",
          "message": "chore(core): improve ci loc job (#2304)\n\n**Motivation**\nThe job doesn't seem to work well with forks (external contributions).\nThis aims to fix it",
          "timestamp": "2025-03-26T15:43:52Z",
          "tree_id": "4bd3553342079f6567bfb4bf0dc62d9354a54f37",
          "url": "https://github.com/lambdaclass/ethrex/commit/4583997f02f572e6587abbd9239994f0c368080b"
        },
        "date": 1743007281368,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 229822436417,
            "range": "± 575467306",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "112426153+tomip01@users.noreply.github.com",
            "name": "Tomás Paradelo",
            "username": "tomip01"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "d03ec50019df7c02dadcf39ba5e08d4c7086a67a",
          "message": "refactor(l2): use levm for sp1 prover using a trait (#2250)\n\n**Motivation**\n\nThis PR exists to use LEVM as the EVM for executing SP1 prover\n\n**Description**\n\n- Implement the trait `LevmDatabase` for the `ExecutionDb`.\n- Now the LEVM backend can execute blocks or transactions on any\ndatabase that implements the `LevmDatabase` trait.\n- Move the `ExecutionDb` to a common place and remove from some of REVM\ndependencies. But, there are some left to be removed in the next PR.\n- Add a feature flag `levm-l2` for choosing whether to execute the\nProver and the ExecutionDb with LEVM or not.\n\n**Status**\n\n- This a second implementation for the same purpose as #2231 . Only one\nshould be maintained.\n- ExecutionDb isn't fully decoupled yet.\n\nLinks to #2225",
          "timestamp": "2025-03-26T18:01:21Z",
          "tree_id": "8bb3c3b10eea3148b2c9cfd302bb1009cf65b2b3",
          "url": "https://github.com/lambdaclass/ethrex/commit/d03ec50019df7c02dadcf39ba5e08d4c7086a67a"
        },
        "date": 1743015552445,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 229831357611,
            "range": "± 477453820",
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
          "id": "956537fb88e932f4bbc629ae82116e05f91ec894",
          "message": "fix(l1, l2): fix load tests (#2323)\n\n**Motivation**\n\nLoad tests were broken for two reasons:\n\n- We were not correctly passing the nonce as an override and thus were\nrelying on the RPC endpoint to get it, which was not correct (since we\nwant to pre-send transactions with higher nonces)\n- We were hardcoding gas fees; this is because when we first wrote the\nload tests, the gas price endpoint on ethrex did not work properly. Now\nthat it does, we can remove the hardcoded values and just rely on the\nendpoint (the default behaviour if you do not pass an `Override` to the\n`build_eip1559_transaction` function).\n\nI also changed the `debug!` log when a mempool transaction failed to be\nexecuted while building a block to be an `error!`, because I noticed\nit's quite a common occurrence when we run load tests due to some nonce\nissue, and I think it's worth investigating (it's the reason why\nsometimes we get empty blocks when running load tests).\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number\n\n---------\n\nCo-authored-by: JereSalo <jeresalo17@gmail.com>",
          "timestamp": "2025-03-26T18:52:46Z",
          "tree_id": "539ef04a6f159f38f5ab44220b15d82d09094181",
          "url": "https://github.com/lambdaclass/ethrex/commit/956537fb88e932f4bbc629ae82116e05f91ec894"
        },
        "date": 1743018680788,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 230749596243,
            "range": "± 1764496588",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "156438142+fborello-lambda@users.noreply.github.com",
            "name": "Federico Borello",
            "username": "fborello-lambda"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "9c0b365bef18c5639aa02a94445e4030ce43ba1d",
          "message": "refactor(l2): handle ctrl_c internally and multiple connections (#2294)\n\n**Motivation**\n\nWe should `spawn` a new task every new connection received in the\n`prover_server`.\nAlso, the ctrl_c handler was wired through the TCP layer.\n\n**Description**\n\n- Create a new task per connection\n- Handle the ctrl_c internally with the help of `select!` and a\n`tokio::mpsc`\n- Add a `Semaphore` to cap the amount of concurrent tasks.\n\nCloses #2283\nCloses #2284\n\n---------\n\nCo-authored-by: Mario Rugiero <mrugiero@gmail.com>",
          "timestamp": "2025-03-26T19:48:39Z",
          "tree_id": "fe6e16cbc963211db4c469210bd0a846f1e96361",
          "url": "https://github.com/lambdaclass/ethrex/commit/9c0b365bef18c5639aa02a94445e4030ce43ba1d"
        },
        "date": 1743022012383,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 233358784894,
            "range": "± 475162402",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "156438142+fborello-lambda@users.noreply.github.com",
            "name": "Federico Borello",
            "username": "fborello-lambda"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "dc392087828781e0c0d1d10008fc38543a2f02eb",
          "message": "feat(l2): omit unneeded proofs (#2235)\n\n**Motivation**\n\nWe had to wait for all prover backends' proofs in order to send the\n`verify` transaction and continue with the desired behavior of\n`commitment` &rarr; then `verification`.\n\nNow, we can make use of only one backend.\n\n**Description**\n\n- Check the `Verification` contract address querying the contract\n  - If it is `0xAA` we don't wait for that backend's proof.\n\n---------\n\nCo-authored-by: Estéfano Bargas <estefano.bargas@fing.edu.uy>",
          "timestamp": "2025-03-26T20:50:41Z",
          "tree_id": "368f634d7ae6c24a3d89414dc4b78b0499257299",
          "url": "https://github.com/lambdaclass/ethrex/commit/dc392087828781e0c0d1d10008fc38543a2f02eb"
        },
        "date": 1743025705932,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 230748524994,
            "range": "± 543494546",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "156438142+fborello-lambda@users.noreply.github.com",
            "name": "Federico Borello",
            "username": "fborello-lambda"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "170308afefe78c08b13bcda3111ec5e4158e87a5",
          "message": "refactor(l2): separated configs for prover_client and sequencer (#2269)\n\n**Motivation**\n\nWhen running the prover_client as a standalone component the\n`config.toml` wasn't being parsed.\nIdeally we should parse it before we run the prover_client.\n\n**Description**\n\n- The `ConfigMode` enum is proposed to parse the .toml for the\n`Sequencer` or the `ProverClient`\n- The prover_client parses the `prover_client_config.toml` and creates a\n`.env.prover` file\n- Created new envars to set the:\n  - `CONFIGS_PATH` \n  - `SEQUENCER_CONFIG_FILE`\n  - `PROVER_CLIENT_CONFIG_FILE`\n  - The references were updated in the Makefile\n\nThis change also enables us to change the `SEQUENCER_CONFIG_FILE` easily\nkeeping it in the `configs` dir and switching the `Makefile`'s variable.\n(Useful when testing locally and with a testnet).\n\nCloses #2053\n\n---------\n\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-03-26T21:12:57Z",
          "tree_id": "031ff061bacfb206344cc64371c74742f0423ba5",
          "url": "https://github.com/lambdaclass/ethrex/commit/170308afefe78c08b13bcda3111ec5e4158e87a5"
        },
        "date": 1743027019113,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 228971430179,
            "range": "± 936604869",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "76252340+MarcosNicolau@users.noreply.github.com",
            "name": "Marcos Nicolau",
            "username": "MarcosNicolau"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "b36a7c603985788c9cc115d123dfa0649eac997b",
          "message": "perf(core): compute tx senders in parallel (#2268)\n\n**Motivation**\nIncrease performance\n\n**Description**\nA big time of `execute_block` in the vm was spent in recovering the\n`address` from the transactions. This pr, parallelizes the computation\nof the address and reduces the time down to almost negligible.\n\nIt also fixes the ci that got broken with the latest changes.\n\nCloses None",
          "timestamp": "2025-03-27T12:35:52Z",
          "tree_id": "061ab79965fa884720b1bac7353c219c7520eba1",
          "url": "https://github.com/lambdaclass/ethrex/commit/b36a7c603985788c9cc115d123dfa0649eac997b"
        },
        "date": 1743082019601,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 184805574099,
            "range": "± 1273678246",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "46695152+LeanSerra@users.noreply.github.com",
            "name": "LeanSerra",
            "username": "LeanSerra"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "f13c24d9197d162c64d2f05b26669307a090681b",
          "message": "feat(levm): implement create_access_list (#2244)\n\n**Motivation**\n\nImplement create_access_list for levm\n\n**Description**\n\n- Implement a function that executes a transaction and creates from the\nresulting `accrued_substate` an access list.\n- Add a function to utils that generates the access list\n\n**Observation**\n\nChanges `touched_storage_slots` from `HashSet` to `BTreeSet` to align\nwith the expected output order of the addresses in the Hive tests.\n\n**Hive Tests**\n\nThese hive tests should be fixed with this PR\n```Shell\nmake run-hive EVM_BACKEND=\"levm\" SIMULATION=\"ethereum/rpc-compat\" TEST_PATTERN=\"rpc-compat/eth_createAccessList/\"                          \n```\n\nCloses #2183\n\n---------\n\nCo-authored-by: avilagaston9 <gavila@fi.uba.ar>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>",
          "timestamp": "2025-03-27T15:46:49Z",
          "tree_id": "2cb5c83041298ea0d404c437c8c0cc55581e155d",
          "url": "https://github.com/lambdaclass/ethrex/commit/f13c24d9197d162c64d2f05b26669307a090681b"
        },
        "date": 1743093450747,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 190274384277,
            "range": "± 857475100",
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
          "distinct": true,
          "id": "f0cfaa6cae7e11c6fdc02427654b04554127ee36",
          "message": "refactor(l2): rename proposer config with a more descriptive name (#2341)",
          "timestamp": "2025-03-27T20:40:24Z",
          "tree_id": "a91440aed48af0296e4d4a0df95941c29af4c8f0",
          "url": "https://github.com/lambdaclass/ethrex/commit/f0cfaa6cae7e11c6fdc02427654b04554127ee36"
        },
        "date": 1743110966847,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 184674546450,
            "range": "± 1003660756",
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
          "distinct": true,
          "id": "d90a9dc5e6269543a85cc7ae9177dfd23bfb17d3",
          "message": "fix(core): make metrics port flag not optional (#2343)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\nCurrently, the metrics are initiated iif the `--metrics-port` flag is\npassed. This is wrong because the flag is used both to configure the\nlistening port and as a metrics enabler flag.\n\nIf needed, a flag `--metrics.enabled` could be introduced in another PR\nif metrics are unwanted for some reason. IMHO initializing metrics as\ndefault is ok.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\n- Always initialize metrics\n- The `--metrics.port` flag is now not optional and defaults to 9090 as\nthe default value.",
          "timestamp": "2025-03-27T20:58:50Z",
          "tree_id": "45eacb2f732ca8dbb353d790b06ee84795360b48",
          "url": "https://github.com/lambdaclass/ethrex/commit/d90a9dc5e6269543a85cc7ae9177dfd23bfb17d3"
        },
        "date": 1743112138346,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 188848074989,
            "range": "± 1075290354",
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
          "id": "d41018978ddb6684452a48e44872e07071175dc1",
          "message": "refactor(l2): rename prover client config to a more descriptive name (#2345)\n\n- Rename prover client `interval_ms` -> `proving_time_ms`.\n- Remove needless `ProverClientConfig` struct in `toml_parser`.",
          "timestamp": "2025-03-28T00:49:34Z",
          "tree_id": "e0c3e409cb121bf4dd608b77a7b788ef32bd83af",
          "url": "https://github.com/lambdaclass/ethrex/commit/d41018978ddb6684452a48e44872e07071175dc1"
        },
        "date": 1743125929502,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 184651396277,
            "range": "± 644941291",
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
          "id": "14216ab80801c1edf2ac0f2f99c4d091dce64cc4",
          "message": "feat(core): add metrics address config flag (#2344)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\nNowadays, the metrics API address is hardcoded to `0.0.0.0`. This PR\naims to parameterize this.\n\n**Description**\n\n- Adds a `--metrics.addr` flag to the CLI with `0.0.0.0` as the default\nvalue.\n- Implement the wiring necessary to pass the flag value to the metrics\nAPI initialization.\n\n---------\n\nCo-authored-by: fborello-lambda <federicoborello@lambdaclass.com>",
          "timestamp": "2025-03-28T13:08:40Z",
          "tree_id": "f796da1d04454ae694dea1c857792ba51caae4c9",
          "url": "https://github.com/lambdaclass/ethrex/commit/14216ab80801c1edf2ac0f2f99c4d091dce64cc4"
        },
        "date": 1743170262005,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 185323473554,
            "range": "± 645552899",
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
          "distinct": true,
          "id": "8a026aa0fdd0e02c2d1bc5c2436e5ab05086cd05",
          "message": "fix(l2): config error handling (#2339)\n\n**Motivation**\n\nIn a previous PR, the configuration file error handling was updated, and\nthe help messages stopped being helpful. This PR aims to make these\nerror messages useful again and improve their handling.\n\nThe current help message does not work:\n\n```Shell\nError parsing the .toml configuration files: Could not find crates/l2/configs/config.toml\nHave you tried copying the provided example? Try:\ncp /Users/ivanlitteri/Repositories/lambdaclass/ethrex/crates/l2/configs/*_example.toml /Users/ivanlitteri/Repositories/lambdaclass/ethrex/crates/l2/configs/*.toml\n\nError: ConfigError(TomlParserError(TomlFileNotFound(\"config.toml\")))\nmake: *** [deploy-l1] Error 1\n➜  l2 git:(main) ✗ cp /Users/ivanlitteri/Repositories/lambdaclass/ethrex/crates/l2/configs/*_example.toml /Users/ivanlitteri/Repositories/lambdaclass/ethrex/crates/l2/configs/*.toml\ncp: /Users/ivanlitteri/Repositories/lambdaclass/ethrex/crates/l2/configs/sequencer_config_example.toml is not a directory\n```\n\n**Description**\n\n- Add a prefix `sequencer_` to the sequencer config file to be\nconsistent with the prover client config file and update its references.\n- Pass `ConfigMode` to the `toml` errors to make the help messages\nhelpful again, and implement `Debug` and `Display` for this on it.\n- Make the `toml` parsing error explicit.",
          "timestamp": "2025-03-28T13:25:33Z",
          "tree_id": "ca573e46b003e3b630b64bdaaad230aa8de55184",
          "url": "https://github.com/lambdaclass/ethrex/commit/8a026aa0fdd0e02c2d1bc5c2436e5ab05086cd05"
        },
        "date": 1743171366371,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 185080725230,
            "range": "± 833080354",
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
          "distinct": true,
          "id": "835045b3fc5b905cb90d64e59b3febef16b960c6",
          "message": "refactor(l2, core): initial iteration for `l2` subcommand (#2324)\n\n**Motivation**\n\nThere are two motivations for this PR:\n1. Decouple L2 logic (initialization, etc) from `ethrex.rs`.\n2. Replace `crates/l2/Makefile` (we'll keep the Makefile for simplicity,\nby replacing I mean to have all the logic in `l2` subcommands such as\n`l2 init` and `l2 removedb` that replace `make init-l2` and `make\nrm-db-l2` logic). In future PRs we'll add more subcommands, making the\nMakefile a shortcut for running `cargo run --release --bin ethrex\n--features l2 -- l2 <some command>`.\n\n**Description**\n\n- Add an `l2.rs` submodule for the L2 subcommand logic.\n- Decouple L2 initialization from `ethrex.rs` file (moved into\nsubcommand handling).\n- Merge `BasedOptions` into `L2Options` (based options are also L2\noptions).\n- Implement `l2 init` and `l2 removedb` commands.\n- Update `crates/l2/Makefile` to use these new commands.\n\n**Test it out**\n\nDoing your regular L2 initialization with the makefile should be enough.\n\nResolves #2246.\nResolver #1987",
          "timestamp": "2025-03-28T14:53:11Z",
          "tree_id": "1bb340b2e037e27a22e0c78206cdbc0cad1d0a82",
          "url": "https://github.com/lambdaclass/ethrex/commit/835045b3fc5b905cb90d64e59b3febef16b960c6"
        },
        "date": 1743176596284,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 187390215232,
            "range": "± 882029235",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "156438142+fborello-lambda@users.noreply.github.com",
            "name": "Federico Borello",
            "username": "fborello-lambda"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "9a28ac444e5e7f01effbdbf36a9d5dddb9943d8b",
          "message": "fix(l2): prover_client_config parser (#2348)\n\n**Motivation**\n\nThe previous PR removed the `ProverClientConfig` leaving just the\n`ProverClient` structure. To successfully parse the file, we should\nremove the `prover_client` table header.\n\n**Description**\n\n- Remove header from `prover_client_config_example.toml`",
          "timestamp": "2025-03-28T16:13:55Z",
          "tree_id": "da357e2b19a1d3929c363d656f3dadbcd12cfb0a",
          "url": "https://github.com/lambdaclass/ethrex/commit/9a28ac444e5e7f01effbdbf36a9d5dddb9943d8b"
        },
        "date": 1743181442687,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 188884940955,
            "range": "± 818849675",
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
          "id": "27232155ca4b70ac1658d86e8411a00618e66598",
          "message": "feat (l1): write nodes in batches during storage healing (#2288)\n\n**Motivation**\nIn a similar fashion to #2270, this PR aims to reduce the time spent\nwriting data to the DB by writing data in batches. In this case the\nnodes received during storage healing are written all at once using the\nalready existing `put_batch` method of the TrieDB trait.\nThis could only be done for nodes belonging to the same trie, as it\nwould otherwise involve leaking and constraining the internal\nrepresentation of TrieDB.\nThis has shown to reduce the time spent writing storage nodes in the DB\nfrom around 4 seconds to less than 20 milliseconds\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Add `write_node_batch` method for `TrieState` relying on\n`TrieDB::put_batch`\n* Refactor storage healer code to write all nodes for a trie in a single\noperation\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number\n\n---------\n\nCo-authored-by: Rodrigo Oliveri <rodrigooliveri10@gmail.com>",
          "timestamp": "2025-03-28T21:21:38Z",
          "tree_id": "b424781e8fce7d01c22aba84916878998d30b789",
          "url": "https://github.com/lambdaclass/ethrex/commit/27232155ca4b70ac1658d86e8411a00618e66598"
        },
        "date": 1743199849892,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 182531092599,
            "range": "± 956807821",
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
          "distinct": false,
          "id": "7cd2ddc67c483fad5bb6de52b4c81a0986118228",
          "message": "feat(l1): add separate queue for large storages during snap sync (#2256)\n\n**Motivation**\nCurrently, large storage tries are handled by the same process that\nhandles smaller storage tries, which can cause the fetcher to stall when\nencountering large storages. This PR aims to fix this by delegating the\nfetching of large storages to a separate queue process\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Add a new fetcher process for large storages with its own queue. (One\nwill be spawned for each storage fetcher\n* Delegate fetching of large storages to the large storage fetcher\n* Allow the rebuilder to skip root validations for partial storage tries\nwhen the pivot becomes stale during a large storage trie fetch\n* Other: unify all SendError into one generic mapping for SyncError\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #1965\n\n---------\n\nCo-authored-by: ElFantasma <estebandh@gmail.com>",
          "timestamp": "2025-03-28T21:21:19Z",
          "tree_id": "13e2d1f16201031cc00b2951e185542735ad341d",
          "url": "https://github.com/lambdaclass/ethrex/commit/7cd2ddc67c483fad5bb6de52b4c81a0986118228"
        },
        "date": 1743199877296,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 186577284621,
            "range": "± 700543628",
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
          "id": "c18d692f640abbc540f73fe288ace5760314e94c",
          "message": "perf(levm): remove repeated `get_account_info` calls in LEVM (#2357)\n\n**Motivation**\n\nNoticed on #2292 that the majority of the time in\n`LEVM::get_state_transitions()` was spent on calls to\n`get_account_info()`. While looking for areas to improve, I found that\nwe were calling `get_account_info()` three times instead of reusing the\nvalue returned in the first call.\n\n**Description**\n\nRemoves the repeated calls to `get_account_info`.\n\nTesting locally shows a `2x` speed improvement in\n`payload_builder::build_payload()` implemented in #2292.\n\nCloses None",
          "timestamp": "2025-03-31T14:24:48Z",
          "tree_id": "914ec1e5c4c92b93dcc44c3cccd62f29a48af3e6",
          "url": "https://github.com/lambdaclass/ethrex/commit/c18d692f640abbc540f73fe288ace5760314e94c"
        },
        "date": 1743434167700,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 192357867237,
            "range": "± 680639721",
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
          "distinct": false,
          "id": "f3063f124020f239617ea2d30de689209ac68e3a",
          "message": "feat(l1): write state nodes in batches during state healing (#2309)\n\n**Motivation**\nIn a similar fashion to #2288, this PR aims to reduce the time spent\nwriting data to the DB by writing data in batches. In this case the\nnodes received during storage healing are written all at once using the\n`write_node_batch` method introduced in #2288\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Refactor state healer code to write all nodes for a trie in a single\noperation\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number\n\n---------\n\nCo-authored-by: Rodrigo Oliveri <rodrigooliveri10@gmail.com>",
          "timestamp": "2025-03-31T14:42:21Z",
          "tree_id": "6182c78c84c86b7b3f8d97ebd099ff2eb007d5bd",
          "url": "https://github.com/lambdaclass/ethrex/commit/f3063f124020f239617ea2d30de689209ac68e3a"
        },
        "date": 1743435097610,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 183532074688,
            "range": "± 572310191",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "56402156+fkrause98@users.noreply.github.com",
            "name": "Francisco Krause Arnim",
            "username": "fkrause98"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "67e1fa89d5ed5f86bfce59d3eaad0f9b4f465890",
          "message": "perf(l1,l2): trie benchmark (#2272)\n\n**Motivation**\n\nWe want to speed-up our trie implementation, for that, we\nwant reproducible benchmarks and a baseline for comparison.\n\n**Description**\n- Add benchmark for Ethrex's Trie, compared against citra.\n- Add UUID dependency to generate random data, a dev-only dep.\n\n\n\nCloses #2262.\n\n---------\n\nCo-authored-by: fmoletta <99273364+fmoletta@users.noreply.github.com>",
          "timestamp": "2025-03-31T16:10:26Z",
          "tree_id": "9f01749764791a04f711a6fb5aced1bf2df1c0da",
          "url": "https://github.com/lambdaclass/ethrex/commit/67e1fa89d5ed5f86bfce59d3eaad0f9b4f465890"
        },
        "date": 1743440458334,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 187077770228,
            "range": "± 727464219",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "156438142+fborello-lambda@users.noreply.github.com",
            "name": "Federico Borello",
            "username": "fborello-lambda"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "b3e3705e7252a3dde1c6edc6da3750ed77b9ac91",
          "message": "chore(l2): remove deprecated EngineApiConfig (#2356)\n\n**Motivation**\n\nThe `EnigneApiConfig` isn't used anymore.\n\n**Description**\n\n- Remove the struct and all the deprecated code related to it.\n\nCloses #2351",
          "timestamp": "2025-03-31T20:57:31Z",
          "tree_id": "0f8781b21c0fad2067f9ee0768d5dd09cc2b6db6",
          "url": "https://github.com/lambdaclass/ethrex/commit/b3e3705e7252a3dde1c6edc6da3750ed77b9ac91"
        },
        "date": 1743457619243,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 184452727576,
            "range": "± 1177277487",
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
          "id": "c19b0a046c396f1b6613ce7ed96505c39126c0de",
          "message": "fix(l1, l2): add \"data\" as an alias to the tx input field (#2364)\n\n**Motivation**\n\nOur `GenericTransaction` struct calls the field where calldata goes\n`input`, but some (especially old) eth clients call it `data` instead.\nThis was giving me problems when integrating with some of those clients.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-03-31T21:22:35Z",
          "tree_id": "cd9657709e01fb7305901f1ef55bda58eda676b9",
          "url": "https://github.com/lambdaclass/ethrex/commit/c19b0a046c396f1b6613ce7ed96505c39126c0de"
        },
        "date": 1743459127059,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 184701597004,
            "range": "± 1116702907",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "martin.c.paulucci@gmail.com",
            "name": "Martin Paulucci",
            "username": "mpaulucci"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "ab8f5c324b9999994eb8002497ab667bdd1223ff",
          "message": "build(l2): enable exec prover by default. (#2372)\n\n**Motivation**\nRunning `cargo build --workspace` should work by default, without having\nto explicitly set a feature flag. Without this change, it errors because\nno prover backend was selected. Unless we're working on the prover, we\ndon't really care about the backend and we should reduce the friction to\npeople working in the project.\n\n---------\n\nCo-authored-by: fborello-lambda <federicoborello@lambdaclass.com>",
          "timestamp": "2025-04-01T14:51:03Z",
          "tree_id": "67e19d03eb045812a1a615c5cb45f7f36b115d27",
          "url": "https://github.com/lambdaclass/ethrex/commit/ab8f5c324b9999994eb8002497ab667bdd1223ff"
        },
        "date": 1743522107191,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 183601995227,
            "range": "± 513683522",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "48994069+JereSalo@users.noreply.github.com",
            "name": "Jeremías Salomón",
            "username": "JereSalo"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "e9f9112b01dc30f0b4d651a33c8241a46807db39",
          "message": "docs(l2): make simple changes/nits to docs (#2370)\n\n**Motivation**\n- Improve existing docs a little bit with things I'm noticing while\nreading it.\n\n**Description**\n- Avoid saying we are updating storage in a basic transaction; we are\nactually just updating the balances of the accounts.\n- Make some other small and unimportant changes that improve docs\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-04-01T14:56:29Z",
          "tree_id": "b18489be910f4ac5320cf98188e15bf9ece95fdf",
          "url": "https://github.com/lambdaclass/ethrex/commit/e9f9112b01dc30f0b4d651a33c8241a46807db39"
        },
        "date": 1743522668900,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 188754922429,
            "range": "± 1394860791",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "martin.c.paulucci@gmail.com",
            "name": "Martin Paulucci",
            "username": "mpaulucci"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "65383392d24f31f4a478fe585ab240e534b29528",
          "message": "ci(core): skip loc job on repository forks. (#2373)\n\n**Motivation**\nExternal contributors don't have permissions to post comments\nprogramatically. So the LOC doesn't make sense in that case.",
          "timestamp": "2025-04-01T15:59:46Z",
          "tree_id": "72320c5e6c821e08299b664b050b08317ec20222",
          "url": "https://github.com/lambdaclass/ethrex/commit/65383392d24f31f4a478fe585ab240e534b29528"
        },
        "date": 1743526224231,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 186078010011,
            "range": "± 1250780320",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "98899785+mdqst@users.noreply.github.com",
            "name": "Dmitry",
            "username": "mdqst"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "5b5a11e135c7d4abd5719d6c69397eed17aa626c",
          "message": "chore(l1): fix JWT secret decoding issue (#2298)\n\n**Motivation**  \nI noticed that `hex::decode(secret).unwrap().into()` could cause a panic\nif decoding fails. Since `generate_jwt_secret()` returns a `String`,\n`hex::decode(secret)` produces a `Result<Vec<u8>, FromHexError>`, which\nwas being unwrapped unsafely. Ensuring safe error handling improves the\nrobustness of the code.\n\n**Description**  \nReplaced the unsafe `.unwrap().into()` with a safer decoding approach:  \n\n```rust\nhex::decode(secret)\n    .map(Bytes::from)\n    .expect(\"Failed to decode generated JWT secret\")\n```\n\nThis ensures that any decoding errors are properly surfaced instead of\ncausing a panic.\n\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-04-01T16:48:43Z",
          "tree_id": "9c7a54eff9e6540b37276ab69db353a676726514",
          "url": "https://github.com/lambdaclass/ethrex/commit/5b5a11e135c7d4abd5719d6c69397eed17aa626c"
        },
        "date": 1743529161926,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 185786068027,
            "range": "± 625343763",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "martin.c.paulucci@gmail.com",
            "name": "Martin Paulucci",
            "username": "mpaulucci"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "0c8ae91c53a62d70e8e699ab445e2e89e9d649c6",
          "message": "ci(core): restrict github job permissions by default. (#2389)\n\n**Motivation**\nUse the principle of least privilege and don't grand write permissions\nthat are then forwarded to potentially malicious actions.",
          "timestamp": "2025-04-03T13:06:01Z",
          "tree_id": "c676caacb1b70135bf57d3b629b3a30c9125f864",
          "url": "https://github.com/lambdaclass/ethrex/commit/0c8ae91c53a62d70e8e699ab445e2e89e9d649c6"
        },
        "date": 1743688634472,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 185259852612,
            "range": "± 945855173",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "martin.c.paulucci@gmail.com",
            "name": "Martin Paulucci",
            "username": "mpaulucci"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "73d94a2457e3e1277a7a4736b9d534b5d23fd53d",
          "message": "feat(l1): add hoodi testnet configuration (#2387)\n\n**Motivation**\nAdd support for hoodi testnet",
          "timestamp": "2025-04-03T13:05:25Z",
          "tree_id": "89f1db2b9baf7765d8b419f8bbcef6a890453b35",
          "url": "https://github.com/lambdaclass/ethrex/commit/73d94a2457e3e1277a7a4736b9d534b5d23fd53d"
        },
        "date": 1743688681478,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 193600467450,
            "range": "± 1409107936",
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
          "id": "c9b0dbbe875497eff4c47f928a1e7de10f83059d",
          "message": "feat(l1): adjust byte code batch size (snap sync parameter) (#2338)\n\n**Motivation**\nPrevious changes have sped up other components of the snap sync process,\nmaking faults in the byte code fetcher more evident. The byte code\nfetcher used the same batch size as storage requests, 300, which is far\nmore than the byte codes normally returned by a peer request, causing\nthe byte code fetcher to keep on fetching the last batches when all\nother fetchers have already finished.\nThis PR reduces the batch size down to 70 so that it coincides with the\namount of byte codes regularly returned by peers\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Rename constant `BATCH_SIZE` -> `STORAGE_BATCH_SIZE`\n* Add constant `BYTECODE_BATCH_SIZE`\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-04-03T13:28:35Z",
          "tree_id": "166aff3e46e72fb6a5a4d83faedfc765e01c6e93",
          "url": "https://github.com/lambdaclass/ethrex/commit/c9b0dbbe875497eff4c47f928a1e7de10f83059d"
        },
        "date": 1743689921923,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 184410905603,
            "range": "± 1401636606",
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
          "distinct": false,
          "id": "f3576706e2a45bc96e5709693f4b9453fd6db25d",
          "message": "feat(l1): abstract syncer <-> codebase interaction (#2303)\n\n**Motivation**\nThe codebase (mainly rpc) currently interacts with the synced by trying\nto acquire its lock, which works if we only need to know if the synced\nis busy, but no longer works if we need more precise information about\nthe sync such as what is the mode of the current sync. This PR\nintroduces the `SyncSupervisor` who is in charge of storing the latest\nfcu head, starting and restarting sync cycles and informing the current\nsync status at all times\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #2282",
          "timestamp": "2025-04-03T13:28:05Z",
          "tree_id": "e2d4bc14288b64cff5ad094313010233d5543046",
          "url": "https://github.com/lambdaclass/ethrex/commit/f3576706e2a45bc96e5709693f4b9453fd6db25d"
        },
        "date": 1743689935033,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 185738763425,
            "range": "± 1374578941",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "martin.c.paulucci@gmail.com",
            "name": "Martin Paulucci",
            "username": "mpaulucci"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "065b797d9f2eb422532032d23081b8f61b028fec",
          "message": "ci(l1): add job that makes sure cli is in sync with README. (#2390)\n\n**Motivation**\nAvoid update to the cli code to end up in an outdated README\n\n**Description**\n- Added a job that checks that the help output in the ethrex command\nthat is in the README is in sync with the code.\n\nCloses #2247",
          "timestamp": "2025-04-03T14:19:03Z",
          "tree_id": "94dc2eb9093c38a7e6bd45d594d3a2f7c5115cc7",
          "url": "https://github.com/lambdaclass/ethrex/commit/065b797d9f2eb422532032d23081b8f61b028fec"
        },
        "date": 1743692921453,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 184947992254,
            "range": "± 1037254491",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "martin.c.paulucci@gmail.com",
            "name": "Martin Paulucci",
            "username": "mpaulucci"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "99c544092663bb241c9cf09b07158415658bd966",
          "message": "refactor(l2): remove references to vm internal api. (#2299)\n\n**Motivation**\nL2 code was accessing internal apis from the vm crate, specifically\n`revm` constructs. This is attempt to replace those with the public api,\nso that we can easily switch between revm and levm.\n\n**Description**\n- Replaces references to `ethrex_vm::backends::` from the prover\nbackends.\n- Moved `ExecutionDB ` to `vm/db.rs`. It is still somewhat coupled with\nrevm but less than before. It should be totally decoupled.\n\n---------\n\nCo-authored-by: Javier Chatruc <jrchatruc@gmail.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: Jeremías Salomón <48994069+JereSalo@users.noreply.github.com>\nCo-authored-by: JereSalo <jeresalo17@gmail.com>\nCo-authored-by: fmoletta <99273364+fmoletta@users.noreply.github.com>",
          "timestamp": "2025-04-03T15:31:56Z",
          "tree_id": "e5ee0003621e4a3b89c6d8c759c30963b487504f",
          "url": "https://github.com/lambdaclass/ethrex/commit/99c544092663bb241c9cf09b07158415658bd966"
        },
        "date": 1743697308730,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 186392698099,
            "range": "± 654097985",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}