window.BENCHMARK_DATA = {
  "lastUpdate": 1750943694583,
  "repoUrl": "https://github.com/lambdaclass/ethrex",
  "entries": {
    "Benchmark": [
      {
        "commit": {
          "author": {
            "email": "18153834+klaus993@users.noreply.github.com",
            "name": "Klaus @ LambdaClass",
            "username": "klaus993"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "e973e9688f3e0ec9c425eb3c5eb89b3ab5e369fe",
          "message": "ci(l1,l2): publish Ethrex docs on https://docs.ethrex.xyz/ (#3217)\n\n**Motivation**\n\nPublish the mdbook of this repo (book.toml) to https://docs.ethrex.xyz/\n\n**Description**\n\nThese changes are to leave the setup like this:\n\n* https://docs.ethrex.xyz/ will have the mdbook\n* https://docs.ethrex.xyz/benchmarks will have the benchmarks graphs\n* https://docs.ethrex.xyz/flamegraphs will have the flamegraphs",
          "timestamp": "2025-06-18T19:55:44Z",
          "tree_id": "f19b7a45c9e78782d48bfd6e3c88a95e2f7fd5b1",
          "url": "https://github.com/lambdaclass/ethrex/commit/e973e9688f3e0ec9c425eb3c5eb89b3ab5e369fe"
        },
        "date": 1750280257990,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222697754655,
            "range": "± 836006884",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "47506558+MegaRedHand@users.noreply.github.com",
            "name": "Tomás Grüner",
            "username": "MegaRedHand"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "0ed60d3f23dd798a8cfdd2c7989a4364550bcc5d",
          "message": "docs(l2): reorganize docs (#3196)\n\n**Motivation**\n\nOur L2 documentation lacks a clear structure.\n\n**Description**\n\nThis PR reorganizes our L2 docs, also moving documentation on L2\nload-tests under `Developers`->`L2 load-tests`. The rest of the\ndocumentation was restructured to a structure like that of other L1 and\nL2 projects:\n\n<img width=\"297\" alt=\"new structure of L2 docs\"\nsrc=\"https://github.com/user-attachments/assets/b9c89a10-c175-4610-b141-3fa4b0097cfb\"\n/>\n\nDocumentation on smart contracts still needs to be filled and is only\nacting as a placeholder for now.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3174",
          "timestamp": "2025-06-18T20:04:19Z",
          "tree_id": "c8f2674a2a89594515ae4e6882381f043bcf22f7",
          "url": "https://github.com/lambdaclass/ethrex/commit/0ed60d3f23dd798a8cfdd2c7989a4364550bcc5d"
        },
        "date": 1750280874141,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 241927282758,
            "range": "± 701383374",
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
          "id": "a727cd76b9a33364ab8506e2482f97d428209e29",
          "message": "fix(l1): compute logs_bloom when building payloads (#3219)\n\n**Motivation**\n\nOur build payload process was not computing and setting the `logs_bloom`\nfield on the block's header, which resulted in other clients rejecting\nblocks built by us. This came up when testing setting up a localnet with\nethrex along with other clients.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-06-18T21:20:26Z",
          "tree_id": "485552181532d3c75cbcded8d4fabb4a20df0e0e",
          "url": "https://github.com/lambdaclass/ethrex/commit/a727cd76b9a33364ab8506e2482f97d428209e29"
        },
        "date": 1750285229817,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 224179628110,
            "range": "± 271461380",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "18153834+klaus993@users.noreply.github.com",
            "name": "Klaus @ LambdaClass",
            "username": "klaus993"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "02fda58b1e8ee0a7014ba43956c9cd846953c4fb",
          "message": "ci(l1,l2): fix GitHub Pages deployments (#3222)\n\n**Motivation**\n\nFix for #3217\n\n**Description**\n\nFixes lack of permissions for mdbook workflow, and new path to publish\nL1 block proving benchmark",
          "timestamp": "2025-06-19T13:57:23Z",
          "tree_id": "3d602ecb1c7539d9c52e1f10728d21da6ed5a778",
          "url": "https://github.com/lambdaclass/ethrex/commit/02fda58b1e8ee0a7014ba43956c9cd846953c4fb"
        },
        "date": 1750345042436,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222026345384,
            "range": "± 424203482",
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
          "id": "161002ab3f24085e0d6bc335335b3c49f7588b64",
          "message": "refactor(levm): tidy code of benchmarks against revm (#3199)\n\n**Motivation**\n\n- Benchmarks are a key piece for measuring performance, the code wasn't\nvery concise so this simplifies it to make further changes that will\nhelp us work on performance in LEVM.\n\n**Description**\n\nBehavior is pretty much the same, the code is just more clear now.\n\nCloses #issue_number",
          "timestamp": "2025-06-19T14:29:14Z",
          "tree_id": "4f5ad3682010a2e0326bd395a769cba192e7450a",
          "url": "https://github.com/lambdaclass/ethrex/commit/161002ab3f24085e0d6bc335335b3c49f7588b64"
        },
        "date": 1750346960074,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 224019929514,
            "range": "± 497931099",
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
          "distinct": true,
          "id": "b8c6d1fb5880ae7a3f02d65b9efe50035f3b60ce",
          "message": "fix(levm): account was already empty don't count as update if it remains empty (#3228)\n\n**Motivation**\n\nThe l2 committer was stuck because it failed when trying to encode state\ndiffs of an account that was initially empty, remained empty after the\ntransaction so the AccountUpdate was completely empty and state diff\ncreation failed with `StateDiffError::EmptyAccountDiff`\n\n**Description**\n\n- `LEVM::get_state_transitions` now checks if the account was initially\nempty, in case it was and it remains empty after the transaction do not\ncount it as an AccountUpdate",
          "timestamp": "2025-06-19T15:02:18Z",
          "tree_id": "f1969960ea7e1bbb57f510838e542a2fa33c00d5",
          "url": "https://github.com/lambdaclass/ethrex/commit/b8c6d1fb5880ae7a3f02d65b9efe50035f3b60ce"
        },
        "date": 1750348907774,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 223587207590,
            "range": "± 214745076",
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
          "id": "144a5ac6f342a602ed33594b64e0e7c3151a087e",
          "message": "chore(l2): rename estimate_gas error (#3225)\n\n**Motivation**\n\n`EstimateGasPriceError` is actually an error triggered in\n`estimate_gas`.\n\n**Description**\n\nRenames `EstimateGasPriceError` to `EstimateGasError`\n\nCloses None",
          "timestamp": "2025-06-19T19:03:22Z",
          "tree_id": "a79f49ae59594656bd34d6c894e3123d7f39d540",
          "url": "https://github.com/lambdaclass/ethrex/commit/144a5ac6f342a602ed33594b64e0e7c3151a087e"
        },
        "date": 1750363437826,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 224549723732,
            "range": "± 930543060",
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
          "id": "d4ce1f75e56e87936fcc4317a84165c891e67297",
          "message": "refactor(levm): make substate more accurate and replace ExecutionReport for ContextResult in some places (#3134)\n\n**Motivation**\n\n- `ExecutionResult` isn't accurate for interaction between callframes so\nthe goal is to replace it for `ContextResult` that has the necessary\ndata. Also, `Substate` should be as specified in Yellow Paper.\n\n**Description**\n\n- Add logs to substate and remove them from the callframe. They belong\nto the substate according to section 6.1 of the [yellow\npaper](https://ethereum.github.io/yellowpaper/paper.pdf).\n- Replace usage of ExecutionReport in callframes execution for\nContextResult. The former contained data that wasn't necessary and\ncaused a little bit of confusion. In ContextResult we have only the data\nwe need: `gas_used`, `output` and `result`.\n- Move `is_create` logic to `CallFrame`. So now it is not\n`create_op_called`, it is `is_create` and it takes into account external\ntransactions, not only internal `create`.\n- Make functions `handle_opcode_result()` and `handle_opcode_error()`\nprettier.\n- `finalize_execution()` now returns an `ExecutionReport` given a\n`ContextResult`\n- Refactor `increase_consumed_gas()`, behavior is still the same but\nlogic before was kinda repetitive.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3045\n\n---------\n\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: juanbono <juanbono94@gmail.com>\nCo-authored-by: fedacking <francisco.gauna@lambdaclass.com>",
          "timestamp": "2025-06-19T19:11:27Z",
          "tree_id": "465e89628fb4a71504c9921dca14d229d1425ea3",
          "url": "https://github.com/lambdaclass/ethrex/commit/d4ce1f75e56e87936fcc4317a84165c891e67297"
        },
        "date": 1750363891196,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222796180281,
            "range": "± 947121191",
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
          "id": "e04ce47ba6613b66e552cc6e24e31fc4318d6af8",
          "message": "chore(l2): change default dev-mode to false (#3214)\n\n**Motivation**\n\nThe default value of `proof-coordinator.dev-mode` is set to true. This\nmeans the only way to set it to false is through the environment\nvariable `ETHREX_PROOF_COORDINATOR_DEV_MODE`. This is also inconsistent\nwith the rest of the parameters, where we set dev values only in the\nMakefile.\n\n**Description**\n\nChanges the default value of `dev-mode` to false.\n\nCloses None",
          "timestamp": "2025-06-19T19:52:06Z",
          "tree_id": "bee76e43220516b6cf5ed0cdacd4708a4fa0ee05",
          "url": "https://github.com/lambdaclass/ethrex/commit/e04ce47ba6613b66e552cc6e24e31fc4318d6af8"
        },
        "date": 1750366331076,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 225455828118,
            "range": "± 524471056",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "iovoid@users.noreply.github.com",
            "name": "Lucas Fiegl",
            "username": "iovoid"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "fd61888759e225200e36a72f7f162d1b9c0fd54b",
          "message": "feat(l2): batch reversion (#3136)\n\n**Motivation**\n\nAs outlined in #3124, sometimes a committed batch can't be verified or\nthe operator wants to prevent it from going though.\n\n**Description**\n\nThis PR implements a `revertBatch` function that allows reverting back\nto any batch, as long as no verified batches are being discarded.\n\nThere's also a l2 CLI subcommand, revert-batch that lets you revert a\nbatch and remove it from the local database.\n\nUsage on local network:\n```\nPRIVATE_KEY=key cargo run --features l2,rollup_storage_libmdbx -- l2 revert-batch \\\n  <batch to revert to> <OnChainProposer address> \\\n  --datadir dev_ethrex_l2 --network test_data/genesis-l2.json\n```\n\nCloses #3124",
          "timestamp": "2025-06-19T20:14:02Z",
          "tree_id": "8ecbba041a42fa46badba02def789ac144e18ba5",
          "url": "https://github.com/lambdaclass/ethrex/commit/fd61888759e225200e36a72f7f162d1b9c0fd54b"
        },
        "date": 1750367696249,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 223298663557,
            "range": "± 765436772",
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
          "id": "ea1e2089f0468e43906adbb59b164c1646caafca",
          "message": "feat(l1, l2): overwrite txs in mempool if fees are higher (#3238)\n\n**Motivation**\n\nMost Ethereum clients let you speed up or overwrite transactions by\naccepting new transactions with the same nonce but higher fees.\nThis PR adds validations similar to what [Geth\ndoes](https://github.com/ethereum/go-ethereum/blob/09289fd154a45420ec916eb842bfb172df7e0d83/core/txpool/legacypool/list.go#L298-L345)\nbut without the `PriceBump` minimum bump percentage\n\n**Description**\n\n- for eip-1559 check that both `max_fee_per_gas` and\n`max_priority_fee_per_gas` are greater in the new tx\n- for legacy tx check that new `gas_price` is greater in the new tx\n- for eip-4844 txs check that `max_fee_per_gas`,\n`max_priority_fee_per_gas` and `max_fee_per_blob_gas` are grater in the\nnew tx\n\n**How to test**\n\n- Send a tx with very low gas price\n\n```shell\nrex send --gas-price 1 --priority-gas-price 1 --rpc-url http://localhost:1729 0x2B29Bea668B044b2b355C370f85b729bcb43EC40 100000000000000 0x8f87d3aca3eff8132256f69e17df5ba3c605e1b5f4e2071d56f7e6cd66047cc2\n```\n\n- Check tx pool the you should see something like\n`\"maxPriorityFeePerGas\":\"0x1\",\"maxFeePerGas\":\"0x1\",\"gasPrice\":\"0x1\"` the\ntx will probably get stuck\n\n```\ncurl 'http://localhost:1729' --data '{\n  \"id\": 1,\n  \"jsonrpc\": \"2.0\",\n  \"method\": \"txpool_content\",\n  \"params\": []\n}' -H 'accept: application/json' -H 'Content-Type: application/json'\n```\n\n- Send tx with higher gas\n\n```shell\nrex send --gas-price 100000000 --priority-gas-price 100000000 --rpc-url http://localhost:1729 0x2B29Bea668B044b2b355C370f85b729bcb43EC40 100000000000000 0x8f87d3aca3eff8132256f69e17df5ba3c605e1b5f4e2071d56f7e6cd66047cc2\n```\n\n- Check that the tx pool you should see something like\n`\"maxPriorityFeePerGas\":\"0x5f5e100\",\"maxFeePerGas\":\"0x5f5e100\",\"gasPrice\":\"0x5f5e100\"`\n\n```shell\ncurl 'http://localhost:1729' --data '{\n  \"id\": 1,\n  \"jsonrpc\": \"2.0\",\n  \"method\": \"txpool_content\",\n  \"params\": []\n}' -H 'accept: application/json' -H 'Content-Type: application/json'\n```",
          "timestamp": "2025-06-19T21:32:23Z",
          "tree_id": "c44a0cb62bc76999b24168da80db8047ef2f6383",
          "url": "https://github.com/lambdaclass/ethrex/commit/ea1e2089f0468e43906adbb59b164c1646caafca"
        },
        "date": 1750372383264,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 227001233154,
            "range": "± 888735919",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "leanrafa@gmail.com",
            "name": "Leandro Ferrigno",
            "username": "lferrigno"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "0a7f3fd0a48151e6c4f21df437213bc1d7f4ff5f",
          "message": "docs(core): add roadmap to README.md (#3249)\n\nAdd roadmap",
          "timestamp": "2025-06-19T22:27:21Z",
          "tree_id": "dee9e82473e3181e285124d85550ea7f8a6e6179",
          "url": "https://github.com/lambdaclass/ethrex/commit/0a7f3fd0a48151e6c4f21df437213bc1d7f4ff5f"
        },
        "date": 1750375736724,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 224324640504,
            "range": "± 910253094",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "49721261+cdiielsi@users.noreply.github.com",
            "name": "cdiielsi",
            "username": "cdiielsi"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "9c031109d687b14ebdcf9f10e6d32ce4447b0ec7",
          "message": "ci(l1): have failed tests output on the console (#3150)\n\n**Motivation**\n\nThe LEVM CI workflow in pr-main_levm.yaml that runs EF state tests\nshould fail with an exit code if a test fails.\n\n**Description**\nThis pr introduces a new `EFTestRunnerError::TestsFailed` error to use\nwhen there's a report of a test failing. This error is thrown under the\n`summary` flag, which is the one used in the target the CI job executes:\n`make run-evm-ef-tests-ci`. So whenever there is any failing tests, the\nintroduced code should print the EFTestReport and then finish with the\n`EFTestRunnerError::TestsFailed` error.\n\nNote: The `summary` flag is used in other targets as well, so the\npreviously described behavior is being implemented for other targets\ntoo.\n\nThe ef-test-main job in pr-main_levm has also been refactored, I dropped\nsteps \"Check EF-TESTS from Paris to Prague is 100%\" and \"Check EF-TESTS\nstatus is 100%\" since now in the case any test fails, the CI job exits\nwith an error code and outputs the failing tests in the console.\n\nIn this pr there are some commits with a hardcoded error with the\nintentions of having the LEVM CI workflow fail on purpose and check the\nconsole output is the one expected.\n[Here](https://github.com/lambdaclass/ethrex/actions/runs/15738130731/job/44356244936)\nis a failing workflow execution under this circumstances to see. (The\nunderscore line above \"Failed tests\" was removed on a later commit.)\n\nCloses #2887",
          "timestamp": "2025-06-20T08:50:27Z",
          "tree_id": "689e3bdc856a2bb95a8a8a47850f8436dd15a7ca",
          "url": "https://github.com/lambdaclass/ethrex/commit/9c031109d687b14ebdcf9f10e6d32ce4447b0ec7"
        },
        "date": 1750413106548,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 224850805252,
            "range": "± 568814025",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "47506558+MegaRedHand@users.noreply.github.com",
            "name": "Tomás Grüner",
            "username": "MegaRedHand"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "8354c87f8953669c9353e8cdd9349c5c6d707113",
          "message": "docs(core): add `mdbook-mermaid` dependency (#3250)\n\n**Motivation**\n\nWe want to include diagrams in the mdbook. The easiest way to manage\ndiagrams with `git` is to declare them with `mermaid`.\n\n**Description**\n\nThis PR adds [the `mdbook-mermaid`\npreprocessor](https://github.com/badboy/mdbook-mermaid), which\nautomatically renders the mermaid diagrams in our docs.\n\nAs part of this, it also adds make targets to automatically install\npreprocessors/backends, and to generate the files required by\n`mdbook-mermaid`.\n\n<img width=\"836\" alt=\"example mermaid diagram in the L2 docs\"\nsrc=\"https://github.com/user-attachments/assets/d14d57f4-4c73-4c99-82e3-281f1693ee84\"\n/>",
          "timestamp": "2025-06-20T11:02:04Z",
          "tree_id": "81ec6d7511fe34fd1b69bb6a3a24de29a14d7573",
          "url": "https://github.com/lambdaclass/ethrex/commit/8354c87f8953669c9353e8cdd9349c5c6d707113"
        },
        "date": 1750420924372,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 223104416433,
            "range": "± 436107233",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "128638963+santiago-MV@users.noreply.github.com",
            "name": "santiago-MV",
            "username": "santiago-MV"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "42d3f03305a615885ecd2253f1bd0acd09b7e9f3",
          "message": "chore(l1): add metrics port to ethrex client (#3237)\n\n**Motivation**\n\nWhen running a localnet with kurtosis the ethrex client wasn't exposing\na metrics port.\n\n**Description**\n\nTo expose the metrics port, the ETHEREUM_PACKAGE_REVISION in the\nMakefile was updated to the latest commit in our fork of\nethereum-package. Additionally, the metrics feature flag was enabled\nwhen building the Docker image (without it, metrics won't work).\nThe ethereum_metrics_exporter_enabled setting was also enabled for all\nparticipants in the ethrex-only localnet.\nWith these changes, we are now able to use metrics with ethrex clients.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3213",
          "timestamp": "2025-06-20T14:56:01Z",
          "tree_id": "2bcef87ad0a191fe4c912c8ba2757c2cbee887ba",
          "url": "https://github.com/lambdaclass/ethrex/commit/42d3f03305a615885ecd2253f1bd0acd09b7e9f3"
        },
        "date": 1750435004752,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 226016975051,
            "range": "± 1167365906",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "128638963+santiago-MV@users.noreply.github.com",
            "name": "santiago-MV",
            "username": "santiago-MV"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "e42990bee8bf6f99cd70049f09ba2ebad750a559",
          "message": "chore(l1): change error message shown when loading a pre-merge genesis file (#3111)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nWhen using a pre-merge genesis.json for importing blocks, which is not\nsupported by ethrex, the error received was `ParentNotFound`, which\ndoesn't explain the real problem.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nBefore merging blocks the genesis.json fork is checked, in case that its\npre Paris return a custom error message.\nFor doing this new checks were added to the `fork()` function.\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3102\n\n---------\n\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-06-20T17:02:42Z",
          "tree_id": "1792c8f6ba83245d32a6ec768b3b1ed9ff6cd9c0",
          "url": "https://github.com/lambdaclass/ethrex/commit/e42990bee8bf6f99cd70049f09ba2ebad750a559"
        },
        "date": 1750442636166,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 224881683343,
            "range": "± 350122633",
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
          "id": "d9d68ebb22183f63061f4f7f6c4b5a7f3346bdcb",
          "message": "fix(l2): fix l2 integration test job (#3258)\n\n**Motivation**\n\nThis was failing on multiple PRs because the ethrex_dev image was not\nbeing built.\n\nThe difference between the failing job and the others (which were\nsucceeding) is the runner (larger_runners). Maybe that has something to\ndo with it.\n\n**Description**\n\n- adds a step to build ethrex_dev explicitly",
          "timestamp": "2025-06-23T11:36:48Z",
          "tree_id": "d54e246f6e559560bfb9a249ab85b4058eefd0de",
          "url": "https://github.com/lambdaclass/ethrex/commit/d9d68ebb22183f63061f4f7f6c4b5a7f3346bdcb"
        },
        "date": 1750682374766,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 225424239608,
            "range": "± 609027739",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "git@edgl.dev",
            "name": "Edgar",
            "username": "edg-l"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "bca73af8f97978ae202cc25a2e9f08753b82beb6",
          "message": "perf(levm): use specialized PUSH1 and PUSH2 implementations (#3262)\n\n**Motivation**\nAccording to stats from @azteca1998 PUSH2 and PUSH1 are widely used:\n\n```\nLoaded 903636264 rows (3447.10MiB)\nStats (of 903636264 records):\n  0xf1: count=   730979  t_min=  2278  t_max=1512728  t_avg=110877.43  t_acc=81049072024  CALL\n  0x61: count=131856777  t_min=   136  t_max= 549032  t_avg=   189.29  t_acc=24959614846  PUSH2\n  0x56: count= 78745029  t_min=   170  t_max=1488792  t_avg=   243.75  t_acc=19194034756  JUMP\n  0x60: count= 86327863  t_min=   136  t_max= 837080  t_avg=   199.78  t_acc=17246262544  PUSH1\n  0x5b: count=107216057  t_min=   102  t_max= 267308  t_avg=   159.43  t_acc=17093508806  JUMPDEST\n  0x50: count= 86546732  t_min=   102  t_max= 353260  t_avg=   174.49  t_acc=15101132640  POP\n  0x57: count= 53096953  t_min=   102  t_max=1382576  t_avg=   233.40  t_acc=12393069292  JUMPI\n  0x81: count= 55585321  t_min=   102  t_max= 267410  t_avg=   192.79  t_acc=10716509980  DUP2\n  0x01: count= 56493418  t_min=   102  t_max=1431060  t_avg=   189.52  t_acc=10706399944  ADD\n  0x91: count= 31380921  t_min=   102  t_max= 146030  t_avg=   205.38  t_acc= 6444862520  SWAP2\n```\n\nFurthermore i keep seeing `U256::from_big_endian` taking quite some time\non samply so I made specialized PUSH1 and PUSH2 implementations that\navoid that, also using fixed size arrays.\n\nBenchmarks:\n\nHoodi 11k:\n\nmain 9m10.471s\npr 8m25.933s\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number\n\n# Benchmark Results Comparison\n\n#### Benchmark Results: Factorial\n| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |\n|:---|---:|---:|---:|---:|\n| `levm_Factorial_pr` | 634.2 ± 7.3 | 629.6 | 654.2 | 2.71 ± 0.04 |\n| `levm_Factorial` | 726.1 ± 5.2 | 722.5 | 740.1 | 3.11 ± 0.03 |\n| `levm_FactorialRecursive_pr` | 3.567 ± 0.021 | 3.541 | 3.604 | 2.22 ±\n0.05 |\n| `levm_FactorialRecursive` | 3.828 ± 0.035 | 3.775 | 3.889 | 2.39 ±\n0.03 |\n| `levm_Fibonacci_pr` | 629.2 ± 6.4 | 625.7 | 646.9 | 2.99 ± 0.03 |\n| `levm_Fibonacci` | 727.7 ± 6.5 | 722.3 | 743.9 | 3.47 ± 0.03 |\n| `levm_ManyHashes_pr` | 14.9 ± 0.2 | 14.7 | 15.3 | 1.70 ± 0.03 |\n| `levm_ManyHashes` | 16.3 ± 0.1 | 16.2 | 16.4 | 1.87 ± 0.02 |\n| `levm_BubbleSort_pr` | 5.065 ± 0.023 | 5.034 | 5.107 | 1.58 ± 0.01 |\n| `levm_BubbleSort` | 5.508 ± 0.035 | 5.489 | 5.603 | 1.71 ± 0.02 |\n| `levm_ERC20Transfer_pr` | 461.5 ± 1.3 | 459.7 | 463.4 | 1.87 ± 0.03 |\n| `levm_ERC20Transfer` | 487.9 ± 2.4 | 484.1 | 491.0 | 1.99 ± 0.01 |\n| `levm_ERC20Mint_pr` | 306.8 ± 8.9 | 300.1 | 328.5 | 2.22 ± 0.07 |\n| `levm_ERC20Mint` | 320.1 ± 1.5 | 317.9 | 322.6 | 2.31 ± 0.05 |\n| `levm_ERC20Approval_pr` | 1.779 ± 0.023 | 1.763 | 1.838 | 1.69 ± 0.02\n|\n| `levm_ERC20Approval` | 1.850 ± 0.011 | 1.837 | 1.873 | 1.76 ± 0.02 |\n\n\n\n![image](https://github.com/user-attachments/assets/8f08cb93-ac5d-4909-a15d-cf799f1ce023)\n\nAccording to the samply this makes op_push nearly negligible (from 30%\nto 0%)\n\n---------\n\nCo-authored-by: Jeremías Salomón <48994069+JereSalo@users.noreply.github.com>",
          "timestamp": "2025-06-23T13:37:30Z",
          "tree_id": "4b5b5507508c2b381a4e2bd1e96764a389cbe6e3",
          "url": "https://github.com/lambdaclass/ethrex/commit/bca73af8f97978ae202cc25a2e9f08753b82beb6"
        },
        "date": 1750689474181,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 223704530468,
            "range": "± 513180546",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "git@edgl.dev",
            "name": "Edgar",
            "username": "edg-l"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "6f1bb69882588a4b55ed0e7aa4c20b5c5268f6fd",
          "message": "perf(core): use a lookup table for opcode parsing (#3253)\n\n**Motivation**\n\nOn x86_64, rust has a harder time when the match is used, 2 things\nhappen:\n\nWith match;\n- Apparently it also uses a lookup table internally but it doesn't have\nas much \"info\" about what we doing than when doing it manually, for\nexample the function has an extra xor instruction, it also looks like it\nhas more trouble inlining the From\n\nWithout match:\n- No unneeded xor instruction\n- Easier to inline for the compiler (as seen on the godbolt url), this\navoids a full function call.\n\nGodbolt: https://godbolt.org/z/eG8M1jz3M\n\nCloses https://github.com/lambdaclass/ethrex/issues/2896\n\nShould close https://github.com/lambdaclass/ethrex/issues/2896",
          "timestamp": "2025-06-23T14:46:05Z",
          "tree_id": "eb3e02512e563857775cd77cf9f991a3026fdd38",
          "url": "https://github.com/lambdaclass/ethrex/commit/6f1bb69882588a4b55ed0e7aa4c20b5c5268f6fd"
        },
        "date": 1750693596836,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 221549469767,
            "range": "± 375001362",
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
          "id": "48ca855ec68c597b477cb2f81c2b775e9235e865",
          "message": "docs(levm): add type 4 transaction validations (#3085)\n\nAdd type 4 transaction validations to validations.md docs\n\nCloses #2545",
          "timestamp": "2025-06-23T17:09:03Z",
          "tree_id": "60f3912aa663eaf0f0f4a704dc0aa397382e415a",
          "url": "https://github.com/lambdaclass/ethrex/commit/48ca855ec68c597b477cb2f81c2b775e9235e865"
        },
        "date": 1750702102879,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220033548282,
            "range": "± 429821547",
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
          "id": "c7652a1b02e4ebfd149a82c0832a47ebf8837bbd",
          "message": "docs(l1): improve roadmap. (#3271)\n\n**Motivation**\nImprove write up of the L1 roadmap",
          "timestamp": "2025-06-23T19:33:58Z",
          "tree_id": "51a2da2f8a78c54a23d617850ed983b8a3c8900f",
          "url": "https://github.com/lambdaclass/ethrex/commit/c7652a1b02e4ebfd149a82c0832a47ebf8837bbd"
        },
        "date": 1750710866381,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220177160866,
            "range": "± 455972797",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "pdeymon@fi.uba.ar",
            "name": "Pablo Deymonnaz",
            "username": "pablodeymo"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "0c7564a9b26feafe78212f3e036caa1eed13a0d3",
          "message": "chore(levm): remove unused remove_account function from CacheDB (#3278)\n\n**Motivation**\n\nRemove unused method.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-06-23T20:39:44Z",
          "tree_id": "8b7a1d4fc275d8a99a4a607de58d1881f1c0d010",
          "url": "https://github.com/lambdaclass/ethrex/commit/0c7564a9b26feafe78212f3e036caa1eed13a0d3"
        },
        "date": 1750714716475,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 218602871368,
            "range": "± 342520327",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "18153834+klaus993@users.noreply.github.com",
            "name": "Klaus @ LambdaClass",
            "username": "klaus993"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "e0bfe4d38c57287710fe6298cd82be9f46ab67d6",
          "message": "fix(l1,l2): move integration test back to normal GitHub Runners (#3272)\n\n**Motivation**\n\nGo back to the normal GitHub runners, instead of larger runners, because\nthe disk size constraint has been removed from this CI job\n\n**Description**\n\n* Changes `runs-on:` from `integration-test` job in `pr-main_l2.yaml`\nworkflow.\n* Removes actionlint label\n* Removes step and comment related to using the larger runners\n\n---------\n\nCo-authored-by: Javier Chatruc <jrchatruc@gmail.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>",
          "timestamp": "2025-06-23T21:19:03Z",
          "tree_id": "363393f5f08d4e226cc9ddb779cb541cc0411147",
          "url": "https://github.com/lambdaclass/ethrex/commit/e0bfe4d38c57287710fe6298cd82be9f46ab67d6"
        },
        "date": 1750717081028,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 221460825038,
            "range": "± 664909599",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "47506558+MegaRedHand@users.noreply.github.com",
            "name": "Tomás Grüner",
            "username": "MegaRedHand"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "f3698b684da6bdadadda2e66a819486895dbdc31",
          "message": "ci(core): install `mdbook-mermaid` preprocessor (#3281)\n\n**Description**\n\nThis PR changes the mdbook build action to use the `docs-deps` and\n`docs` targets from the makefile to install dependencies and build\ndocumentation, instead of manually doing so like until now.",
          "timestamp": "2025-06-23T21:29:04Z",
          "tree_id": "e94f7d01142df84f63689b78c6c48cdb4a999678",
          "url": "https://github.com/lambdaclass/ethrex/commit/f3698b684da6bdadadda2e66a819486895dbdc31"
        },
        "date": 1750717770487,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222922089882,
            "range": "± 1209322975",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "iovoid@users.noreply.github.com",
            "name": "Lucas Fiegl",
            "username": "iovoid"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "d89fc93fad30c2a58176a1db8375adbc9f731a1e",
          "message": "feat(l2): implement calldata decode (#3204)\n\n**Motivation**\n\nWe want to be able to decode ABI-packed data.\n\n**Description**\n\nThis PR copies the implementation [made for\nrex](https://github.com/lambdaclass/rex/pull/134), with two slicing\noperations fixed to avoid panicking.\n\n---------\n\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>",
          "timestamp": "2025-06-23T22:17:37Z",
          "tree_id": "030b7e3a717064fcc1c308ba72eb76f34a5b1918",
          "url": "https://github.com/lambdaclass/ethrex/commit/d89fc93fad30c2a58176a1db8375adbc9f731a1e"
        },
        "date": 1750720632289,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 218503452289,
            "range": "± 482804344",
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
          "distinct": false,
          "id": "8880fb4c7fc2fcbb5b802a22f430e2ccaeba418c",
          "message": "fix(l2): fix rpc job (#3244)\n\n#3180 happened again\n\nCo-authored-by: LeanSerra <46695152+LeanSerra@users.noreply.github.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>",
          "timestamp": "2025-06-24T01:21:41Z",
          "tree_id": "f1f12afdac43f08caae758cff6ac25325e41c4d0",
          "url": "https://github.com/lambdaclass/ethrex/commit/8880fb4c7fc2fcbb5b802a22f430e2ccaeba418c"
        },
        "date": 1750731670874,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 221032735781,
            "range": "± 438425879",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "46385380+tomasdema@users.noreply.github.com",
            "name": "Tomás Agustín De Mattey",
            "username": "tomasdema"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "c141419d06626fc84a3bbe7d0ce80a0ae4e074ac",
          "message": "docs(core): update roadmap (#3279)\n\n**Motivation**\n\nRoadmap was hard to follow through in a fast read. \n\n**Description**\n\nRearranged the roadmap items to group \"In Progress\" and \"Planned\" tasks\nproperly.",
          "timestamp": "2025-06-24T10:41:47Z",
          "tree_id": "0030a34385742ebeec6505973f379334631f470c",
          "url": "https://github.com/lambdaclass/ethrex/commit/c141419d06626fc84a3bbe7d0ce80a0ae4e074ac"
        },
        "date": 1750765331280,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 221004987563,
            "range": "± 776610129",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "iovoid@users.noreply.github.com",
            "name": "Lucas Fiegl",
            "username": "iovoid"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "37ab4d38fd6d942baa1bc7a0cc88b8746f2c15f5",
          "message": "refactor(l2): use L1Messages for withdrawals (#3187)\n\n**Motivation**\n\nIn preparation to support more complex bridging (such as of ERC20\nassets), we want to use generic L2->L1 messaging primitives that can be\neasily extended and reused.\n\n**Description**\n\nThis replaces withdrawals with a new type L1Message, and has the bridge\nmake use of them.\n\n- Allows for multiple messages per transaction\n- Allows for arbitrary data to be sent. Hashed for easier handling.\n\n---------\n\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>",
          "timestamp": "2025-06-24T13:40:13Z",
          "tree_id": "b5b47bf0beed5ae19cabb4b1ee8cec5c277c50d9",
          "url": "https://github.com/lambdaclass/ethrex/commit/37ab4d38fd6d942baa1bc7a0cc88b8746f2c15f5"
        },
        "date": 1750776095419,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220401265290,
            "range": "± 225467854",
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
          "id": "fe1e166d133f48ea01fca2a0a5c2764f269e7383",
          "message": "fix(l1,l2): improve metrics (#3160)\n\n**Motivation**\n\nOur `transaction_tracker` metric is reset every time the node is\nrestarted.\n\n**Description**\n\n- Uses the `increase()` function in Grafana to avoid resetting the\ncounter on node restarts.\n- Initializes each possible value to 0 when starting the metrics to\nproperly calculate increments.\n- Splits the Transaction panel into two: `Transactions` and `Transaction\nErrors`.\n- Inverts the colors in `Gas Limit Usage` and `Blob Gas Usage`.\n- Pushes transaction metrics only after closing the block in L2, since\ntransactions may be added or removed depending on the state diff size.\n\n**Last Blob Usage**:  Before | After\n\n\n\n<img width=\"312\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/cd8e5471-3fa9-491b-93c0-10cf24da663c\"\n/>\n\n\n<img width=\"324\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/1fe9b992-1d05-4269-86dd-78ec1f885be0\"\n/>\n\n\n**Transactions**\n\n<img width=\"700\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/ff785f62-fc07-406f-8e8e-4d0f2b4d9aa1\"\n/>\n\n**Transaction Errors**\n\n<img width=\"694\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/146d46b0-c22b-4ff4-969d-a57acdc7916b\"\n/>\n\n\n### How to test\n\n1. Start an L2 node with metrics enabled:\n\n```bash\ncd ethrex/crates/l2\nmake init\n```\n2. Go to `http://localhost:3802/` to watch the Grafana dashboard.\n\n3. Restart the node and check that the `Transactions` panel is not\nreset.\n\n```bash\ncrtl + c\nmake init-l2-no-metrics\n```\n\n4. Modify `apply_plain_transactions` in\n`ethrex/crates/blockchain/payload.rs:543` to generate some errors:\n\n```Rust\npub fn apply_plain_transaction(\nhead: &HeadTransaction,\ncontext: &mut PayloadBuildContext,\n) -> Result<Receipt, ChainError> {\n \n      use std::time::{SystemTime, UNIX_EPOCH};\n      \n      let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();\n      let seed = (now.as_secs() ^ now.subsec_nanos() as u64) as usize;\n\n      match seed % 5 {\n            1 => Err(ChainError::ParentNotFound),\n            2 => Err(ChainError::ParentStateNotFound),\n            3 => Err(ChainError::InvalidTransaction(\"tx error\".into())),\n            4 => Err(ChainError::WitnessGeneration(\"witness failure\".into())),\n            _ => Err(ChainError::Custom(\"custom error\".into())),\n      }\n\n}\n```\n\n5. Restart the node and send some transactions:\n\n```bash\ncd ethrex\ncargo run --release --manifest-path ./tooling/load_test/Cargo.toml -- -k ./test_data/private_keys.txt -t eth-transfers -n http://localhost:1729\n```\n\nif necessary run `ulimit -n 65536` before the command.\n\nCloses None",
          "timestamp": "2025-06-24T13:45:49Z",
          "tree_id": "02f89869e2c9ab3e0094dd4337ca7d0decbde7e6",
          "url": "https://github.com/lambdaclass/ethrex/commit/fe1e166d133f48ea01fca2a0a5c2764f269e7383"
        },
        "date": 1750776416130,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 221578918460,
            "range": "± 385628545",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "18153834+klaus993@users.noreply.github.com",
            "name": "Klaus @ LambdaClass",
            "username": "klaus993"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "34c588c5a3cbf71c2ba00796c2d1eef5395ce61e",
          "message": "fix(l1,l2): swap back to standard GitHub runners (#3285)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-06-24T13:55:22Z",
          "tree_id": "82a861ba44af75941fc4adde3a0f107c1c0d8e24",
          "url": "https://github.com/lambdaclass/ethrex/commit/34c588c5a3cbf71c2ba00796c2d1eef5395ce61e"
        },
        "date": 1750776989926,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220687860813,
            "range": "± 781158718",
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
          "id": "b135bebfc1e6d74c3324376def79cffc315a121d",
          "message": "ci(l1,l2): add 'build block' benchmark to PR checks (#2827)\n\n**Motivation**\n\nMake the \"build block\" benchmark run in the CI.\n\n---------\n\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-06-24T14:11:42Z",
          "tree_id": "bf2ab404844a1d0bff5492a42a82bc2839f98c8d",
          "url": "https://github.com/lambdaclass/ethrex/commit/b135bebfc1e6d74c3324376def79cffc315a121d"
        },
        "date": 1750777877832,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220785440981,
            "range": "± 450845566",
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
          "id": "47d56b55960a27de0f9587a0d38c850b64e1611c",
          "message": "fix(l2): use aligned sdk latest release (#3200)\n\n**Motivation**\n\nWe are using a specific `aligned-sdk` commit. Now that we've bumped the\nSP1 version to `v5.0.0`, we can use the latest release.\n\n**Description**\n\n- Uses the latest release of the `aligned-sdk`.\n- Refactors `estimate_gas` since some clients don't allow empty\n`blobVersionedHashes`, and our deployer doesn't work with\n`ethereum-package`.\n- Adds a guide on how to run an Aligned dev environment.\n\n## How to test\n\nRead the new section `How to Run Using an Aligned Dev Environment` in\n`docs/l2/aligned_mode.md`.\n\nCloses #3169",
          "timestamp": "2025-06-24T14:57:38Z",
          "tree_id": "9f2f1ebe1326e01c794f2f4c11d86b9eeb8b11d0",
          "url": "https://github.com/lambdaclass/ethrex/commit/47d56b55960a27de0f9587a0d38c850b64e1611c"
        },
        "date": 1750780805275,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222641182572,
            "range": "± 254618972",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "francisco.gauna@lambdaclass.com",
            "name": "fedacking",
            "username": "fedacking"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "bda9db7998463a96786587f71e73a5a0415e7d02",
          "message": "refactor(l2): implement Metrics Gatherer using spawned library  (#3037)\n\n**Motivation**\n\n[spawned](https://github.com/lambdaclass/spawned) goal is to simplify\nconcurrency implementations and decouple any runtime implementation from\nthe code.\nOn this PR we aim to replace the Metrics Gatherer with a spawned\nimplementation to learn if this approach is beneficial.\n\n**Description**\n\nReplaces Metrics Gatherer task spawn with a series of spawned gen_server\nimplementation.\n\n---------\n\nCo-authored-by: Esteban Dimitroff Hodi <esteban.dimitroff@lambdaclass.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-06-24T16:30:04Z",
          "tree_id": "e250d6362a643f862666a67ea40b2118218cb1af",
          "url": "https://github.com/lambdaclass/ethrex/commit/bda9db7998463a96786587f71e73a5a0415e7d02"
        },
        "date": 1750786181141,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222276700322,
            "range": "± 276327006",
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
          "distinct": false,
          "id": "196a17b1e734d7510cb48192b944361641ea29c3",
          "message": "chore(l2): remove execution cache (#3091)\n\n**Motivation**\n\nWe can use the rollup store for this purpose (by adding a table to store\naccount updates)\n\n**Description**\n\n- deletes `ExecutionCache`, replaces it with the `StoreRollup`\n- adds new tables in all store backends for storing account updates\n- now the block producer has a reference to the rollup store, to push\naccount updates\n\n---------\n\nCo-authored-by: Tomás Paradelo <112426153+tomip01@users.noreply.github.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>",
          "timestamp": "2025-06-24T16:46:08Z",
          "tree_id": "2da6943241aaa38aba591c90dbd14323dd54d0ab",
          "url": "https://github.com/lambdaclass/ethrex/commit/196a17b1e734d7510cb48192b944361641ea29c3"
        },
        "date": 1750787186615,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220310290530,
            "range": "± 857262126",
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
          "id": "d671a809973d3c60d14fe06f85161ceb93e87875",
          "message": "refactor(l2): use hardcoded vk in Aligned mode (#3175)\n\n**Motivation**\n\nWe are passing the verification key every time we call\n`verifyBatchAligned()`.\n\n**Description**\n\n- Initializes `SP1_VERIFICATION_KEY` in the `OnChainProposer` contract\nwith the Aligned vk and reuses it in `verifyBatchAligned()`.\n- Since `l1_proof_verifier` needs the vk for\n`aligned_sdk:check_proof_verification()`, it retrieves it from the\ncontract as well.\n\n\nCloses #3030\n\n---------\n\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-06-24T19:04:02Z",
          "tree_id": "b4ce937a9867ae5012962ef2ec8d4fcebc0f5c5a",
          "url": "https://github.com/lambdaclass/ethrex/commit/d671a809973d3c60d14fe06f85161ceb93e87875"
        },
        "date": 1750795407696,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220661716482,
            "range": "± 296977084",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "iovoid@users.noreply.github.com",
            "name": "Lucas Fiegl",
            "username": "iovoid"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "5b246bd60fbb0aec2b3bbe4680fd22f5f1a1167e",
          "message": "feat(l2): implement SQL backend for L2 store (#3093)\n\n**Motivation**\n\nWe want a SQL-based backend for easier inspection.\n\n**Description**\n\nImplements a SQLite-like backend using libSQL.\n\nRemoves the RefUnwindSafe requirement from the rollup store, where it\nwasn't needed.\n\nRefactors usages of StoreError in the rollup store into\nRollupStoreError, to avoid cluttering the Store with features it doesn't\nimplement.",
          "timestamp": "2025-06-24T19:42:10Z",
          "tree_id": "452b4b3246e2829b09b3e8cb3aea5073a8426aeb",
          "url": "https://github.com/lambdaclass/ethrex/commit/5b246bd60fbb0aec2b3bbe4680fd22f5f1a1167e"
        },
        "date": 1750797738929,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220463701192,
            "range": "± 816678874",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "18153834+klaus993@users.noreply.github.com",
            "name": "Klaus @ LambdaClass",
            "username": "klaus993"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "2d24185147fc5be9f2b98afb3ab8351cdd44d851",
          "message": "fix(l1,l2): update Rust version to 1.87.0 in every CI workflow, parametrize it by putting it in a GitHub Variable (#3284)\n\n**Motivation**\n\n* There are [17 CI\nfiles](https://github.com/search?q=repo%3Alambdaclass%2Fethrex+dtolnay%2Frust-toolchain&type=code)\nthat use the [`dtolnay/rust-toolchain` GitHub\nAction](https://github.com/dtolnay/rust-toolchain).\n* Each one of these sets up Rust in the GitHub runner, and a version\nspecification is needed\n* By storing the unified Rust version in a [GitHub\nVariable](https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/store-information-in-variables)\nwe don't need to update 17 files each time we need to update the Rust\nversion\n\n**Description**\n\nPreviously, we were using the action this way: `uses:\ndtolnay/rust-toolchain@<rust_version>`.\nFor example: `dtolnay/rust-toolchain@1.87.0`.\nThis is the easy way of pinning a Rust version using this GitHub Action.\n\nAs we want to parametrize this and put it in a variable, we need to\nremove the version specification from the GitHub Workflow YAML `uses:`,\nas GitHub Actions syntax doesn't accept putting expressions in there.\nThe `@<version>` actually means \"get this GitHub Action from the target\nrepository **_branch_**\", so we can just use the action version [from\ntheir `master`\nbranch](https://github.com/dtolnay/rust-toolchain/tree/master) and\nspecify the version with a setting below like this:\n\n```yaml\nuses: dtolnay/rust-toolchain@master\n  with:\n    toolchain: ${{ vars.RUST_VERSION }}\n```\n\nSo we can use the GitHub Variable RUST_VERSION to store it. If there is\nany special case which needs a different version, we can just put the\nversion directly inside the `toolchain:` spec.",
          "timestamp": "2025-06-24T20:14:19Z",
          "tree_id": "212d98bd740accf3ec735b591d3862c66a407026",
          "url": "https://github.com/lambdaclass/ethrex/commit/2d24185147fc5be9f2b98afb3ab8351cdd44d851"
        },
        "date": 1750799616060,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222492241004,
            "range": "± 279592992",
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
          "id": "e846b11033de83d369ba5a9ab1621cbef7d3307a",
          "message": "chore(l2): remove redundant checks in contracts (#3282)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nWe have a bunch of redundant and too cautious `require`s in the\ncontracts that increase the cost of deployment and don't provide too\nmuch value.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n- Remove checks for storage slots different from 0, since the\n`initializer` modifier already handle those cases.\n- Remove checks for addresses different from the contract itself, since\nthey don't add too much value.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>",
          "timestamp": "2025-06-24T20:16:33Z",
          "tree_id": "09abaafea256af08890263ce17c8eb48f5fead5a",
          "url": "https://github.com/lambdaclass/ethrex/commit/e846b11033de83d369ba5a9ab1621cbef7d3307a"
        },
        "date": 1750799748733,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222279872168,
            "range": "± 353295224",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "47506558+MegaRedHand@users.noreply.github.com",
            "name": "Tomás Grüner",
            "username": "MegaRedHand"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "5a11dda349151747fe86e0d627823a4353964a77",
          "message": "chore(l2): remove nested workspace (#3277)\n\n**Motivation**\n\nWhen installing with `cargo install --git\nhttps://github.com/lambdaclass/ethrex.git ethrex`, I was getting weird\nerrors from the `zkvm_interface` crate. Looking into that, I found that\nwe have a nested workspace in our repo. This is not supported, and hence\nshould be avoided.\n\n**Description**\n\nThis PR removes the second workspace. I also tried keeping some of the\noptimization options for the package in\n4635cfec948a12e52d6a61ec317781845714838c, but the `lto` option was\ngiving me problems. In case we want to re-add them, the other options\ncan be added directly to the main `Cargo.toml` instead of a\n`config.toml`.",
          "timestamp": "2025-06-24T20:36:01Z",
          "tree_id": "740b19aaa6ca5fd2b08951082d8a0b58ef8c5f15",
          "url": "https://github.com/lambdaclass/ethrex/commit/5a11dda349151747fe86e0d627823a4353964a77"
        },
        "date": 1750800995655,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222105366835,
            "range": "± 599026439",
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
          "id": "9fab1cb7c2369d6b762d1e0aa775af55498fafac",
          "message": "fix(l2): remove withdrawals processing in build_payload (#3297)\n\n**Motivation**\n\nWhen producing a block in L2 mode, we unnecessarily call\n`blockchain::apply_withdrawals`, even though we don't have a consensus\nnode passing withdrawals via the `ForkChoiceUpdated` engine message. In\nfact, we create a default value\n[here](https://github.com/lambdaclass/ethrex/blob/d671a809973d3c60d14fe06f85161ceb93e87875/crates/l2/sequencer/block_producer.rs#L171).\n\n**Description**\n\n- Removes the call to `blockchain::apply_withdrawals()` in\n`payload_builder`.\n\nCloses None",
          "timestamp": "2025-06-24T20:50:06Z",
          "tree_id": "4ee96703bdb1a355573ab1ccdb22a8f0ee2c08a5",
          "url": "https://github.com/lambdaclass/ethrex/commit/9fab1cb7c2369d6b762d1e0aa775af55498fafac"
        },
        "date": 1750801855950,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220352185029,
            "range": "± 300589884",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "rodrigooliveri10@gmail.com",
            "name": "Rodrigo Oliveri",
            "username": "rodrigo-o"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "5a39b693d285690b479657d69b0939f03bd5075f",
          "message": "feat(l1): running localnet with client comparisions (#3221)\n\n**Motivation**\n\nWe want to be able to spin up a local network with 3 nodes,\nethrex[levm], ethrex[revm] and reth an be able to compare throughput\nbetween them on different spamoor configurations.\n\n**Description**\n\nThis PR add a new make target that spin-up a localnet with ethrex[levm],\nethrex[revm] and reth, enable the metrics exporter, and prepare the\ndashboard to be able to be used in arbitrary datasources, both imported\nor provisioned, automatically setting a datasource variable to the\ndefault prometheus.",
          "timestamp": "2025-06-24T22:29:24Z",
          "tree_id": "55758539e0bfbaa198ef96bf470eeeae9546ec45",
          "url": "https://github.com/lambdaclass/ethrex/commit/5a39b693d285690b479657d69b0939f03bd5075f"
        },
        "date": 1750807764503,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220274983454,
            "range": "± 790969873",
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
          "id": "79ed215def989e62e92653c9d0226cc93d941878",
          "message": "ci(l2): check if the sp1 `Cargo.lock` is modified but not committed (#3302)\n\n**Motivation**\n\nRPC prover ci is constantly breaking because Cargo.lock is modified but\nnot committed in PRs\n\n**Description**\n\n- Add a check in the Lint job that executes `git diff --exit-code --\ncrates/l2/prover/zkvm/interface/sp1/Cargo.lock` and fails if there is a\ndiff\n- Update `Cargo.lock` to fix currently broken ci\n- Example of a failed run:\n-\nhttps://github.com/lambdaclass/ethrex/actions/runs/15863233694/job/44724951989",
          "timestamp": "2025-06-25T13:45:24Z",
          "tree_id": "9bf943d70d37d3e05cdab7bce5d099e5632017a0",
          "url": "https://github.com/lambdaclass/ethrex/commit/79ed215def989e62e92653c9d0226cc93d941878"
        },
        "date": 1750862710367,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222182231512,
            "range": "± 614814378",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "49721261+cdiielsi@users.noreply.github.com",
            "name": "cdiielsi",
            "username": "cdiielsi"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "e84f61faddbd32288af58db8ca3c56aa4f3541d7",
          "message": "test(l1): check if vm state reverts correctly on error (#3198)\n\n**Motivation**\n\nPer the instructions of the [ethereum state test\ndocs](https://eest.ethereum.org/v3.0.0/consuming_tests/state_test/), we\nshould be reverting to the pre-state when the execution throws an\nexception. Levm does this, but it's not asserted in the test runner in\nthe case an exception is expected.\n\n**Description**\n\nThis pr introduces a new error to check if the state was reverted\ncorrectly in the case an exception must occur, or throw error otherwise.\nTo check if the state was correctly reverted I'm using the post state\nhash from the tests and comparing it with the hash of the account's\nlatest state recorded in the db.\n\nCloses #2604",
          "timestamp": "2025-06-25T14:02:28Z",
          "tree_id": "45788c1aac7394b905b3c9f02a81cefdd1b332e6",
          "url": "https://github.com/lambdaclass/ethrex/commit/e84f61faddbd32288af58db8ca3c56aa4f3541d7"
        },
        "date": 1750863715235,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220230552362,
            "range": "± 351014067",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "azteca1998@users.noreply.github.com",
            "name": "MrAzteca",
            "username": "azteca1998"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "3a18678adbe6be41af3e05ba0f089d40d9618aed",
          "message": "perf(levm): refactor `gas_used` to `gas_remaining` (#3256)\n\n**Motivation**\n\nBy using `gas_used` there have to be multiple operations for each gas\ncheck. Replacing it with `gas_remaining`, the same overflow check can be\nused to determine whether there was enough gas or not.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-06-25T16:36:49Z",
          "tree_id": "a2ca8debd174ceddfe5037692d47e3855436a185",
          "url": "https://github.com/lambdaclass/ethrex/commit/3a18678adbe6be41af3e05ba0f089d40d9618aed"
        },
        "date": 1750872990523,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220022053969,
            "range": "± 586341724",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "leanrafa@gmail.com",
            "name": "Leandro Ferrigno",
            "username": "lferrigno"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "f673b693b3fcdc1fe4b87fa2abcce8b660cce0a6",
          "message": "docs(l1,l2): move readme components to documentation in mdbook (#3295)\n\n**Motivation**\n\nThe main idea is to only have a quick introduction to ethrex in the\nreadme, with a quick way to set up a local L1+L2 stack from scratch\n(without even having the repo). The rest should be links to the book.\nThe current readme documentation should be moved to the book.\n\n\nCloses #3289\n\n---------\n\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-06-25T20:41:54Z",
          "tree_id": "75b09dedeeaaf00b66d40241c96d6e79b6920788",
          "url": "https://github.com/lambdaclass/ethrex/commit/f673b693b3fcdc1fe4b87fa2abcce8b660cce0a6"
        },
        "date": 1750887740716,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 219117931766,
            "range": "± 938841928",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "89949621+ricomateo@users.noreply.github.com",
            "name": "Mateo Rico",
            "username": "ricomateo"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "f9b9008d8319607d894bd2410d7e4b0964e62314",
          "message": "refactor(l1): `SIGTERM` handling (#3288)\n\n**Motivation**\nStopping an L1 Docker container sends a `SIGTERM` to the node process,\nbut since the node doesn't handle this signal, it takes 10 seconds for\nthe `SIGKILL` to force the shutdown.\nSome temporary fixes were introduced to address this issue, but the\ncorrect solution is to handle the `SIGTERM` signal.\n\n\n**Description**\nThis PR adds a `SIGTERM` handler to allow graceful shutdown without the\n10-second delay.\n\nIt also reverts the temporary fixes that were introduced in previous PRs\n(https://github.com/lambdaclass/ethrex/commit/47a1d4c5b23a9ae03839556b92bce3c6dd029f17\nand\nhttps://github.com/lambdaclass/ethrex/commit/67edcaff73624446f2b75c40b385df69aabe4882)\nin favor of this solution.\n\nNote that when the node is syncing, shutdown isn’t immediate, as the\nnode waits for the current batch to complete.\nTo handle this, the `CancellationToken` is propagated and checked before\nprocessing each block, which allows the node to terminate immediately,\nwithout waiting for the whole batch to complete.\n\nCloses #2944\nCloses #3236",
          "timestamp": "2025-06-25T21:29:38Z",
          "tree_id": "78e8790824f063020edda3f7bc0be6531f18bd8b",
          "url": "https://github.com/lambdaclass/ethrex/commit/f9b9008d8319607d894bd2410d7e4b0964e62314"
        },
        "date": 1750890571535,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 219171300772,
            "range": "± 583199276",
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
          "id": "e03f3b0011184f47542db125afa33039176dd1c0",
          "message": "docs(l2): update based roadmap (#3319)\n\n> [!NOTE]\n> This is not the final version of the latest update.\n\n---------\n\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: Manuel Iñaki Bilbao <manuel.bilbao@lambdaclass.com>",
          "timestamp": "2025-06-25T21:46:19Z",
          "tree_id": "78387864124b8f1e738172616e71c4dd1699b546",
          "url": "https://github.com/lambdaclass/ethrex/commit/e03f3b0011184f47542db125afa33039176dd1c0"
        },
        "date": 1750891514052,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 219892999296,
            "range": "± 444798092",
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
          "id": "3eee599280855b30cfd023ecce4a706ebe69d3a5",
          "message": "docs(l2): update image and add suggestions (#3328)\n\nCo-authored-by: Javier Chatruc <jrchatruc@gmail.com>",
          "timestamp": "2025-06-25T23:11:27Z",
          "tree_id": "e83b3b37045190a2d8c8a3425969a1e070a94eb0",
          "url": "https://github.com/lambdaclass/ethrex/commit/3eee599280855b30cfd023ecce4a706ebe69d3a5"
        },
        "date": 1750896711967,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220985078809,
            "range": "± 605677520",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "azteca1998@users.noreply.github.com",
            "name": "MrAzteca",
            "username": "azteca1998"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "217ad13acdaa6d7219ec4a5ad16fce5164749ae8",
          "message": "perf(levm): refactor `Stack` to have a fixed size and grow downwards (#3266)\n\n**Motivation**\n\nThe stack is currently implemented using a `Vec<U256>` that grows\nupwards. Since the stack is limited to a reasonable amount by design\n(1024 entries, or 32KiB) it can be converted to a static array along\nwith an offset and made to grow downwards.\n\n**Description**\n\nThis PR:\n- Removes stack allocation (and its associated runtime checks) from the\nruntime costs.\n- Makes the stack grow downwards: better integration with stack\noperations.\n- Changes the API so that multiple items can be inserted and removed\nfrom the stack at the same time, which is especially useful for opcodes\nthat handle multiple arguments or return values.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number\n\n---------\n\nCo-authored-by: Jeremías Salomón <48994069+JereSalo@users.noreply.github.com>",
          "timestamp": "2025-06-26T12:02:50Z",
          "tree_id": "5ac3d6fb4b5de3b973e523181a199aa9ab11338c",
          "url": "https://github.com/lambdaclass/ethrex/commit/217ad13acdaa6d7219ec4a5ad16fce5164749ae8"
        },
        "date": 1750943014631,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 221781615350,
            "range": "± 432103051",
            "unit": "ns/iter"
          }
        ]
      }
    ],
    "L1 block proving benchmark": [
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
          "distinct": false,
          "id": "8880fb4c7fc2fcbb5b802a22f430e2ccaeba418c",
          "message": "fix(l2): fix rpc job (#3244)\n\n#3180 happened again\n\nCo-authored-by: LeanSerra <46695152+LeanSerra@users.noreply.github.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>",
          "timestamp": "2025-06-24T01:21:41Z",
          "tree_id": "f1f12afdac43f08caae758cff6ac25325e41c4d0",
          "url": "https://github.com/lambdaclass/ethrex/commit/8880fb4c7fc2fcbb5b802a22f430e2ccaeba418c"
        },
        "date": 1750731092217,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.008671909190974133,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "46385380+tomasdema@users.noreply.github.com",
            "name": "Tomás Agustín De Mattey",
            "username": "tomasdema"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "c141419d06626fc84a3bbe7d0ce80a0ae4e074ac",
          "message": "docs(core): update roadmap (#3279)\n\n**Motivation**\n\nRoadmap was hard to follow through in a fast read. \n\n**Description**\n\nRearranged the roadmap items to group \"In Progress\" and \"Planned\" tasks\nproperly.",
          "timestamp": "2025-06-24T10:41:47Z",
          "tree_id": "0030a34385742ebeec6505973f379334631f470c",
          "url": "https://github.com/lambdaclass/ethrex/commit/c141419d06626fc84a3bbe7d0ce80a0ae4e074ac"
        },
        "date": 1750765876712,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.008648111416026345,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "iovoid@users.noreply.github.com",
            "name": "Lucas Fiegl",
            "username": "iovoid"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "37ab4d38fd6d942baa1bc7a0cc88b8746f2c15f5",
          "message": "refactor(l2): use L1Messages for withdrawals (#3187)\n\n**Motivation**\n\nIn preparation to support more complex bridging (such as of ERC20\nassets), we want to use generic L2->L1 messaging primitives that can be\neasily extended and reused.\n\n**Description**\n\nThis replaces withdrawals with a new type L1Message, and has the bridge\nmake use of them.\n\n- Allows for multiple messages per transaction\n- Allows for arbitrary data to be sent. Hashed for easier handling.\n\n---------\n\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>",
          "timestamp": "2025-06-24T13:40:13Z",
          "tree_id": "b5b47bf0beed5ae19cabb4b1ee8cec5c277c50d9",
          "url": "https://github.com/lambdaclass/ethrex/commit/37ab4d38fd6d942baa1bc7a0cc88b8746f2c15f5"
        },
        "date": 1750775695394,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.008681465013774104,
            "unit": "Mgas/s"
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
          "id": "fe1e166d133f48ea01fca2a0a5c2764f269e7383",
          "message": "fix(l1,l2): improve metrics (#3160)\n\n**Motivation**\n\nOur `transaction_tracker` metric is reset every time the node is\nrestarted.\n\n**Description**\n\n- Uses the `increase()` function in Grafana to avoid resetting the\ncounter on node restarts.\n- Initializes each possible value to 0 when starting the metrics to\nproperly calculate increments.\n- Splits the Transaction panel into two: `Transactions` and `Transaction\nErrors`.\n- Inverts the colors in `Gas Limit Usage` and `Blob Gas Usage`.\n- Pushes transaction metrics only after closing the block in L2, since\ntransactions may be added or removed depending on the state diff size.\n\n**Last Blob Usage**:  Before | After\n\n\n\n<img width=\"312\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/cd8e5471-3fa9-491b-93c0-10cf24da663c\"\n/>\n\n\n<img width=\"324\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/1fe9b992-1d05-4269-86dd-78ec1f885be0\"\n/>\n\n\n**Transactions**\n\n<img width=\"700\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/ff785f62-fc07-406f-8e8e-4d0f2b4d9aa1\"\n/>\n\n**Transaction Errors**\n\n<img width=\"694\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/146d46b0-c22b-4ff4-969d-a57acdc7916b\"\n/>\n\n\n### How to test\n\n1. Start an L2 node with metrics enabled:\n\n```bash\ncd ethrex/crates/l2\nmake init\n```\n2. Go to `http://localhost:3802/` to watch the Grafana dashboard.\n\n3. Restart the node and check that the `Transactions` panel is not\nreset.\n\n```bash\ncrtl + c\nmake init-l2-no-metrics\n```\n\n4. Modify `apply_plain_transactions` in\n`ethrex/crates/blockchain/payload.rs:543` to generate some errors:\n\n```Rust\npub fn apply_plain_transaction(\nhead: &HeadTransaction,\ncontext: &mut PayloadBuildContext,\n) -> Result<Receipt, ChainError> {\n \n      use std::time::{SystemTime, UNIX_EPOCH};\n      \n      let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();\n      let seed = (now.as_secs() ^ now.subsec_nanos() as u64) as usize;\n\n      match seed % 5 {\n            1 => Err(ChainError::ParentNotFound),\n            2 => Err(ChainError::ParentStateNotFound),\n            3 => Err(ChainError::InvalidTransaction(\"tx error\".into())),\n            4 => Err(ChainError::WitnessGeneration(\"witness failure\".into())),\n            _ => Err(ChainError::Custom(\"custom error\".into())),\n      }\n\n}\n```\n\n5. Restart the node and send some transactions:\n\n```bash\ncd ethrex\ncargo run --release --manifest-path ./tooling/load_test/Cargo.toml -- -k ./test_data/private_keys.txt -t eth-transfers -n http://localhost:1729\n```\n\nif necessary run `ulimit -n 65536` before the command.\n\nCloses None",
          "timestamp": "2025-06-24T13:45:49Z",
          "tree_id": "02f89869e2c9ab3e0094dd4337ca7d0decbde7e6",
          "url": "https://github.com/lambdaclass/ethrex/commit/fe1e166d133f48ea01fca2a0a5c2764f269e7383"
        },
        "date": 1750777906012,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.008797799553322166,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "18153834+klaus993@users.noreply.github.com",
            "name": "Klaus @ LambdaClass",
            "username": "klaus993"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "34c588c5a3cbf71c2ba00796c2d1eef5395ce61e",
          "message": "fix(l1,l2): swap back to standard GitHub runners (#3285)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-06-24T13:55:22Z",
          "tree_id": "82a861ba44af75941fc4adde3a0f107c1c0d8e24",
          "url": "https://github.com/lambdaclass/ethrex/commit/34c588c5a3cbf71c2ba00796c2d1eef5395ce61e"
        },
        "date": 1750780926287,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00871025925925926,
            "unit": "Mgas/s"
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
          "id": "b135bebfc1e6d74c3324376def79cffc315a121d",
          "message": "ci(l1,l2): add 'build block' benchmark to PR checks (#2827)\n\n**Motivation**\n\nMake the \"build block\" benchmark run in the CI.\n\n---------\n\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-06-24T14:11:42Z",
          "tree_id": "bf2ab404844a1d0bff5492a42a82bc2839f98c8d",
          "url": "https://github.com/lambdaclass/ethrex/commit/b135bebfc1e6d74c3324376def79cffc315a121d"
        },
        "date": 1750784806791,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00878308751393534,
            "unit": "Mgas/s"
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
          "id": "47d56b55960a27de0f9587a0d38c850b64e1611c",
          "message": "fix(l2): use aligned sdk latest release (#3200)\n\n**Motivation**\n\nWe are using a specific `aligned-sdk` commit. Now that we've bumped the\nSP1 version to `v5.0.0`, we can use the latest release.\n\n**Description**\n\n- Uses the latest release of the `aligned-sdk`.\n- Refactors `estimate_gas` since some clients don't allow empty\n`blobVersionedHashes`, and our deployer doesn't work with\n`ethereum-package`.\n- Adds a guide on how to run an Aligned dev environment.\n\n## How to test\n\nRead the new section `How to Run Using an Aligned Dev Environment` in\n`docs/l2/aligned_mode.md`.\n\nCloses #3169",
          "timestamp": "2025-06-24T14:57:38Z",
          "tree_id": "9f2f1ebe1326e01c794f2f4c11d86b9eeb8b11d0",
          "url": "https://github.com/lambdaclass/ethrex/commit/47d56b55960a27de0f9587a0d38c850b64e1611c"
        },
        "date": 1750787886382,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00871989983397897,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "francisco.gauna@lambdaclass.com",
            "name": "fedacking",
            "username": "fedacking"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "bda9db7998463a96786587f71e73a5a0415e7d02",
          "message": "refactor(l2): implement Metrics Gatherer using spawned library  (#3037)\n\n**Motivation**\n\n[spawned](https://github.com/lambdaclass/spawned) goal is to simplify\nconcurrency implementations and decouple any runtime implementation from\nthe code.\nOn this PR we aim to replace the Metrics Gatherer with a spawned\nimplementation to learn if this approach is beneficial.\n\n**Description**\n\nReplaces Metrics Gatherer task spawn with a series of spawned gen_server\nimplementation.\n\n---------\n\nCo-authored-by: Esteban Dimitroff Hodi <esteban.dimitroff@lambdaclass.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-06-24T16:30:04Z",
          "tree_id": "e250d6362a643f862666a67ea40b2118218cb1af",
          "url": "https://github.com/lambdaclass/ethrex/commit/bda9db7998463a96786587f71e73a5a0415e7d02"
        },
        "date": 1750790092150,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.008773306792873052,
            "unit": "Mgas/s"
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
          "id": "79ed215def989e62e92653c9d0226cc93d941878",
          "message": "ci(l2): check if the sp1 `Cargo.lock` is modified but not committed (#3302)\n\n**Motivation**\n\nRPC prover ci is constantly breaking because Cargo.lock is modified but\nnot committed in PRs\n\n**Description**\n\n- Add a check in the Lint job that executes `git diff --exit-code --\ncrates/l2/prover/zkvm/interface/sp1/Cargo.lock` and fails if there is a\ndiff\n- Update `Cargo.lock` to fix currently broken ci\n- Example of a failed run:\n-\nhttps://github.com/lambdaclass/ethrex/actions/runs/15863233694/job/44724951989",
          "timestamp": "2025-06-25T13:45:24Z",
          "tree_id": "9bf943d70d37d3e05cdab7bce5d099e5632017a0",
          "url": "https://github.com/lambdaclass/ethrex/commit/79ed215def989e62e92653c9d0226cc93d941878"
        },
        "date": 1750863304219,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.008758676486937187,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "49721261+cdiielsi@users.noreply.github.com",
            "name": "cdiielsi",
            "username": "cdiielsi"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "e84f61faddbd32288af58db8ca3c56aa4f3541d7",
          "message": "test(l1): check if vm state reverts correctly on error (#3198)\n\n**Motivation**\n\nPer the instructions of the [ethereum state test\ndocs](https://eest.ethereum.org/v3.0.0/consuming_tests/state_test/), we\nshould be reverting to the pre-state when the execution throws an\nexception. Levm does this, but it's not asserted in the test runner in\nthe case an exception is expected.\n\n**Description**\n\nThis pr introduces a new error to check if the state was reverted\ncorrectly in the case an exception must occur, or throw error otherwise.\nTo check if the state was correctly reverted I'm using the post state\nhash from the tests and comparing it with the hash of the account's\nlatest state recorded in the db.\n\nCloses #2604",
          "timestamp": "2025-06-25T14:02:28Z",
          "tree_id": "45788c1aac7394b905b3c9f02a81cefdd1b332e6",
          "url": "https://github.com/lambdaclass/ethrex/commit/e84f61faddbd32288af58db8ca3c56aa4f3541d7"
        },
        "date": 1750866764638,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.008802714525139666,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "azteca1998@users.noreply.github.com",
            "name": "MrAzteca",
            "username": "azteca1998"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "3a18678adbe6be41af3e05ba0f089d40d9618aed",
          "message": "perf(levm): refactor `gas_used` to `gas_remaining` (#3256)\n\n**Motivation**\n\nBy using `gas_used` there have to be multiple operations for each gas\ncheck. Replacing it with `gas_remaining`, the same overflow check can be\nused to determine whether there was enough gas or not.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-06-25T16:36:49Z",
          "tree_id": "a2ca8debd174ceddfe5037692d47e3855436a185",
          "url": "https://github.com/lambdaclass/ethrex/commit/3a18678adbe6be41af3e05ba0f089d40d9618aed"
        },
        "date": 1750873696210,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00881256096196868,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "leanrafa@gmail.com",
            "name": "Leandro Ferrigno",
            "username": "lferrigno"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "f673b693b3fcdc1fe4b87fa2abcce8b660cce0a6",
          "message": "docs(l1,l2): move readme components to documentation in mdbook (#3295)\n\n**Motivation**\n\nThe main idea is to only have a quick introduction to ethrex in the\nreadme, with a quick way to set up a local L1+L2 stack from scratch\n(without even having the repo). The rest should be links to the book.\nThe current readme documentation should be moved to the book.\n\n\nCloses #3289\n\n---------\n\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-06-25T20:41:54Z",
          "tree_id": "75b09dedeeaaf00b66d40241c96d6e79b6920788",
          "url": "https://github.com/lambdaclass/ethrex/commit/f673b693b3fcdc1fe4b87fa2abcce8b660cce0a6"
        },
        "date": 1750888299555,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00877819442896936,
            "unit": "Mgas/s"
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
          "id": "3eee599280855b30cfd023ecce4a706ebe69d3a5",
          "message": "docs(l2): update image and add suggestions (#3328)\n\nCo-authored-by: Javier Chatruc <jrchatruc@gmail.com>",
          "timestamp": "2025-06-25T23:11:27Z",
          "tree_id": "e83b3b37045190a2d8c8a3425969a1e070a94eb0",
          "url": "https://github.com/lambdaclass/ethrex/commit/3eee599280855b30cfd023ecce4a706ebe69d3a5"
        },
        "date": 1750898966299,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.008773306792873052,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "azteca1998@users.noreply.github.com",
            "name": "MrAzteca",
            "username": "azteca1998"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "217ad13acdaa6d7219ec4a5ad16fce5164749ae8",
          "message": "perf(levm): refactor `Stack` to have a fixed size and grow downwards (#3266)\n\n**Motivation**\n\nThe stack is currently implemented using a `Vec<U256>` that grows\nupwards. Since the stack is limited to a reasonable amount by design\n(1024 entries, or 32KiB) it can be converted to a static array along\nwith an offset and made to grow downwards.\n\n**Description**\n\nThis PR:\n- Removes stack allocation (and its associated runtime checks) from the\nruntime costs.\n- Makes the stack grow downwards: better integration with stack\noperations.\n- Changes the API so that multiple items can be inserted and removed\nfrom the stack at the same time, which is especially useful for opcodes\nthat handle multiple arguments or return values.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number\n\n---------\n\nCo-authored-by: Jeremías Salomón <48994069+JereSalo@users.noreply.github.com>",
          "timestamp": "2025-06-26T12:02:50Z",
          "tree_id": "5ac3d6fb4b5de3b973e523181a199aa9ab11338c",
          "url": "https://github.com/lambdaclass/ethrex/commit/217ad13acdaa6d7219ec4a5ad16fce5164749ae8"
        },
        "date": 1750943683885,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.008643367526055951,
            "unit": "Mgas/s"
          }
        ]
      }
    ]
  }
}