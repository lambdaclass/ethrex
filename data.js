window.BENCHMARK_DATA = {
  "lastUpdate": 1742326818143,
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
      }
    ]
  }
}