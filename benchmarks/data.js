window.BENCHMARK_DATA = {
  "lastUpdate": 1751908129306,
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
          "id": "f630e1c0f4fce2264b2991b3d743172e1514b196",
          "message": "test(l1): run all blockchain tests and refactor logic (#3280)\n\n**Motivation**\n\n- We weren't running all tests that we needed to. We ran tests from\nfolders prague, cancun and shanghai but folders that have names of older\nforks have \"old\" tests but they perform checks for current forks too. So\nwe should run them too!\n\n**Description**\n\n- Deletes `cancun.rs`, `shanghai.rs` and `prague.rs`. Doesn't make sense\nto run tests based on that. For example, when running cancun.rs you\ncould find tests which post state was Prague or Shanghai, so that\ndistinction we were making was kinda useless. Now we just have `all.rs`\nand I simplified it so that it is more clean.\n- Adds all networks to Network enum\n- Refactor `test_runner` so that parsing is better (now it's recursive)\nand also now when a test fails it doesn't stop executing the rest of the\ntests, which was pretty annoying.\n\n---------\n\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-06-26T14:36:53Z",
          "tree_id": "70a14b6e94d5840a1ac56f6960b54ff93de31be3",
          "url": "https://github.com/lambdaclass/ethrex/commit/f630e1c0f4fce2264b2991b3d743172e1514b196"
        },
        "date": 1750952228526,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222012327070,
            "range": "± 434402955",
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
          "distinct": false,
          "id": "3741a2ad5647ad1945907dfe6f1ac02d65054bc4",
          "message": "fix(l1, levm): fix remaining blockchain test (#3293)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n- Fix last blockchain test, it was failing for both LEVM and REVM\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- The test destroyed an account that had non-empty storage and then\nre-created it with CREATE2. When getting the AccountUpdates we just said\nthat the account had no added storage (which is true) but we don't have\na way to directly communicate that the account was destroyed and then\ncreated again, so even though it exists its old storage should be\ncleared.\n- For this I implemented an ugly solution. For both LEVM and REVM in\nget_account_updates if I see that an account was Destroyed but now\nexists what I'll do is I'll push 2 Account Updates, one that removes the\naccount and another one with the new state of the account, so that the\nwhole account is removed (and therefore, its storage) and then we write\nto the database the new state of the account with it's new storage. I\nthink a cleaner solution would be to have an attribute `removed_storage`\n(or similar) in `AccountUpdate` that will tell the client to remove the\nstorage of the existing account without removing all the account and\nthen we don't have to do messy things like the one I implemented. The\ndownside that I see on this new approach is that we'll have an attribute\nthat we'll hardly ever use, because it's an edge case.\n- Then, for LEVM I had to implement a `destroyed_accounts` in\n`GeneralizedDatabase` so that in `get_state_transitions()` we can check\nwhich accounts were destroyed and now exist so that we do the procedure\nthat I described above. This and many other things would be way nicer if\nwe used in LEVM our own Account struct instead of reusing the one in\nEthrex. I'm seriously considering making that change because it seems\nworth doing so, there are other reasons to pull the trigger on that.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nQuestions:\n1. Should we add a `removed_storage` to `AccountUpdate` instead? Or this\nway of implementing it (removing account and then writing it) is good\nenough? Created #3321\n2. Should we use our own Account type in LEVM so that we don't rely on\nexternal HashSets and HashMaps for some things? For this I opened #3298\n\nCloses #3283\n\n---------\n\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-06-26T15:18:38Z",
          "tree_id": "73dcb8916d9e6a46cc1f6b47ab5c31c7b2ba2616",
          "url": "https://github.com/lambdaclass/ethrex/commit/3741a2ad5647ad1945907dfe6f1ac02d65054bc4"
        },
        "date": 1750954764949,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 221326895624,
            "range": "± 285129188",
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
          "id": "09cac25ff1b390c5a03ab1846a46ef82283e8e2e",
          "message": "fix(l1): flaky test `test_peer_scoring_system ` (#3301)\n\n**Motivation**\nThe test executes a function that selects peers randomly but\nprioritizing those with high score, and checks that the number of\nselections for each peer is proportional to its score. However given\nthat the selection is somehow random, this is not always the case.\n\n**Description**\nIntroduces the following changes in the test\n* Increments the number of selections, which should reduce the\nprobability of failure.\n* Initializes a different `KademliaTable` for working with multiple\npeers.\nNote that the table used for multiple-peer scoring checks was the same\nas the one used for single-peer scoring tests. The problem is that a\nhigh-scoring peer from the initial phase remains in the table but is\nincorrectly omitted from subsequent multi-peer selection calculations,\nthus impacting the final outcome.\n\nThe following bash script can be used to get a sense of the failure rate\nof the test. It loops running the test and printing the total and failed\nruns.\n\nWith these changes, the failure rate dropped from (approximately) 4% to\n0.025%.\n\n```bash\n#!/bin/bash\n\nCOMMAND=(cargo test --package=ethrex-p2p --lib -- --exact kademlia::tests::test_peer_scoring_system --nocapture)\n\ntotal=0\nfailed=0\n\nwhile true; do\n    \"${COMMAND[@]}\" >/dev/null 2>&1\n    exit_code=$?\n\n    if [ $exit_code -ne 0 ]; then\n        failed=$((failed + 1))\n        echo \"❌ failed\"\n    else\n        echo \"✅ ok\"\n    fi\n\n    total=$((total + 1))\n    echo \"Total = $total, Failed = $failed\"\n    echo \"---\"\ndone\n\n```\n\n\n\nCloses #3191",
          "timestamp": "2025-06-26T16:03:58Z",
          "tree_id": "0b5e1ee9b62d541aa34180f07b73361f632a71be",
          "url": "https://github.com/lambdaclass/ethrex/commit/09cac25ff1b390c5a03ab1846a46ef82283e8e2e"
        },
        "date": 1750957375842,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220126650798,
            "range": "± 260140480",
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
          "id": "319656f7891424735deeb7a18d9b0976680dd04b",
          "message": "ci(l2): build docker images for integration test (#3338)\n\n**Motivation**\n\nCi failing because ethrex_dev:latest not found \n\n**Description**\n\n- add step to build the image",
          "timestamp": "2025-06-26T16:52:58Z",
          "tree_id": "db70eefd6d0032e7f761cfd28e4186e80e9775d5",
          "url": "https://github.com/lambdaclass/ethrex/commit/319656f7891424735deeb7a18d9b0976680dd04b"
        },
        "date": 1750960393763,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 222816708012,
            "range": "± 419298346",
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
          "id": "e12f615f1a6e67c1fb719f6693d6627d8e2a2e70",
          "message": "docs(l1): fix readme links and improve L1 documentation (#3340)\n\n**Motivation**\n\nThe current readme links to the unrendered documentation instead of our\nhosted book. Also, the general landing page doesn't have any links or\npointers on where to go next, while the L1 landing page is empty.\n\n**Description**\n\nThis PR addresses the previous issues, adding some content to the L1\nlanding page and generally cleaning up the docs. It also merges the two\ndocumentation sections in the readme and updates links to point to\ndocs.ethrex.xyz",
          "timestamp": "2025-06-26T18:13:10Z",
          "tree_id": "9da9d65d7fc47ea4423505e1996ffbbca8421bcc",
          "url": "https://github.com/lambdaclass/ethrex/commit/e12f615f1a6e67c1fb719f6693d6627d8e2a2e70"
        },
        "date": 1750965138428,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 218838381174,
            "range": "± 846219042",
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
          "id": "a0eebfa256edaaf3ffc35676fc3b3ad021dccde8",
          "message": "ci(l2): build docker images for all l2 tests (#3342)\n\n**Motivation**\n\nAfter #3338 state reconstruct test and based tests started failing\n**Description**\n\n- build the docker image for those steps too",
          "timestamp": "2025-06-26T18:13:36Z",
          "tree_id": "3f597839d3d2dc64484734cb7189722eede96728",
          "url": "https://github.com/lambdaclass/ethrex/commit/a0eebfa256edaaf3ffc35676fc3b3ad021dccde8"
        },
        "date": 1750965201069,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 220949612216,
            "range": "± 278785962",
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
          "id": "8675fe8bc44977275db954cc4730a79c382cc15a",
          "message": "perf(levm): refactor levm jump opcodes (#3275)\n\n**Motivation**\n\nThe `JUMP` and `JUMPI` opcodes need to check the target address's\nvalidity. This is currently done with a `HashSet` of valid target\naddresses, which caused the hashing to become a significant part of the\nprofiling time when checking for address validity.\n\n**Description**\n\nThis PR rewrites the `JUMPDEST` checks so that instead of having a\nwhitelist, we do the following:\n- Check the program bytecode directly. The jump target's value should be\na `JUMPDEST`.\n- Check a blacklist of values 0x5B (`JUMPDEST`) that are NOT opcodes\n(they are part of push literals).\n\nThe blacklist is not a `HashMap`, but rather a sorted slice that can be\nchecked efficiently using the binary search algorithm, which should\nterminate on average after the first or second step.\n\nRational: After extracting stats of the first 10k hoodi blocks, I found\nout that...\n- There are almost 60 times more `JUMPDEST` than values 0x5B in push\nliterals.\n- On average, there are less than 2 values in the blacklist. If we were\nto use a whitelist as before, there would be about 70 entries on\naverage.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nRelated to #3305",
          "timestamp": "2025-06-26T20:36:36Z",
          "tree_id": "d839a849cc638da9fe2743a8f65a578f6579a8bd",
          "url": "https://github.com/lambdaclass/ethrex/commit/8675fe8bc44977275db954cc4730a79c382cc15a"
        },
        "date": 1750973738845,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210448650992,
            "range": "± 434898055",
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
          "id": "f6b0ba4a352725e762dc0420b84ef9198a20d640",
          "message": "chore(l2): change default chain ID (#3337)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nCurrent chain ID (1729) is causing some problems with wallets like\nMetamask as the chain ID is registered for another network.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nChanged default chain ID to 65536999 following our new naming method:\n- `65536XYY`\n- Being `X` the stage (0 for mainnet, 1 for testnet, 2 for staging,\netc.).\n- Being `YY` each specific rollup.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3312",
          "timestamp": "2025-06-26T21:35:10Z",
          "tree_id": "5c0e989bf8fcfec78d3f3aca85cd30d7ff18d6c0",
          "url": "https://github.com/lambdaclass/ethrex/commit/f6b0ba4a352725e762dc0420b84ef9198a20d640"
        },
        "date": 1750977127124,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210972090092,
            "range": "± 454975535",
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
          "id": "19cfb4e1874c35f32ff78602093f1305c17b6f4d",
          "message": "chore(core): add install script (#3273)\n\n**Description**\n\nThis PR adds an installation script with readme instructions on how to\nquickly set up a local L1 with `ethrex`, without having to clone the\nrepo.\n\nThe idea is to extend this script once the L2 can be more easily\ndeployed. Right now, it requires installing two more binaries and\ndownloading multiple contracts (either the source or compiled\nartifacts), which makes this more complex than it needs to be.\n\n---------\n\nCo-authored-by: Javier Chatruc <jrchatruc@gmail.com>",
          "timestamp": "2025-06-26T21:40:31Z",
          "tree_id": "cfb5520d0c3f5cac05f111e8f19749137c261c90",
          "url": "https://github.com/lambdaclass/ethrex/commit/19cfb4e1874c35f32ff78602093f1305c17b6f4d"
        },
        "date": 1750977436376,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208858247765,
            "range": "± 755486195",
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
          "id": "b584bcc7e51507e5c943305e8d63dfc9f6f7ec68",
          "message": "refactor(l1, l2): remove `l2` feature flag from `rpc` crate (first proposal) [RFC] (#3330)\n\n**Motivation**\n\nThe main goal is to decouple L2 logic from the crate `rpc`. \n\nThis PR proposes an initial solution to this.\n\n**Description**\n\nThis solution extends `ethrex-rpc` by wrapping it where needed, and\nadding new data structures. I think a better solution can be achieved\nfrom this one.\n\n- Adds `ethrex-l2-rpc` crate in `crates/l2/networking/rpc`.\n- Moves L2 logic from `ethrex-rpc` to `ethrex-l2-rpc`.\n- Refactors some functions in `ethrex-rpx` to be reusable in\n`ethrex-l2-rpc`.\n- Exposes some functions and types from `ethrex-rpc`.\n- Updates `cmd/ethrex` with this changes and moves L2 initializers from\n`cmd/ethrex/initializers.rs` to `cmd/ethrex/l2/initializers.rs`.\n\n**Pros and Cons**\n\n| Pros | Cons|\n| --- | --- |\n| L2 logic is decoupled from `ethrex-rpc` | L2 logic is decoupled from\n`ethrex-rpc` by wrapping `ethrex-rpc` functions and duplicating some\ntypes to extend them |\n| `ethrex-rpc` logic is reused by `ethrex-l2-rpc` | Some types and\nfunctions were exposed in `ethrex-rpc` public API |\n\nDespite the cons, this could be an acceptable first iteration to a\nproper solution as this implementation highlights somehow which parts of\nthe rpc crate need to be abstracted.\n\n**Next Steps**\n\n- Remove `l2` feature flag from `cmd/ethrex`.\n- Move `crates/networking/rpc/clients` module.\n- Cleanup `ethrex-rpc` public API.\n- The next iteration should include an more general abstraction of the\nRPC API implementation so it can be extended organically instead of by\nwrapping it.",
          "timestamp": "2025-06-27T15:44:41Z",
          "tree_id": "b6aa69359a73072b97acca21d9c2daa7abe8b880",
          "url": "https://github.com/lambdaclass/ethrex/commit/b584bcc7e51507e5c943305e8d63dfc9f6f7ec68"
        },
        "date": 1751042636898,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210953905193,
            "range": "± 534112181",
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
          "id": "2e4bc87ddb14b2e5215d40bada54dad384741747",
          "message": "perf(core): improve u256 handling, improving PUSH and other opcodes (#3332)\n\n**Motivation**\n\nThe function `u265::from_big_endian(slice)` which is widely used in the\ncodebase can be made faster through more compile time information.\n\nWith this change, i could add a constant generic op_push\n\nBenchmarks:\n\nSeems like our current benchmarks don't measure this part of the code,\nthere is no difference in the levm bench, however external opcode\nbenchmarks show a 2-2.5x improvement on PUSH and MSTORE based benches\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3358",
          "timestamp": "2025-06-27T15:46:44Z",
          "tree_id": "ceac0361630c7d3402664dc9569e183fdae03ccb",
          "url": "https://github.com/lambdaclass/ethrex/commit/2e4bc87ddb14b2e5215d40bada54dad384741747"
        },
        "date": 1751042806554,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210457693357,
            "range": "± 708614294",
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
          "id": "f57dd24e3a4b142f6de1f80bc2d9ee98692bb08f",
          "message": "chore(levm): improve levm vs revm bench (#3255)\n\n**Motivation**\n\nCurrently the benchmark comparisson shown for levm is not very useful\nsince there is a lot of information distributed along different tables\nand it's not easy to see the change.\n\n**Description**\n\nThis pr introduces the following changes:\n\n- Instead of having the tables compare levm and revm, now they compare\nbetween pr's obtained mean time and main' obtained mean time for both\nvms. To do this we modify the `run_benchmark_ci` function that the\ntarget runned by the ci calls to have the benchmark comparison run for\nall cases.\n\n- If nothing changed between pr and main, no message is printed, using\nas a margin of error a porcentual difference higher than 10%.\n\n- If something changed, only output the tests where there was a change,\nnothing is printed if the test that stayed the same.\n\nThe tables always show the obtained metrics in the same order despite\nthe fact one of them is not shown:\n\n| Command | \n|--------|\n|`main_revm_`|\n|`main_levm_`|\n|`pr_revm_`|\n|`pr_levm_`|\n\nFor example, in the case you do an optimization in levm that improves\nfactorial but does nothing for revm, the table would look like this:\n| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |\n|--------|--------|--------|--------|--------|\n| `main_revm_Factorial` | 138.1 ± 0.5 | 137.4 | 139.2 | 1.00 |\n| `main_levm_Factorial` | 326.4 ± 6.3 | 321.4 | 340.2 | 2.36 ± 0.05 |\n| `pr_levm_Factorial` | 223.8 ± 6.3 | 216.4 | 234.1 | 2.04 ± 0.05 |\n\nCloses #3254\n\n---------\n\nCo-authored-by: cdiielsi <49721261+cdiielsi@users.noreply.github.com>\nCo-authored-by: Camila Di Ielsi <camila.diielsi@lambdaclass.com>",
          "timestamp": "2025-06-27T16:03:04Z",
          "tree_id": "e340415a97b05069af05234e55becf17585d535c",
          "url": "https://github.com/lambdaclass/ethrex/commit/f57dd24e3a4b142f6de1f80bc2d9ee98692bb08f"
        },
        "date": 1751043607092,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 206190289334,
            "range": "± 770356972",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "30327624+mechanix97@users.noreply.github.com",
            "name": "Mechardo",
            "username": "mechanix97"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "6d64350d80b39e755a771917f8fbd79e16db627b",
          "message": "docs(l2): based docs outdated contract addresses (#3348)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nThe contract addresses in the docs were outdated. \n\n**Description**\n\nTo avoid having to update the addresses of the contracts every time a\ncontract changes, placeholder were introduced.\n\nAlso removed the pico verifier from the contract deployment as it is no\nlonger needed.\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\n---------\n\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-06-27T19:49:45Z",
          "tree_id": "b0720e7467d3a80f93b7e9fea335bca7e7bf7c31",
          "url": "https://github.com/lambdaclass/ethrex/commit/6d64350d80b39e755a771917f8fbd79e16db627b"
        },
        "date": 1751057335444,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211082612032,
            "range": "± 592729609",
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
          "id": "510856b92b5aa4b9da070824ea10e28a7e5ec44c",
          "message": "fix(l2): serde skip attributes incompatible with bincode (#3370)\n\n**Motivation**\n\nsame old problem which we brute fix by serializing into JSON first, was\nreintroduced with the addition of `ExecutionWitnessResult` (which has\ntwo fields that use `#[serde(skip)]`)",
          "timestamp": "2025-06-27T20:38:17Z",
          "tree_id": "80b73f14440435541056a670f63a1e8f6e4f8173",
          "url": "https://github.com/lambdaclass/ethrex/commit/510856b92b5aa4b9da070824ea10e28a7e5ec44c"
        },
        "date": 1751060430563,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 213513081609,
            "range": "± 2669531763",
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
          "id": "c9c750d8b1c2b0ab8cba89a3657f123dabe6f3fc",
          "message": "perf(levm): reduce handle_debug runtime cost (#3356)\n\n**Motivation**\n\nWith this handle_debug disappears from samplies on push/memory heavy\nbenches.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3355",
          "timestamp": "2025-06-27T20:44:26Z",
          "tree_id": "6ddd2bd3f702e9b92bede8dab5e830166d6e8827",
          "url": "https://github.com/lambdaclass/ethrex/commit/c9c750d8b1c2b0ab8cba89a3657f123dabe6f3fc"
        },
        "date": 1751060650667,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210879737422,
            "range": "± 404546331",
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
          "id": "2dd0e7b4e73f1578fbd2c43e2845bcfb2fd46e8c",
          "message": "chore(levm): add push/mstore based bench (#3354)\n\n**Motivation**\n\nAdds a benchmark that mainly tests PUSH and MSTORE\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3364\n\n---------\n\nCo-authored-by: Javier Chatruc <jrchatruc@gmail.com>\nCo-authored-by: ilitteri <ilitteri@fi.uba.ar>",
          "timestamp": "2025-06-27T20:49:14Z",
          "tree_id": "92222d56d1d8b08b0d73e056fee5f4f6efcab91f",
          "url": "https://github.com/lambdaclass/ethrex/commit/2dd0e7b4e73f1578fbd2c43e2845bcfb2fd46e8c"
        },
        "date": 1751060972531,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209882948866,
            "range": "± 848066479",
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
          "id": "7b626fddc2a1556db2c61af9b76ce0e26dd352eb",
          "message": "refactor(l2): remove blobs logic from payload_builder (#3326)\n\n**Motivation**\n\nRemoves unnecessary logic for handling blob transactions in\n`L2::payload_builder`, since these transactions are discarded just a few\nlines below.\n\n\nCloses None",
          "timestamp": "2025-06-27T21:03:02Z",
          "tree_id": "5d209dd66d6568c111588f3f7624f8553a67c85f",
          "url": "https://github.com/lambdaclass/ethrex/commit/7b626fddc2a1556db2c61af9b76ce0e26dd352eb"
        },
        "date": 1751061615490,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209374061533,
            "range": "± 899896290",
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
          "id": "4d6cbf6db95cff5b0fe11af74dd4c629a3e1be71",
          "message": "feat(l1): embed genesis of public networks in binary (#3372)\n\n**Motivation**\n\nCurrently, users need to download the genesis to be able to run a\nnetwork without being in the repo's root.\n\n**Description**\n\nThis PR embeds the genesis file of known public networks inside the\ncompiled binary. It has the downside of increasing binary size from 23.6\nMb to 24.7 Mb.\n\nFurther code simplifications are left for other PRs.\n\nIn the future, a possible improvement could be to parse each genesis\nfile before embedding it in the binary. Now we just embed the plain\nJSON.\n\nRelated #3292",
          "timestamp": "2025-06-27T21:16:21Z",
          "tree_id": "22bbe2194dc8cf877b504540879f04fd9ffd8b9e",
          "url": "https://github.com/lambdaclass/ethrex/commit/4d6cbf6db95cff5b0fe11af74dd4c629a3e1be71"
        },
        "date": 1751062516603,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210057228422,
            "range": "± 561318655",
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
          "id": "25432a8606f153031410173b56c7a283fd08cc9e",
          "message": "chore(l2): remove save state module (#3119)\n\n**Motivation**\n\nThe save state module was unnecessarily complex for what it achieved\n(storing incoming batch proofs). This PR replaces it with the rollup\nstore.\n\n**Description**\n\n- adds tables for storing batch proofs indexed by batch number and proof\ntype to the rollup store\n- removes the save state module\n- all places that used the save state module now use the rollup storage\n- had to move the prover interface into `ethex-l2-common` because\n`ethrex-storage-rollup` can't depend on the prover because of cyclic\ndependencies\n- because of the previous point had to move the `Value` type into\n`ethrex-l2-common` because `ethrex-l2-common` can't depend on\n`ethrex-sdk`\n\n---------\n\nCo-authored-by: Tomás Paradelo <112426153+tomip01@users.noreply.github.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>",
          "timestamp": "2025-06-27T21:32:19Z",
          "tree_id": "c551144c82bed4a41367a84ded8abb3732a39e43",
          "url": "https://github.com/lambdaclass/ethrex/commit/25432a8606f153031410173b56c7a283fd08cc9e"
        },
        "date": 1751063470321,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209510267943,
            "range": "± 509174897",
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
          "id": "3f65132f7fb6d206f189754ace88340184811e7d",
          "message": "fix(l2): update rpc job cache file (#3377)\n\n**Motivation**\n\nin #3370 i didn't update it after changing how the program input is\nserialized",
          "timestamp": "2025-06-27T22:22:17Z",
          "tree_id": "cdf056c0a40fbe670409308b79b3f0b9f5f937ba",
          "url": "https://github.com/lambdaclass/ethrex/commit/3f65132f7fb6d206f189754ace88340184811e7d"
        },
        "date": 1751066407966,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209395567112,
            "range": "± 463770214",
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
          "id": "a51ad76d5dc1ca9ad6bb371037c9894a01af356a",
          "message": "chore(core): update Rust version to 1.88 (#3382)",
          "timestamp": "2025-06-30T13:04:49Z",
          "tree_id": "7b9134c67f0cd8bbceeb6285c954f84e85063274",
          "url": "https://github.com/lambdaclass/ethrex/commit/a51ad76d5dc1ca9ad6bb371037c9894a01af356a"
        },
        "date": 1751292189402,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207103992446,
            "range": "± 297046142",
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
          "id": "98512676ba767d6c635633da9b70932af1a2c483",
          "message": "docs(l1): fix broken links in sync doc (#3369)\n\n**Motivation**\nSome links in the sync documentation do not point to the correct files\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Fix broken links to diagrams in sync documentation\n* Remove link and mention of no longer existing diagram\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-06-30T13:15:09Z",
          "tree_id": "c1ab3a358155856f42742f8819e4ba0c51fb07cf",
          "url": "https://github.com/lambdaclass/ethrex/commit/98512676ba767d6c635633da9b70932af1a2c483"
        },
        "date": 1751292887512,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211597378504,
            "range": "± 758816429",
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
          "id": "4b6318fb291bc731b4123fd88e9398c5aacb4b30",
          "message": "feat(l2): value sending in privileged transactions (#3320)\n\n**Motivation**\n\nWe want to be able to send (\"deposit\") value-carrying privileged\ntransactions, for example to do forced withdrawals.\n\n**Description**\n\nThis PR allows users to specify the transaction value while sending\nprivileged transactions.\n\nFor regular deposits, the bridge calls it's L2 version and relies on the\nL2 hook allowing the bridge address to send without having funds. It\ncalls it's own L2 version, which handles failed transfers by making a\nwithdrawal.\n\nFor transactions, since they can't fail they are instead made to revert\nwhen there aren't enough funds to cover the operation.\n\nCloses #3290, closes #3291\n\n---------\n\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>",
          "timestamp": "2025-06-30T13:28:49Z",
          "tree_id": "0e7d86f866aa0e1081ab669a2f064c76a1c0eb95",
          "url": "https://github.com/lambdaclass/ethrex/commit/4b6318fb291bc731b4123fd88e9398c5aacb4b30"
        },
        "date": 1751293617090,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210671124628,
            "range": "± 770806776",
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
          "id": "11384d62135ed354397c0a02b57adbe8d8392a8b",
          "message": "ci(l2): use build-push-action to build L1 dev docker image (#3357)\n\n**Motivation**\n\nuse official\n[build-push-action](https://github.com/docker/build-push-action)",
          "timestamp": "2025-06-30T14:03:04Z",
          "tree_id": "34ce27d88d8e5c5e41e2acbbd24f840ad8e44840",
          "url": "https://github.com/lambdaclass/ethrex/commit/11384d62135ed354397c0a02b57adbe8d8392a8b"
        },
        "date": 1751295720838,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211390794540,
            "range": "± 832058859",
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
          "id": "57b2a2b26933eebf3e878a05d1c46aa7a898fd0c",
          "message": "fix(l2): remove requests processing in build_payload (#3300)\n\n**Motivation**\n\nWhen producing a block in L2 mode, we unnecessarily call\n`blockchain::extract_requests`, even though no requests are being\ngenerated.\n\n**Description**\n\n- Refactors `PayloadBuildContext` to store `requests:\nOption<Vec<EncodedRequests>>` and calculates the `request_hash` directly\nin `finalize_payload`.\n- Removes the `blockchain::extract_requests` call from\n`payload_builder`.\n\n\nCloses None",
          "timestamp": "2025-06-30T14:18:21Z",
          "tree_id": "bac5bdc1fbd5ae7e0f729563d2de21a7df9e4ff5",
          "url": "https://github.com/lambdaclass/ethrex/commit/57b2a2b26933eebf3e878a05d1c46aa7a898fd0c"
        },
        "date": 1751296654173,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210921941993,
            "range": "± 595466263",
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
          "id": "2ebb67cf51f137c75f26bdfa66540f5ea8835d6e",
          "message": "chore(l1,l2): reorder fixtures (#3155)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nThe `tests_data` folder was unorganized \n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nCreated a `fixtures` folder inside of test data with a folder for each\ntype of file.\nThe current structure looks like this:\n```\nfixtures/\n├── genesis/                    # All genesis files\n│   └── *.json\n│   \n├── network/                    # Network configuration\n│   ├── params.yaml\n│   └── hive_clients\n|       └── *.yml\n|\n├── contracts/                  # Smart contracts for testing\n│   ├── ERC20/\n│   │   ├── ERC20.sol\n│   │   ├── ERC20.bin\n│   │   |   └── TestToken.bin\n│   │   └── deps.sol\n│   ├── load-test/\n│   |   └── IOHeavyContract.sol\n|   └──levm_print\n|        └── Print.sol\n|\n├── blockchain/                 # Blockchain data files\n│   └── *.rlp\n|\n├── blobs/                      # BLOB files\n│   └── *.blob\n|\n├── keys/                       # Private keys for testing\n│   ├── private_keys.txt\n│   └── private_keys_l1.txt\n|\n├── cache/                      # Cached data\n│   └── rpc_prover/\n│       └── cache_3990967.json\n|\n└── rsp/                       \n    └── input/\n        └── 1/\n             └── 21272632.bin\n ```\nAll references were updated to avoid breaking the code\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3006\n\n---------\n\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-06-30T15:33:31Z",
          "tree_id": "d4cc7189a817b84df69f40c674589bbf9ad8a4d8",
          "url": "https://github.com/lambdaclass/ethrex/commit/2ebb67cf51f137c75f26bdfa66540f5ea8835d6e"
        },
        "date": 1751301102039,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207736713004,
            "range": "± 411989142",
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
          "id": "bc56f4764a782fd952a56308dd8e28c41a190a83",
          "message": "feat(l2): verify batches in chunks with aligned (#3242)\n\n**Motivation**\n\nAligned verifies proofs in batches, so it's possible to have an array of\nproofs ready to be verified at once.\n\n**Description**\n\n- Modifies `l1_proof_verifier` to check all already aggregated proofs\nand build a single verify transaction for them.\n- Updates `verifyBatchAligned()` in the `OnChainProposer` contract to\naccept an array of proofs.\n\n> [!WARNING]\n> #3276 was accidentally merged into this PR, so the diff includes\nchanges from both.\n\nCloses #3168\n\n---------\n\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-06-30T15:34:32Z",
          "tree_id": "8a028358cc3b77a2e6c4a4e70df7faef6669dded",
          "url": "https://github.com/lambdaclass/ethrex/commit/bc56f4764a782fd952a56308dd8e28c41a190a83"
        },
        "date": 1751301147543,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210495960759,
            "range": "± 808046138",
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
          "id": "b8dffc6936af6669927bb1657f19fb6756dabf13",
          "message": "fix(l2): parse default value correctly (#3317)\n\n**Description**\n\nThis PR fixes a panic we had when the user didn't specify any of the\n`proof-coordinator.*` flags. It seems clap called the `Default::default`\nimplementation, which panicked because of the parsing not having support\nfor leading `0x`.\n\nCloses #3309",
          "timestamp": "2025-06-30T17:01:49Z",
          "tree_id": "a1dd4d6768884e79ec22aa6bf5dc3a80898432a5",
          "url": "https://github.com/lambdaclass/ethrex/commit/b8dffc6936af6669927bb1657f19fb6756dabf13"
        },
        "date": 1751306441073,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209701210784,
            "range": "± 390615335",
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
          "id": "6e15e05062457168e668c3bd58f9c7cc4017f3f4",
          "message": "docs(l2): document ETH and ERC20 deposits/withdrawals (#3223)\n\n**Description**\n\nThis PR updates the withdrawals documentation with native ERC20\nwithdrawals and adds documentation for L2 deposits.\n\n---------\n\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-06-30T17:06:43Z",
          "tree_id": "55ba012f1c1de7304da9e0753767a034e0176a79",
          "url": "https://github.com/lambdaclass/ethrex/commit/6e15e05062457168e668c3bd58f9c7cc4017f3f4"
        },
        "date": 1751306798098,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209338782689,
            "range": "± 392301260",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "damian.ramirez@lambdaclass.com",
            "name": "Damian Ramirez",
            "username": "damiramirez"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "f32b20294993af9eb01f152ff1861ce17bad180e",
          "message": "docs(core): add LaTex support (#3398)\n\n**Motivation**\n\nhttps://docs.ethrex.xyz/, isn't rendering LaTex expression \n\n**Description**\n\nThis PR adds the [mdbook-katex](https://github.com/lzanini/mdbook-katex)\npreprocessor to `book.toml` to enable LaTeX rendering.\n\nRun the following command to check how the documentation looks\n\n```bash\nmake docs-deps && make docs-serve\n```\n\n**Before**\n\n![image](https://github.com/user-attachments/assets/393cf3f8-bd4a-455a-bba0-4e46cc4e42f0)\n\n**After**\n\n![image](https://github.com/user-attachments/assets/8e6954ad-9050-4161-ada2-2f3d0e13a804)",
          "timestamp": "2025-06-30T18:04:46Z",
          "tree_id": "f67ea03499eb83538e48885b04d35ce1c113c2a4",
          "url": "https://github.com/lambdaclass/ethrex/commit/f32b20294993af9eb01f152ff1861ce17bad180e"
        },
        "date": 1751310159777,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209322451102,
            "range": "± 729403895",
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
          "id": "8795e325f112d967cee60b965d64a6945dec068f",
          "message": "fix(l2): based CI (#3404)\n\n**Motivation**\n\nIn #3242, `verifyBatchesAligned()` was updated in the based\n`OnChainProposer` to be consistent with the non-based one, but the\n`onlySequencer` identifier was added by mistake.\n\n**Description**\n\nRemoves the `onlySequencer` identifier.\n\nCloses None",
          "timestamp": "2025-06-30T18:11:54Z",
          "tree_id": "7c28ebcd53dded82020376b2a62d37f3caba2597",
          "url": "https://github.com/lambdaclass/ethrex/commit/8795e325f112d967cee60b965d64a6945dec068f"
        },
        "date": 1751310787973,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209706714945,
            "range": "± 441054948",
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
          "id": "3d375136a47d1e74bfe5db3ad5e21fd3481c3e55",
          "message": "feat(l2): implement ERC20 bridge (#3241)\n\n**Motivation**\n\nWe want to be able to bridge ERC20 tokens.\n\n**Description**\n\nThe inner workings are explained on #3223\n\n---------\n\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Manuel Iñaki Bilbao <manuel.bilbao@lambdaclass.com>",
          "timestamp": "2025-06-30T19:19:40Z",
          "tree_id": "7b4fb4915fb461d10b28a55ed7755a3c5f692e1d",
          "url": "https://github.com/lambdaclass/ethrex/commit/3d375136a47d1e74bfe5db3ad5e21fd3481c3e55"
        },
        "date": 1751314853812,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211730397039,
            "range": "± 580301151",
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
          "id": "0cae5580655068ccad5aa3e5d1fdf357a459c384",
          "message": "feat(l2): add instance info to Grafana alerts (#3333)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nWe need to easily differentiate between environments when alerts come up\n(staging-1, staging-2, etc.).\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nAdd an \"$INSTANCE\" variable in the Slack contact point so it's\nover-ridden with an env var.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-06-30T19:52:03Z",
          "tree_id": "b2e3209043911fd5929de4a4ebf5af80232542d8",
          "url": "https://github.com/lambdaclass/ethrex/commit/0cae5580655068ccad5aa3e5d1fdf357a459c384"
        },
        "date": 1751316615591,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208263508263,
            "range": "± 1490572123",
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
          "id": "68f360fc345e8064a67f266c68c00e7439db961b",
          "message": "feat(l2): exchange commit hash in node-prover communication (#3339)\n\n**Motivation**\n\nWe want to prevent a divergence between the code that is running in the\nL2 node and the prover.\n\n**Description**\n\n- Updates the client version to use `GIT_BRANCH` and `GIT_SHA` instead\nof `RUSTC_COMMIT_HASH`.\n- Adds a `build.rs` script for both the node and prover, using\n`vergen_git2` to export the git env vars.\n- Adds a `code_version` field to the `BatchRequest` message.\n- Introduces a new `ProofData` message: `InvalidCodeVersion`.\n\n## How to test\n\nYou can create an empty commit with:\n\n```bash\ngit commit --allow-empty -m \"empty commit\"\n```\n\nThen run the node and the prover using different commits.\n\n> [!WARNING]\n> Remember to run `make build-prover` whenever you change the commit\n\nCloses #3311",
          "timestamp": "2025-06-30T20:59:41Z",
          "tree_id": "d5078c5b0bbbe506fab9ce26a087572ff05c0971",
          "url": "https://github.com/lambdaclass/ethrex/commit/68f360fc345e8064a67f266c68c00e7439db961b"
        },
        "date": 1751320681708,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207836514670,
            "range": "± 617500350",
            "unit": "ns/iter"
          }
        ]
      },
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
          "id": "48cfb26ddd8e60f63b9c5bcc5dee86243c2ae9d4",
          "message": "ci(core): fix block benchmark ci",
          "timestamp": "2025-07-04T13:47:37Z",
          "url": "https://github.com/lambdaclass/ethrex/pull/3484/commits/48cfb26ddd8e60f63b9c5bcc5dee86243c2ae9d4"
        },
        "date": 1751639842256,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208719291147,
            "range": "± 361376477",
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
          "id": "ccfc231b5756c045b2d55febbb796ea63877d56a",
          "message": "ci(core): fix block benchmark ci (#3484)\n\n**Motivation**\n\nIts broken :(\n\n**Description**\n\n- The file `genesis-perf-ci.json` was moved and the reference was not\nupdated. This PR updates the reference\n- Here is a failing run in main\nhttps://github.com/lambdaclass/ethrex/actions/runs/16075239686/job/45368784296\n- Here is a test run in this pr that didn't panic when reading the file\nhttps://github.com/lambdaclass/ethrex/actions/runs/16075285386/job/45368928205.\n   - The run was cut short because it takes 40+ minutes to run",
          "timestamp": "2025-07-04T14:33:11Z",
          "tree_id": "98ae5f4311797a75c27363ff9b6ddb9d43b49a04",
          "url": "https://github.com/lambdaclass/ethrex/commit/ccfc231b5756c045b2d55febbb796ea63877d56a"
        },
        "date": 1751643223543,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208748164326,
            "range": "± 306672263",
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
          "id": "5abf85df328b22d2674a321d30a89dae033b01f6",
          "message": "fix(l2): gas used in replay block range (#3483)\n\n**Motivation**\n\nThe `gas_used` value in block range execution/proving was incorrect. We\nwere returning the gas used by only the first block.\n\n**Description**\n\n- Returns the total gas used across all blocks.\n- Also moves the `or_latest` function, as it doesn’t belong in `fetcher`\nanymore.\n\nCloses: None",
          "timestamp": "2025-07-04T14:57:07Z",
          "tree_id": "4fa8efbdb3ce22caed01d05826918a163151f9b5",
          "url": "https://github.com/lambdaclass/ethrex/commit/5abf85df328b22d2674a321d30a89dae033b01f6"
        },
        "date": 1751644676522,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208587874888,
            "range": "± 583828328",
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
          "id": "f6a839d10689bbf0c016dc7c486d73d485850b21",
          "message": "fix(l1): `RpcBlock` `uncles` field should have the hashes and not the block headers (#3245)\n\n**Motivation**\nFix inconsistencies between our RPC outputs and the spec.\nAccording to the spec endpoints such as `eth_getBlockByNumber` return a\nblock where the `uncles` field contains the hashes of the uncle blocks,\nwhile we return the full headers.\nThis has not been a problem for us as we have been mainly using\npost-merge blocks without uncles, but it will become a problem if we\nneed to export/import older blocks via rpc\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Change `uncles` field of `RpcBlock` from `Vec<BlockHeader>` to\n`Vec<H256`\n* (Bonus) Allow deserializing blocks without `base_fee_per_gas`\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses None",
          "timestamp": "2025-07-04T15:10:19Z",
          "tree_id": "4ece4b3ae184c29396de8151f4b8a22ab0a95a0b",
          "url": "https://github.com/lambdaclass/ethrex/commit/f6a839d10689bbf0c016dc7c486d73d485850b21"
        },
        "date": 1751645487409,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209056357578,
            "range": "± 417087772",
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
          "id": "0f0e27084645dc8fa4675a61ecfc2fe602cdcaf0",
          "message": "feat(l1): embed dev-mode genesis in binary (#3413)\n\n**Motivation**\n\nGiving the user a default dev-mode genesis block makes starting out with\n`ethrex` easy.\n\n**Description**\n\nThis PR:\n\n- Adds a new network option (`Network::LocalDevnet`).\n- Default to the new network when `--dev` is specified but no custom\nnetwork was specified.\n- Remove the genesis downloading step from install script.\n- Update the readme to reflect that no genesis file needs to be\nspecified; ethrex comes with batteries included 🦖\n\nCloses #3378",
          "timestamp": "2025-07-04T16:00:11Z",
          "tree_id": "2185d63d0fde7445a38117dbcc9e0c2398c6e0d1",
          "url": "https://github.com/lambdaclass/ethrex/commit/0f0e27084645dc8fa4675a61ecfc2fe602cdcaf0"
        },
        "date": 1751648437404,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209286317190,
            "range": "± 1508328942",
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
          "id": "0c4beceb320f47a1223674cf95ba19b9e204006f",
          "message": "chore(l2): remove leftover l2 feature flag references (#3488)\n\n**Motivation**\n\nThe `make rm-db-l2` command was broken due to a leftover reference to\nthe removed `l2` feature flag. This PR also removes a few other outdated\nreferences.\n\nCloses: None",
          "timestamp": "2025-07-04T18:19:40Z",
          "tree_id": "73a90b78b8de44e830d79f9a0b4d6f7c9f2202e0",
          "url": "https://github.com/lambdaclass/ethrex/commit/0c4beceb320f47a1223674cf95ba19b9e204006f"
        },
        "date": 1751656799028,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208752795777,
            "range": "± 512499926",
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
          "id": "9d4d6b4766c238e7f11ac98dc450eb49691e4069",
          "message": "ci(core): create release when pushing a semver tag  (#3431)\n\n**Motivation**\n\nCreate an ethrex release when a semver tag is pushed to the repo\n\n**Description**\n\n- Modifies already existing L2 releases workflow to build \n- For `ethrex client` (right now only for L2 but can be used for both L1\nand L2 after #3381 is merged )\n     - Build for `linux-x86_64` , `linux-arm64`, `macos-arm64`\n  - For `ethrex prover client`\n    - For `exec` mode\n       - Build for `linux-x86_64` `linux-arm64` and `macos-arm64`\n     - For `sp1 gpu` mode \n         - Build for `linux-x86_64` `linux-arm64`\n     - For `risc0 gpu` mode\n        - Build for `linux-x86_64`\n - For `ethrex replay`\n    - For `exec` mode\n       - Build for `linux-x86_64` `linux-arm64` and `macos-arm64`\n     - For `sp1 gpu` mode \n         - Build for `linux-x86_64` `linux-arm64`\n     - For `risc0 gpu` mode\n        - Build for `linux-x86_64`\n  - Creates a release with\n- Changelog from all the changes between the previous tag and the newly\ncreated one.\n- All the built binaries and the rollup L1 and L2 contracts in a tar\narchive\n- Example from testing repo\nhttps://github.com/LeanSerra/ethrex/releases/tag/v0.0.7-rc.1 before\nadding the contracts to the release output\n- Another example with the contracts\nhttps://github.com/LeanSerra/ethrex/releases/tag/v0.0.7-rc.2\n- Example of all outputs\nhttps://github.com/lambdaclass/ethrex/actions/runs/16077779945\n- Non related change: pin the version for the docker image in sp1 \n- With #3381 merged we now build ethrex with --all-features so the\ndefault database was changed to libmdbx when initializing the store.\n**Other considerations**\n\n> Q: No sp1 cpu?\n> A: Right now it's too slow to be viable in production environments\n\n> Q: No sp1 macos?\n> A: sp1 does not support the metal api for gpu acceleration. Also\nbecause zkvms are built using docker we'll run into a problem where the\nmacos github runner does not have docker installed.\n> After a quick investigation into this issue we have to use\n[colima](https://github.com/actions/runner/issues/1456#issuecomment-1676495453)\nfor docker.\n> This leads into another issue where sp1 uses a docker image built only\nfor amd64 this requires nested virtualization support that is enabled in\napple M3 chips or later but [the runner is currently using M1\nchips](https://docs.github.com/en/actions/concepts/runners/about-larger-runners#limitations-for-macos-larger-runners).\n\n> Q: Why does building the sp1 prover for\n[arm-64](https://github.com/LeanSerra/ethrex/actions/runs/16010958409/job/45168455867)\ntake twice as long as building for\n[x86-64](https://github.com/LeanSerra/ethrex/actions/runs/16010958409/job/45168455854)\n> A: We have to use QEMU inside the `arm-64` runner because the sp1\ndocker image is only built for `amd64`\n\n> Q: wen risc0\n> A: ~After #3172 is merged~\n[Now](https://github.com/lambdaclass/ethrex/actions/runs/16036016678/job/45247866997)",
          "timestamp": "2025-07-04T19:07:17Z",
          "tree_id": "7866af3a86be0f74b5fd91ce7bfb3795eb4edc2d",
          "url": "https://github.com/lambdaclass/ethrex/commit/9d4d6b4766c238e7f11ac98dc450eb49691e4069"
        },
        "date": 1751659682724,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208049843480,
            "range": "± 211691326",
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
          "id": "17f3b4be3480414759727a6a485e409dcf645a1a",
          "message": "chore(levm): improve beacon_root_contract_call readibility (#3490)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-04T19:15:47Z",
          "tree_id": "f2e3c9c9f5b13e9b9b00aba525f45c58d252f288",
          "url": "https://github.com/lambdaclass/ethrex/commit/17f3b4be3480414759727a6a485e409dcf645a1a"
        },
        "date": 1751660227885,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211999012884,
            "range": "± 1788137684",
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
          "distinct": false,
          "id": "516cc4dcd68616f2e61c988e23e91aa59ffc2f93",
          "message": "refactor(levm): give more details in `TxValidationError` (#3408)\n\n**Motivation**\nFrom #3062 \n> When validating a transaction it would be useful for the LEVM user to\nknow more details about why a transaction didn't pass the initial\nvalidations.\nThis involves changing errors inside of `TxValidationError` enum.\n\n**Description**\nThis PR adds more detail to some of the `TxValidationError` error types.\nIt also introduces the following changes:\n* Updates the error messages in the `ef_tests` deserializer.\n* Adds a regex to match the new error format in the `ef_tests` runner.\nThis is required since the deserialized messages do not include the\nnewly added details, so we need to ignore those details and ensure the\nmatch still succeeds.\n\n> [!CAUTION]\n> Do not merge until [the PR to\nexecution-spec-tests](https://github.com/ethereum/execution-spec-tests/pull/1832)\nhas been merged.\n\nCloses #3062\n\n---------\n\nCo-authored-by: JereSalo <jeresalo17@gmail.com>\nCo-authored-by: Jeremías Salomón <48994069+JereSalo@users.noreply.github.com>",
          "timestamp": "2025-07-05T18:32:29Z",
          "tree_id": "1e946722ffcf6cf6e1d1641533fffada47d8a2cb",
          "url": "https://github.com/lambdaclass/ethrex/commit/516cc4dcd68616f2e61c988e23e91aa59ffc2f93"
        },
        "date": 1751743947164,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209748175258,
            "range": "± 659747660",
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
          "id": "3e35ab5aede958a262917a7dff1b8396e3806748",
          "message": "ci(l2): comment out slack notification on risc0 replay job because of timeouts (#3501)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-05T18:38:43Z",
          "tree_id": "203c1a0437ad879a6a411e6e2e75027f22d5ce4a",
          "url": "https://github.com/lambdaclass/ethrex/commit/3e35ab5aede958a262917a7dff1b8396e3806748"
        },
        "date": 1751744442928,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210146635681,
            "range": "± 482304169",
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
          "id": "7269b9392bd5d2b091bc3c389e1a69bcb6f9363d",
          "message": "fix(l2): rename field  `from` of `PrivilegeL2Transaction` for serialization (#3487)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\nThe field `from` of the struct `PrivilegeL2Transaction` overlaps with\nthe field `from` of\n[`RpcTransaction`](https://github.com/lambdaclass/ethrex/blob/main/crates/networking/rpc/types/transaction.rs#L15)\nduring serialization (because the field `tx` is flattened by `serde`),\nmaking `RpcTransaction` deserializations invalid for\n`PrivilegeL2Transaction`s.\n\n**Description**\n\nRenames the field `from` of `PrivilegeL2Transaction` (only for\nserialization and deserializations) to fix the deserializations of\n`RpcTransaction`s of `PrivilegeL2Transaction`.",
          "timestamp": "2025-07-07T15:36:46Z",
          "tree_id": "e34179340ff84d4864b691398227ec1f130791e7",
          "url": "https://github.com/lambdaclass/ethrex/commit/7269b9392bd5d2b091bc3c389e1a69bcb6f9363d"
        },
        "date": 1751906465930,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209081240138,
            "range": "± 841121534",
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
          "id": "b08fa01cfcefc97428d5d9872aca7a7637dab6c3",
          "message": "fix(l2): revert to rust 1.87 (#3506)\n\n**Motivation**\n\nThe SP1 and RISC0 workflows are broken because they don't yet support\nRust 1.88.\n\n**Description**\n\nReverts the Rust version to 1.87.\n\n\nCloses None",
          "timestamp": "2025-07-07T16:02:04Z",
          "tree_id": "8ec748de442339ffc840eb543717650bdc405e65",
          "url": "https://github.com/lambdaclass/ethrex/commit/b08fa01cfcefc97428d5d9872aca7a7637dab6c3"
        },
        "date": 1751907856527,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210537404199,
            "range": "± 519479005",
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
          "id": "f630e1c0f4fce2264b2991b3d743172e1514b196",
          "message": "test(l1): run all blockchain tests and refactor logic (#3280)\n\n**Motivation**\n\n- We weren't running all tests that we needed to. We ran tests from\nfolders prague, cancun and shanghai but folders that have names of older\nforks have \"old\" tests but they perform checks for current forks too. So\nwe should run them too!\n\n**Description**\n\n- Deletes `cancun.rs`, `shanghai.rs` and `prague.rs`. Doesn't make sense\nto run tests based on that. For example, when running cancun.rs you\ncould find tests which post state was Prague or Shanghai, so that\ndistinction we were making was kinda useless. Now we just have `all.rs`\nand I simplified it so that it is more clean.\n- Adds all networks to Network enum\n- Refactor `test_runner` so that parsing is better (now it's recursive)\nand also now when a test fails it doesn't stop executing the rest of the\ntests, which was pretty annoying.\n\n---------\n\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-06-26T14:36:53Z",
          "tree_id": "70a14b6e94d5840a1ac56f6960b54ff93de31be3",
          "url": "https://github.com/lambdaclass/ethrex/commit/f630e1c0f4fce2264b2991b3d743172e1514b196"
        },
        "date": 1750951626875,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.008596213311511183,
            "unit": "Mgas/s"
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
          "distinct": false,
          "id": "3741a2ad5647ad1945907dfe6f1ac02d65054bc4",
          "message": "fix(l1, levm): fix remaining blockchain test (#3293)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n- Fix last blockchain test, it was failing for both LEVM and REVM\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- The test destroyed an account that had non-empty storage and then\nre-created it with CREATE2. When getting the AccountUpdates we just said\nthat the account had no added storage (which is true) but we don't have\na way to directly communicate that the account was destroyed and then\ncreated again, so even though it exists its old storage should be\ncleared.\n- For this I implemented an ugly solution. For both LEVM and REVM in\nget_account_updates if I see that an account was Destroyed but now\nexists what I'll do is I'll push 2 Account Updates, one that removes the\naccount and another one with the new state of the account, so that the\nwhole account is removed (and therefore, its storage) and then we write\nto the database the new state of the account with it's new storage. I\nthink a cleaner solution would be to have an attribute `removed_storage`\n(or similar) in `AccountUpdate` that will tell the client to remove the\nstorage of the existing account without removing all the account and\nthen we don't have to do messy things like the one I implemented. The\ndownside that I see on this new approach is that we'll have an attribute\nthat we'll hardly ever use, because it's an edge case.\n- Then, for LEVM I had to implement a `destroyed_accounts` in\n`GeneralizedDatabase` so that in `get_state_transitions()` we can check\nwhich accounts were destroyed and now exist so that we do the procedure\nthat I described above. This and many other things would be way nicer if\nwe used in LEVM our own Account struct instead of reusing the one in\nEthrex. I'm seriously considering making that change because it seems\nworth doing so, there are other reasons to pull the trigger on that.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nQuestions:\n1. Should we add a `removed_storage` to `AccountUpdate` instead? Or this\nway of implementing it (removing account and then writing it) is good\nenough? Created #3321\n2. Should we use our own Account type in LEVM so that we don't rely on\nexternal HashSets and HashMaps for some things? For this I opened #3298\n\nCloses #3283\n\n---------\n\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-06-26T15:18:38Z",
          "tree_id": "73dcb8916d9e6a46cc1f6b47ab5c31c7b2ba2616",
          "url": "https://github.com/lambdaclass/ethrex/commit/3741a2ad5647ad1945907dfe6f1ac02d65054bc4"
        },
        "date": 1750955291352,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00865286051619989,
            "unit": "Mgas/s"
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
          "id": "09cac25ff1b390c5a03ab1846a46ef82283e8e2e",
          "message": "fix(l1): flaky test `test_peer_scoring_system ` (#3301)\n\n**Motivation**\nThe test executes a function that selects peers randomly but\nprioritizing those with high score, and checks that the number of\nselections for each peer is proportional to its score. However given\nthat the selection is somehow random, this is not always the case.\n\n**Description**\nIntroduces the following changes in the test\n* Increments the number of selections, which should reduce the\nprobability of failure.\n* Initializes a different `KademliaTable` for working with multiple\npeers.\nNote that the table used for multiple-peer scoring checks was the same\nas the one used for single-peer scoring tests. The problem is that a\nhigh-scoring peer from the initial phase remains in the table but is\nincorrectly omitted from subsequent multi-peer selection calculations,\nthus impacting the final outcome.\n\nThe following bash script can be used to get a sense of the failure rate\nof the test. It loops running the test and printing the total and failed\nruns.\n\nWith these changes, the failure rate dropped from (approximately) 4% to\n0.025%.\n\n```bash\n#!/bin/bash\n\nCOMMAND=(cargo test --package=ethrex-p2p --lib -- --exact kademlia::tests::test_peer_scoring_system --nocapture)\n\ntotal=0\nfailed=0\n\nwhile true; do\n    \"${COMMAND[@]}\" >/dev/null 2>&1\n    exit_code=$?\n\n    if [ $exit_code -ne 0 ]; then\n        failed=$((failed + 1))\n        echo \"❌ failed\"\n    else\n        echo \"✅ ok\"\n    fi\n\n    total=$((total + 1))\n    echo \"Total = $total, Failed = $failed\"\n    echo \"---\"\ndone\n\n```\n\n\n\nCloses #3191",
          "timestamp": "2025-06-26T16:03:58Z",
          "tree_id": "0b5e1ee9b62d541aa34180f07b73361f632a71be",
          "url": "https://github.com/lambdaclass/ethrex/commit/09cac25ff1b390c5a03ab1846a46ef82283e8e2e"
        },
        "date": 1750958849926,
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
          "id": "e12f615f1a6e67c1fb719f6693d6627d8e2a2e70",
          "message": "docs(l1): fix readme links and improve L1 documentation (#3340)\n\n**Motivation**\n\nThe current readme links to the unrendered documentation instead of our\nhosted book. Also, the general landing page doesn't have any links or\npointers on where to go next, while the L1 landing page is empty.\n\n**Description**\n\nThis PR addresses the previous issues, adding some content to the L1\nlanding page and generally cleaning up the docs. It also merges the two\ndocumentation sections in the readme and updates links to point to\ndocs.ethrex.xyz",
          "timestamp": "2025-06-26T18:13:10Z",
          "tree_id": "9da9d65d7fc47ea4423505e1996ffbbca8421bcc",
          "url": "https://github.com/lambdaclass/ethrex/commit/e12f615f1a6e67c1fb719f6693d6627d8e2a2e70"
        },
        "date": 1750965915365,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00861501312192455,
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
          "distinct": true,
          "id": "a0eebfa256edaaf3ffc35676fc3b3ad021dccde8",
          "message": "ci(l2): build docker images for all l2 tests (#3342)\n\n**Motivation**\n\nAfter #3338 state reconstruct test and based tests started failing\n**Description**\n\n- build the docker image for those steps too",
          "timestamp": "2025-06-26T18:13:36Z",
          "tree_id": "3f597839d3d2dc64484734cb7189722eede96728",
          "url": "https://github.com/lambdaclass/ethrex/commit/a0eebfa256edaaf3ffc35676fc3b3ad021dccde8"
        },
        "date": 1750968676207,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00865286051619989,
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
          "id": "8675fe8bc44977275db954cc4730a79c382cc15a",
          "message": "perf(levm): refactor levm jump opcodes (#3275)\n\n**Motivation**\n\nThe `JUMP` and `JUMPI` opcodes need to check the target address's\nvalidity. This is currently done with a `HashSet` of valid target\naddresses, which caused the hashing to become a significant part of the\nprofiling time when checking for address validity.\n\n**Description**\n\nThis PR rewrites the `JUMPDEST` checks so that instead of having a\nwhitelist, we do the following:\n- Check the program bytecode directly. The jump target's value should be\na `JUMPDEST`.\n- Check a blacklist of values 0x5B (`JUMPDEST`) that are NOT opcodes\n(they are part of push literals).\n\nThe blacklist is not a `HashMap`, but rather a sorted slice that can be\nchecked efficiently using the binary search algorithm, which should\nterminate on average after the first or second step.\n\nRational: After extracting stats of the first 10k hoodi blocks, I found\nout that...\n- There are almost 60 times more `JUMPDEST` than values 0x5B in push\nliterals.\n- On average, there are less than 2 values in the blacklist. If we were\nto use a whitelist as before, there would be about 70 entries on\naverage.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nRelated to #3305",
          "timestamp": "2025-06-26T20:36:36Z",
          "tree_id": "d839a849cc638da9fe2743a8f65a578f6579a8bd",
          "url": "https://github.com/lambdaclass/ethrex/commit/8675fe8bc44977275db954cc4730a79c382cc15a"
        },
        "date": 1750972968203,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.009792951522684898,
            "unit": "Mgas/s"
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
          "id": "f6b0ba4a352725e762dc0420b84ef9198a20d640",
          "message": "chore(l2): change default chain ID (#3337)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nCurrent chain ID (1729) is causing some problems with wallets like\nMetamask as the chain ID is registered for another network.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nChanged default chain ID to 65536999 following our new naming method:\n- `65536XYY`\n- Being `X` the stage (0 for mainnet, 1 for testnet, 2 for staging,\netc.).\n- Being `YY` each specific rollup.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3312",
          "timestamp": "2025-06-26T21:35:10Z",
          "tree_id": "5c0e989bf8fcfec78d3f3aca85cd30d7ff18d6c0",
          "url": "https://github.com/lambdaclass/ethrex/commit/f6b0ba4a352725e762dc0420b84ef9198a20d640"
        },
        "date": 1750976433657,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.009854195747342089,
            "unit": "Mgas/s"
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
          "id": "19cfb4e1874c35f32ff78602093f1305c17b6f4d",
          "message": "chore(core): add install script (#3273)\n\n**Description**\n\nThis PR adds an installation script with readme instructions on how to\nquickly set up a local L1 with `ethrex`, without having to clone the\nrepo.\n\nThe idea is to extend this script once the L2 can be more easily\ndeployed. Right now, it requires installing two more binaries and\ndownloading multiple contracts (either the source or compiled\nartifacts), which makes this more complex than it needs to be.\n\n---------\n\nCo-authored-by: Javier Chatruc <jrchatruc@gmail.com>",
          "timestamp": "2025-06-26T21:40:31Z",
          "tree_id": "cfb5520d0c3f5cac05f111e8f19749137c261c90",
          "url": "https://github.com/lambdaclass/ethrex/commit/19cfb4e1874c35f32ff78602093f1305c17b6f4d"
        },
        "date": 1750978457260,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.009829606363069246,
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
          "id": "b584bcc7e51507e5c943305e8d63dfc9f6f7ec68",
          "message": "refactor(l1, l2): remove `l2` feature flag from `rpc` crate (first proposal) [RFC] (#3330)\n\n**Motivation**\n\nThe main goal is to decouple L2 logic from the crate `rpc`. \n\nThis PR proposes an initial solution to this.\n\n**Description**\n\nThis solution extends `ethrex-rpc` by wrapping it where needed, and\nadding new data structures. I think a better solution can be achieved\nfrom this one.\n\n- Adds `ethrex-l2-rpc` crate in `crates/l2/networking/rpc`.\n- Moves L2 logic from `ethrex-rpc` to `ethrex-l2-rpc`.\n- Refactors some functions in `ethrex-rpx` to be reusable in\n`ethrex-l2-rpc`.\n- Exposes some functions and types from `ethrex-rpc`.\n- Updates `cmd/ethrex` with this changes and moves L2 initializers from\n`cmd/ethrex/initializers.rs` to `cmd/ethrex/l2/initializers.rs`.\n\n**Pros and Cons**\n\n| Pros | Cons|\n| --- | --- |\n| L2 logic is decoupled from `ethrex-rpc` | L2 logic is decoupled from\n`ethrex-rpc` by wrapping `ethrex-rpc` functions and duplicating some\ntypes to extend them |\n| `ethrex-rpc` logic is reused by `ethrex-l2-rpc` | Some types and\nfunctions were exposed in `ethrex-rpc` public API |\n\nDespite the cons, this could be an acceptable first iteration to a\nproper solution as this implementation highlights somehow which parts of\nthe rpc crate need to be abstracted.\n\n**Next Steps**\n\n- Remove `l2` feature flag from `cmd/ethrex`.\n- Move `crates/networking/rpc/clients` module.\n- Cleanup `ethrex-rpc` public API.\n- The next iteration should include an more general abstraction of the\nRPC API implementation so it can be extended organically instead of by\nwrapping it.",
          "timestamp": "2025-06-27T15:44:41Z",
          "tree_id": "b6aa69359a73072b97acca21d9c2daa7abe8b880",
          "url": "https://github.com/lambdaclass/ethrex/commit/b584bcc7e51507e5c943305e8d63dfc9f6f7ec68"
        },
        "date": 1751043831514,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.009750531559405941,
            "unit": "Mgas/s"
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
          "id": "2e4bc87ddb14b2e5215d40bada54dad384741747",
          "message": "perf(core): improve u256 handling, improving PUSH and other opcodes (#3332)\n\n**Motivation**\n\nThe function `u265::from_big_endian(slice)` which is widely used in the\ncodebase can be made faster through more compile time information.\n\nWith this change, i could add a constant generic op_push\n\nBenchmarks:\n\nSeems like our current benchmarks don't measure this part of the code,\nthere is no difference in the levm bench, however external opcode\nbenchmarks show a 2-2.5x improvement on PUSH and MSTORE based benches\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3358",
          "timestamp": "2025-06-27T15:46:44Z",
          "tree_id": "ceac0361630c7d3402664dc9569e183fdae03ccb",
          "url": "https://github.com/lambdaclass/ethrex/commit/2e4bc87ddb14b2e5215d40bada54dad384741747"
        },
        "date": 1751046825998,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.009841885696439725,
            "unit": "Mgas/s"
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
          "id": "f57dd24e3a4b142f6de1f80bc2d9ee98692bb08f",
          "message": "chore(levm): improve levm vs revm bench (#3255)\n\n**Motivation**\n\nCurrently the benchmark comparisson shown for levm is not very useful\nsince there is a lot of information distributed along different tables\nand it's not easy to see the change.\n\n**Description**\n\nThis pr introduces the following changes:\n\n- Instead of having the tables compare levm and revm, now they compare\nbetween pr's obtained mean time and main' obtained mean time for both\nvms. To do this we modify the `run_benchmark_ci` function that the\ntarget runned by the ci calls to have the benchmark comparison run for\nall cases.\n\n- If nothing changed between pr and main, no message is printed, using\nas a margin of error a porcentual difference higher than 10%.\n\n- If something changed, only output the tests where there was a change,\nnothing is printed if the test that stayed the same.\n\nThe tables always show the obtained metrics in the same order despite\nthe fact one of them is not shown:\n\n| Command | \n|--------|\n|`main_revm_`|\n|`main_levm_`|\n|`pr_revm_`|\n|`pr_levm_`|\n\nFor example, in the case you do an optimization in levm that improves\nfactorial but does nothing for revm, the table would look like this:\n| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |\n|--------|--------|--------|--------|--------|\n| `main_revm_Factorial` | 138.1 ± 0.5 | 137.4 | 139.2 | 1.00 |\n| `main_levm_Factorial` | 326.4 ± 6.3 | 321.4 | 340.2 | 2.36 ± 0.05 |\n| `pr_levm_Factorial` | 223.8 ± 6.3 | 216.4 | 234.1 | 2.04 ± 0.05 |\n\nCloses #3254\n\n---------\n\nCo-authored-by: cdiielsi <49721261+cdiielsi@users.noreply.github.com>\nCo-authored-by: Camila Di Ielsi <camila.diielsi@lambdaclass.com>",
          "timestamp": "2025-06-27T16:03:04Z",
          "tree_id": "e340415a97b05069af05234e55becf17585d535c",
          "url": "https://github.com/lambdaclass/ethrex/commit/f57dd24e3a4b142f6de1f80bc2d9ee98692bb08f"
        },
        "date": 1751049030908,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0097686664600124,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "30327624+mechanix97@users.noreply.github.com",
            "name": "Mechardo",
            "username": "mechanix97"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "6d64350d80b39e755a771917f8fbd79e16db627b",
          "message": "docs(l2): based docs outdated contract addresses (#3348)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nThe contract addresses in the docs were outdated. \n\n**Description**\n\nTo avoid having to update the addresses of the contracts every time a\ncontract changes, placeholder were introduced.\n\nAlso removed the pico verifier from the contract deployment as it is no\nlonger needed.\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\n---------\n\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-06-27T19:49:45Z",
          "tree_id": "b0720e7467d3a80f93b7e9fea335bca7e7bf7c31",
          "url": "https://github.com/lambdaclass/ethrex/commit/6d64350d80b39e755a771917f8fbd79e16db627b"
        },
        "date": 1751057724511,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.009841885696439725,
            "unit": "Mgas/s"
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
          "id": "510856b92b5aa4b9da070824ea10e28a7e5ec44c",
          "message": "fix(l2): serde skip attributes incompatible with bincode (#3370)\n\n**Motivation**\n\nsame old problem which we brute fix by serializing into JSON first, was\nreintroduced with the addition of `ExecutionWitnessResult` (which has\ntwo fields that use `#[serde(skip)]`)",
          "timestamp": "2025-06-27T20:38:17Z",
          "tree_id": "80b73f14440435541056a670f63a1e8f6e4f8173",
          "url": "https://github.com/lambdaclass/ethrex/commit/510856b92b5aa4b9da070824ea10e28a7e5ec44c"
        },
        "date": 1751060011879,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.009257849001175088,
            "unit": "Mgas/s"
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
          "id": "c9c750d8b1c2b0ab8cba89a3657f123dabe6f3fc",
          "message": "perf(levm): reduce handle_debug runtime cost (#3356)\n\n**Motivation**\n\nWith this handle_debug disappears from samplies on push/memory heavy\nbenches.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3355",
          "timestamp": "2025-06-27T20:44:26Z",
          "tree_id": "6ddd2bd3f702e9b92bede8dab5e830166d6e8827",
          "url": "https://github.com/lambdaclass/ethrex/commit/c9c750d8b1c2b0ab8cba89a3657f123dabe6f3fc"
        },
        "date": 1751063214953,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.009323585207100592,
            "unit": "Mgas/s"
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
          "id": "2dd0e7b4e73f1578fbd2c43e2845bcfb2fd46e8c",
          "message": "chore(levm): add push/mstore based bench (#3354)\n\n**Motivation**\n\nAdds a benchmark that mainly tests PUSH and MSTORE\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3364\n\n---------\n\nCo-authored-by: Javier Chatruc <jrchatruc@gmail.com>\nCo-authored-by: ilitteri <ilitteri@fi.uba.ar>",
          "timestamp": "2025-06-27T20:49:14Z",
          "tree_id": "92222d56d1d8b08b0d73e056fee5f4f6efcab91f",
          "url": "https://github.com/lambdaclass/ethrex/commit/2dd0e7b4e73f1578fbd2c43e2845bcfb2fd46e8c"
        },
        "date": 1751068837352,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.009318071555292726,
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
          "distinct": true,
          "id": "7b626fddc2a1556db2c61af9b76ce0e26dd352eb",
          "message": "refactor(l2): remove blobs logic from payload_builder (#3326)\n\n**Motivation**\n\nRemoves unnecessary logic for handling blob transactions in\n`L2::payload_builder`, since these transactions are discarded just a few\nlines below.\n\n\nCloses None",
          "timestamp": "2025-06-27T21:03:02Z",
          "tree_id": "5d209dd66d6568c111588f3f7624f8553a67c85f",
          "url": "https://github.com/lambdaclass/ethrex/commit/7b626fddc2a1556db2c61af9b76ce0e26dd352eb"
        },
        "date": 1751071114199,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.009274195997645673,
            "unit": "Mgas/s"
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
          "id": "4d6cbf6db95cff5b0fe11af74dd4c629a3e1be71",
          "message": "feat(l1): embed genesis of public networks in binary (#3372)\n\n**Motivation**\n\nCurrently, users need to download the genesis to be able to run a\nnetwork without being in the repo's root.\n\n**Description**\n\nThis PR embeds the genesis file of known public networks inside the\ncompiled binary. It has the downside of increasing binary size from 23.6\nMb to 24.7 Mb.\n\nFurther code simplifications are left for other PRs.\n\nIn the future, a possible improvement could be to parse each genesis\nfile before embedding it in the binary. Now we just embed the plain\nJSON.\n\nRelated #3292",
          "timestamp": "2025-06-27T21:16:21Z",
          "tree_id": "22bbe2194dc8cf877b504540879f04fd9ffd8b9e",
          "url": "https://github.com/lambdaclass/ethrex/commit/4d6cbf6db95cff5b0fe11af74dd4c629a3e1be71"
        },
        "date": 1751074306317,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.009296082005899705,
            "unit": "Mgas/s"
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
          "id": "25432a8606f153031410173b56c7a283fd08cc9e",
          "message": "chore(l2): remove save state module (#3119)\n\n**Motivation**\n\nThe save state module was unnecessarily complex for what it achieved\n(storing incoming batch proofs). This PR replaces it with the rollup\nstore.\n\n**Description**\n\n- adds tables for storing batch proofs indexed by batch number and proof\ntype to the rollup store\n- removes the save state module\n- all places that used the save state module now use the rollup storage\n- had to move the prover interface into `ethex-l2-common` because\n`ethrex-storage-rollup` can't depend on the prover because of cyclic\ndependencies\n- because of the previous point had to move the `Value` type into\n`ethrex-l2-common` because `ethrex-l2-common` can't depend on\n`ethrex-sdk`\n\n---------\n\nCo-authored-by: Tomás Paradelo <112426153+tomip01@users.noreply.github.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>",
          "timestamp": "2025-06-27T21:32:19Z",
          "tree_id": "c551144c82bed4a41367a84ded8abb3732a39e43",
          "url": "https://github.com/lambdaclass/ethrex/commit/25432a8606f153031410173b56c7a283fd08cc9e"
        },
        "date": 1751078032301,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00935680463182898,
            "unit": "Mgas/s"
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
          "id": "3f65132f7fb6d206f189754ace88340184811e7d",
          "message": "fix(l2): update rpc job cache file (#3377)\n\n**Motivation**\n\nin #3370 i didn't update it after changing how the program input is\nserialized",
          "timestamp": "2025-06-27T22:22:17Z",
          "tree_id": "cdf056c0a40fbe670409308b79b3f0b9f5f937ba",
          "url": "https://github.com/lambdaclass/ethrex/commit/3f65132f7fb6d206f189754ace88340184811e7d"
        },
        "date": 1751079782054,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006880061855670103,
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
          "id": "a51ad76d5dc1ca9ad6bb371037c9894a01af356a",
          "message": "chore(core): update Rust version to 1.88 (#3382)",
          "timestamp": "2025-06-30T13:04:49Z",
          "tree_id": "7b9134c67f0cd8bbceeb6285c954f84e85063274",
          "url": "https://github.com/lambdaclass/ethrex/commit/a51ad76d5dc1ca9ad6bb371037c9894a01af356a"
        },
        "date": 1751290377502,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006809857142857143,
            "unit": "Mgas/s"
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
          "id": "98512676ba767d6c635633da9b70932af1a2c483",
          "message": "docs(l1): fix broken links in sync doc (#3369)\n\n**Motivation**\nSome links in the sync documentation do not point to the correct files\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Fix broken links to diagrams in sync documentation\n* Remove link and mention of no longer existing diagram\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-06-30T13:15:09Z",
          "tree_id": "c1ab3a358155856f42742f8819e4ba0c51fb07cf",
          "url": "https://github.com/lambdaclass/ethrex/commit/98512676ba767d6c635633da9b70932af1a2c483"
        },
        "date": 1751292314912,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006880061855670103,
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
          "id": "4b6318fb291bc731b4123fd88e9398c5aacb4b30",
          "message": "feat(l2): value sending in privileged transactions (#3320)\n\n**Motivation**\n\nWe want to be able to send (\"deposit\") value-carrying privileged\ntransactions, for example to do forced withdrawals.\n\n**Description**\n\nThis PR allows users to specify the transaction value while sending\nprivileged transactions.\n\nFor regular deposits, the bridge calls it's L2 version and relies on the\nL2 hook allowing the bridge address to send without having funds. It\ncalls it's own L2 version, which handles failed transfers by making a\nwithdrawal.\n\nFor transactions, since they can't fail they are instead made to revert\nwhen there aren't enough funds to cover the operation.\n\nCloses #3290, closes #3291\n\n---------\n\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>",
          "timestamp": "2025-06-30T13:28:49Z",
          "tree_id": "0e7d86f866aa0e1081ab669a2f064c76a1c0eb95",
          "url": "https://github.com/lambdaclass/ethrex/commit/4b6318fb291bc731b4123fd88e9398c5aacb4b30"
        },
        "date": 1751294265710,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006988125654450262,
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
          "distinct": true,
          "id": "11384d62135ed354397c0a02b57adbe8d8392a8b",
          "message": "ci(l2): use build-push-action to build L1 dev docker image (#3357)\n\n**Motivation**\n\nuse official\n[build-push-action](https://github.com/docker/build-push-action)",
          "timestamp": "2025-06-30T14:03:04Z",
          "tree_id": "34ce27d88d8e5c5e41e2acbbd24f840ad8e44840",
          "url": "https://github.com/lambdaclass/ethrex/commit/11384d62135ed354397c0a02b57adbe8d8392a8b"
        },
        "date": 1751300426436,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006775289340101523,
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
          "distinct": true,
          "id": "57b2a2b26933eebf3e878a05d1c46aa7a898fd0c",
          "message": "fix(l2): remove requests processing in build_payload (#3300)\n\n**Motivation**\n\nWhen producing a block in L2 mode, we unnecessarily call\n`blockchain::extract_requests`, even though no requests are being\ngenerated.\n\n**Description**\n\n- Refactors `PayloadBuildContext` to store `requests:\nOption<Vec<EncodedRequests>>` and calculates the `request_hash` directly\nin `finalize_payload`.\n- Removes the `blockchain::extract_requests` call from\n`payload_builder`.\n\n\nCloses None",
          "timestamp": "2025-06-30T14:18:21Z",
          "tree_id": "bac5bdc1fbd5ae7e0f729563d2de21a7df9e4ff5",
          "url": "https://github.com/lambdaclass/ethrex/commit/57b2a2b26933eebf3e878a05d1c46aa7a898fd0c"
        },
        "date": 1751303793407,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006809857142857143,
            "unit": "Mgas/s"
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
          "id": "2ebb67cf51f137c75f26bdfa66540f5ea8835d6e",
          "message": "chore(l1,l2): reorder fixtures (#3155)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nThe `tests_data` folder was unorganized \n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nCreated a `fixtures` folder inside of test data with a folder for each\ntype of file.\nThe current structure looks like this:\n```\nfixtures/\n├── genesis/                    # All genesis files\n│   └── *.json\n│   \n├── network/                    # Network configuration\n│   ├── params.yaml\n│   └── hive_clients\n|       └── *.yml\n|\n├── contracts/                  # Smart contracts for testing\n│   ├── ERC20/\n│   │   ├── ERC20.sol\n│   │   ├── ERC20.bin\n│   │   |   └── TestToken.bin\n│   │   └── deps.sol\n│   ├── load-test/\n│   |   └── IOHeavyContract.sol\n|   └──levm_print\n|        └── Print.sol\n|\n├── blockchain/                 # Blockchain data files\n│   └── *.rlp\n|\n├── blobs/                      # BLOB files\n│   └── *.blob\n|\n├── keys/                       # Private keys for testing\n│   ├── private_keys.txt\n│   └── private_keys_l1.txt\n|\n├── cache/                      # Cached data\n│   └── rpc_prover/\n│       └── cache_3990967.json\n|\n└── rsp/                       \n    └── input/\n        └── 1/\n             └── 21272632.bin\n ```\nAll references were updated to avoid breaking the code\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3006\n\n---------\n\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-06-30T15:33:31Z",
          "tree_id": "d4cc7189a817b84df69f40c674589bbf9ad8a4d8",
          "url": "https://github.com/lambdaclass/ethrex/commit/2ebb67cf51f137c75f26bdfa66540f5ea8835d6e"
        },
        "date": 1751304367669,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006809857142857143,
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
          "distinct": true,
          "id": "bc56f4764a782fd952a56308dd8e28c41a190a83",
          "message": "feat(l2): verify batches in chunks with aligned (#3242)\n\n**Motivation**\n\nAligned verifies proofs in batches, so it's possible to have an array of\nproofs ready to be verified at once.\n\n**Description**\n\n- Modifies `l1_proof_verifier` to check all already aggregated proofs\nand build a single verify transaction for them.\n- Updates `verifyBatchAligned()` in the `OnChainProposer` contract to\naccept an array of proofs.\n\n> [!WARNING]\n> #3276 was accidentally merged into this PR, so the diff includes\nchanges from both.\n\nCloses #3168\n\n---------\n\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-06-30T15:34:32Z",
          "tree_id": "8a028358cc3b77a2e6c4a4e70df7faef6669dded",
          "url": "https://github.com/lambdaclass/ethrex/commit/bc56f4764a782fd952a56308dd8e28c41a190a83"
        },
        "date": 1751304939047,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0066736600000000005,
            "unit": "Mgas/s"
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
          "id": "b8dffc6936af6669927bb1657f19fb6756dabf13",
          "message": "fix(l2): parse default value correctly (#3317)\n\n**Description**\n\nThis PR fixes a panic we had when the user didn't specify any of the\n`proof-coordinator.*` flags. It seems clap called the `Default::default`\nimplementation, which panicked because of the parsing not having support\nfor leading `0x`.\n\nCloses #3309",
          "timestamp": "2025-06-30T17:01:49Z",
          "tree_id": "a1dd4d6768884e79ec22aa6bf5dc3a80898432a5",
          "url": "https://github.com/lambdaclass/ethrex/commit/b8dffc6936af6669927bb1657f19fb6756dabf13"
        },
        "date": 1751305519630,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006809857142857143,
            "unit": "Mgas/s"
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
          "id": "6e15e05062457168e668c3bd58f9c7cc4017f3f4",
          "message": "docs(l2): document ETH and ERC20 deposits/withdrawals (#3223)\n\n**Description**\n\nThis PR updates the withdrawals documentation with native ERC20\nwithdrawals and adds documentation for L2 deposits.\n\n---------\n\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-06-30T17:06:43Z",
          "tree_id": "55ba012f1c1de7304da9e0753767a034e0176a79",
          "url": "https://github.com/lambdaclass/ethrex/commit/6e15e05062457168e668c3bd58f9c7cc4017f3f4"
        },
        "date": 1751314491935,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006775289340101523,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "damian.ramirez@lambdaclass.com",
            "name": "Damian Ramirez",
            "username": "damiramirez"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "f32b20294993af9eb01f152ff1861ce17bad180e",
          "message": "docs(core): add LaTex support (#3398)\n\n**Motivation**\n\nhttps://docs.ethrex.xyz/, isn't rendering LaTex expression \n\n**Description**\n\nThis PR adds the [mdbook-katex](https://github.com/lzanini/mdbook-katex)\npreprocessor to `book.toml` to enable LaTeX rendering.\n\nRun the following command to check how the documentation looks\n\n```bash\nmake docs-deps && make docs-serve\n```\n\n**Before**\n\n![image](https://github.com/user-attachments/assets/393cf3f8-bd4a-455a-bba0-4e46cc4e42f0)\n\n**After**\n\n![image](https://github.com/user-attachments/assets/8e6954ad-9050-4161-ada2-2f3d0e13a804)",
          "timestamp": "2025-06-30T18:04:46Z",
          "tree_id": "f67ea03499eb83538e48885b04d35ce1c113c2a4",
          "url": "https://github.com/lambdaclass/ethrex/commit/f32b20294993af9eb01f152ff1861ce17bad180e"
        },
        "date": 1751315111768,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006741070707070708,
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
          "id": "8795e325f112d967cee60b965d64a6945dec068f",
          "message": "fix(l2): based CI (#3404)\n\n**Motivation**\n\nIn #3242, `verifyBatchesAligned()` was updated in the based\n`OnChainProposer` to be consistent with the non-based one, but the\n`onlySequencer` identifier was added by mistake.\n\n**Description**\n\nRemoves the `onlySequencer` identifier.\n\nCloses None",
          "timestamp": "2025-06-30T18:11:54Z",
          "tree_id": "7c28ebcd53dded82020376b2a62d37f3caba2597",
          "url": "https://github.com/lambdaclass/ethrex/commit/8795e325f112d967cee60b965d64a6945dec068f"
        },
        "date": 1751324073238,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006741070707070708,
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
          "id": "3d375136a47d1e74bfe5db3ad5e21fd3481c3e55",
          "message": "feat(l2): implement ERC20 bridge (#3241)\n\n**Motivation**\n\nWe want to be able to bridge ERC20 tokens.\n\n**Description**\n\nThe inner workings are explained on #3223\n\n---------\n\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Manuel Iñaki Bilbao <manuel.bilbao@lambdaclass.com>",
          "timestamp": "2025-06-30T19:19:40Z",
          "tree_id": "7b4fb4915fb461d10b28a55ed7755a3c5f692e1d",
          "url": "https://github.com/lambdaclass/ethrex/commit/3d375136a47d1e74bfe5db3ad5e21fd3481c3e55"
        },
        "date": 1751324654884,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006809857142857143,
            "unit": "Mgas/s"
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
          "id": "0cae5580655068ccad5aa3e5d1fdf357a459c384",
          "message": "feat(l2): add instance info to Grafana alerts (#3333)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nWe need to easily differentiate between environments when alerts come up\n(staging-1, staging-2, etc.).\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nAdd an \"$INSTANCE\" variable in the Slack contact point so it's\nover-ridden with an env var.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-06-30T19:52:03Z",
          "tree_id": "b2e3209043911fd5929de4a4ebf5af80232542d8",
          "url": "https://github.com/lambdaclass/ethrex/commit/0cae5580655068ccad5aa3e5d1fdf357a459c384"
        },
        "date": 1751328015594,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006844779487179487,
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
          "id": "68f360fc345e8064a67f266c68c00e7439db961b",
          "message": "feat(l2): exchange commit hash in node-prover communication (#3339)\n\n**Motivation**\n\nWe want to prevent a divergence between the code that is running in the\nL2 node and the prover.\n\n**Description**\n\n- Updates the client version to use `GIT_BRANCH` and `GIT_SHA` instead\nof `RUSTC_COMMIT_HASH`.\n- Adds a `build.rs` script for both the node and prover, using\n`vergen_git2` to export the git env vars.\n- Adds a `code_version` field to the `BatchRequest` message.\n- Introduces a new `ProofData` message: `InvalidCodeVersion`.\n\n## How to test\n\nYou can create an empty commit with:\n\n```bash\ngit commit --allow-empty -m \"empty commit\"\n```\n\nThen run the node and the prover using different commits.\n\n> [!WARNING]\n> Remember to run `make build-prover` whenever you change the commit\n\nCloses #3311",
          "timestamp": "2025-06-30T20:59:41Z",
          "tree_id": "d5078c5b0bbbe506fab9ce26a087572ff05c0971",
          "url": "https://github.com/lambdaclass/ethrex/commit/68f360fc345e8064a67f266c68c00e7439db961b"
        },
        "date": 1751334341230,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0067071959798994975,
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
          "distinct": true,
          "id": "8940f50dcac4ff9ca7b22a86ff2534450c50097e",
          "message": "refactor(l1, l2, levm): remove `l2` feature flag from crates `ethrex-vm` and `ethrex-levm` (#3367)\n\n**Motivation**\n\nMy primary goal was to remove the `l2` feature flag from `cmd/ethrex`\nbut to do this, we first need to remove it from:\n- `ethrex-vm`.\n- `ethrex-levm`.\n- `ethrex-blockchain`.\n\n**Description**\n\nThis PR removes the feature flag `l2` from crates `ethrex-vm` and\n`ethrex-levm`.\n\n> *TL;DR:*\n> - In `ethrex-vm` the l2 precompiles logic was moved to a separate\nmodule, `l2_precompiles`.\n> - A new `VMType` enum was introduced in `ethrex-levm` as a field of\n`VM` (main LEVM's struct). It is used by LEVM to behave differently\nwhere needed (this is specifically, when executing precompiles, and when\nexecuting hooks).\n> - A new `BlockchainType` enum was introduced in `ethrex-blockchain` as\na field of the struct `Blockchain` to differentiate when nodes are\nstarted as L1 or L2 nodes (this is later used in the code to instantiate\nthe VM properly, matching the `BlockchainType` variants with `VMType`\nones).\n\nThe `l2` feature flag exists in `ethrex-vm` only because of\n`ethrex-levm`, so to remove it I needed to remove it from `ethrex-levm`\nfirst. The following commits do that:\n- [Move l2 precompiles logic to new\nmodule](https://github.com/lambdaclass/ethrex/commit/28843a6b7b7bee0cacc95589e66190bdae510f94)\n- [Remove feature flag from hooks public\nAPI](https://github.com/lambdaclass/ethrex/commit/39a509fc7046dd2ffb34c405db89e1f38aead490)\n- [Use the correct\nfunctions](https://github.com/lambdaclass/ethrex/commit/3023b88d96455337f6b1fb6d34ea2c6d087b3518)\n- [Replace\nget_hooks](https://github.com/lambdaclass/ethrex/commit/88bc9a25691b06663600e4afe75f30332517f039)\n- [Remove l2 feature flag from\nlevm](https://github.com/lambdaclass/ethrex/commit/8b098836b23fcdee1c85294d33090cd30f77c689)\n\nAfter that, it was almost safe to remove it from `ethrex-vm`:\n- [Remove l2 feature flag from vm\ncrate](https://github.com/lambdaclass/ethrex/commit/fd971bec15d0934ccde5f6d25b16a4d16d0693df)\n\nThis brought some compilation errors that were solved in:\n- [Implement BlockchainType and fix\ncompilation](https://github.com/lambdaclass/ethrex/commit/32557eb7cabcefc935f2d525354ab981870af45f)\n\n**Next Steps**\n\n- Remove feature flag `l2` from `ethrex-blockchain` crate.\n- Remove feature flag `l2` from `cmd/ethrex`.\n\n---------\n\nCo-authored-by: Jeremías Salomón <48994069+JereSalo@users.noreply.github.com>",
          "timestamp": "2025-06-30T21:11:30Z",
          "tree_id": "ca1641496a92408cdb3c69075de725cb3d1d00f2",
          "url": "https://github.com/lambdaclass/ethrex/commit/8940f50dcac4ff9ca7b22a86ff2534450c50097e"
        },
        "date": 1751335013598,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006775289340101523,
            "unit": "Mgas/s"
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
          "id": "cdc0bb8c2fb459bf62deb25bdc82e3d9418e6281",
          "message": "fix(core): more accurate throughput (#3412)\n\nThroughput in the logged metrics was computed over a truncated number of\nseconds, which meant the same block taking 1999ms or 1000ms reports the\nsame throughput, when one is indeed twice as slow as the other.\nThis fixes it by asking for the `as_secs_f64` directly rather than\ntaking an integer number of millis, dividing (with integer semantics) by\n1000 and then casting.",
          "timestamp": "2025-06-30T21:29:01Z",
          "tree_id": "0aac684e2a008af2325ffa0d11310c21428d1aed",
          "url": "https://github.com/lambdaclass/ethrex/commit/cdc0bb8c2fb459bf62deb25bdc82e3d9418e6281"
        },
        "date": 1751340473458,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006809857142857143,
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
          "id": "d62fcc5accaaa13b9909af04071379219dc6a0b1",
          "message": "perf(levm): refactor `CacheDB` to use more efficient APIs (#3259)\n\n**Motivation**\n\nThe cache db is a bunch of functions that accept a state object as an\nargument. This is confusing since those are not methods, but functions,\nwhich also do stuff that the state object already supports natively (not\nto mention the duplicated function).\n\n**Description**\n\nRemove the `cache.rs` file and use the state object directly. Move stuff\nto more relevant places to fix borrow issues.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-01T11:14:17Z",
          "tree_id": "56071286cc95f7df094c33217f324109aa83f6c2",
          "url": "https://github.com/lambdaclass/ethrex/commit/d62fcc5accaaa13b9909af04071379219dc6a0b1"
        },
        "date": 1751372272286,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006915709844559585,
            "unit": "Mgas/s"
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
          "id": "d6d79a4d1781a6aa39dd9dfb3cc6e79cfe7350e6",
          "message": "perf(levm): add fib recursive bench (#3391)\n\n**Motivation**\nThe fibonacci recursive can show perfomance results of stack reuse that\nthe factorial recursive one can't because factorial will never be able\nto \"reuse\" the stack.\n\nSee also https://github.com/lambdaclass/ethrex/pull/3386\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-01T13:47:40Z",
          "tree_id": "c9c109f9a6cf0890e08cf3280f85eda9dd076a88",
          "url": "https://github.com/lambdaclass/ethrex/commit/d6d79a4d1781a6aa39dd9dfb3cc6e79cfe7350e6"
        },
        "date": 1751381475003,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006844779487179487,
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
          "id": "cffc84b07c17135c5a87cebbae4c1bbaae61c202",
          "message": "feat(l2): replace custom merkle tree with `OpenZeppelin` + `lambdaworks` (#3344)\n\n**Motivation**\n\nWe are using an unsafe (not audited) custom merkle tree implementation\nfor the L1messaging system\n\n**Description**\n\n- Replace the merkle tree verify function in the CommonBridge contract\nto use OppenZeppelin's `MerkleProof.sol` contract\n- Replace our custom merkle tree implementation with lambdaworks' for\nthis:\n- We implement the trait `IsMerkleTreeBackend` for H256 to build a tree\nthat is compliant with\n- https://docs.openzeppelin.com/contracts/5.x/api/utils#MerkleProof\n - The implementation is taken from \n-\nhttps://github.com/yetanotherco/aligned_layer/blob/8a3a6448c974d09c645f3b74d4c9ff9d2dd27249/batcher/aligned-sdk/src/aggregation_layer/types.rs",
          "timestamp": "2025-07-01T15:16:50Z",
          "tree_id": "fa2255e61c907a119f01fe63cb1e5cdcd6160904",
          "url": "https://github.com/lambdaclass/ethrex/commit/cffc84b07c17135c5a87cebbae4c1bbaae61c202"
        },
        "date": 1751384443753,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006844779487179487,
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
          "distinct": true,
          "id": "416581dc8833dccf10121a116b1fd462b2bb1644",
          "message": "feat(l2): burn gas when sending privileged transactions (#3407)\n\n**Motivation**\n\nTo prevent users from sending L2 transactions for free, we must charge\nthem for the gas sent.\n\n**Description**\n\nOne way to do this is to burn the gas limit specified at L1 prices.\n\nCloses #3402, closes #2156",
          "timestamp": "2025-07-01T15:24:31Z",
          "tree_id": "2abf5e9f8515e6822bfdb5d83421a77346f61133",
          "url": "https://github.com/lambdaclass/ethrex/commit/416581dc8833dccf10121a116b1fd462b2bb1644"
        },
        "date": 1751387476733,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006844779487179487,
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
          "id": "8c1812c8b0a657ac11392bda47bc88034bd10c67",
          "message": "feat(l2): implement batch endpoint (#3374)\n\n**Motivation**\n\nFor debugging purposes, it's useful to have an `ethrex_getBatchByNumber`\nendpoint that returns a `Batch` struct:\n\n```Rust\npub struct Batch {\n    pub number: u64,\n    pub first_block: u64,\n    pub last_block: u64,\n    pub state_root: H256,\n    pub deposit_logs_hash: H256,\n    pub message_hashes: Vec<H256>,\n    pub blobs_bundle: BlobsBundle,\n    pub commit_tx: Option<H256>,\n    pub verify_tx: Option<H256>,\n}\n```\n\n**Description**\n\n- Modifies the `Batch` struct to incude `commit_tx` and `verify_tx`.\n- Updates `block_fetcher` to process verify tx logs and extract the\nverify tx hashes as well.\n- Fixes a bug found during development: the `rollup_storage::getBatch()`\nfunction incorrectly treated batches without `L1Messages` as an error.\n\n## How to test\n\nYou can run:\n```bash\ncurl -X POST http://localhost:1729 \\\n  -H \"Content-Type: application/json\" \\\n  -d '{\n    \"jsonrpc\":\"2.0\",\n    \"method\":\"ethrex_getBatchByNumber\",\n    \"params\": [\"0x1\", true],\n    \"id\":1\n  }'\n  ```\n\nCloses None\n\n---------\n\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>\nCo-authored-by: Damian Ramirez <damian.ramirez@lambdaclass.com>",
          "timestamp": "2025-07-01T22:08:29Z",
          "tree_id": "c5741d911edff60bcb50e7172cf6f59a11d5246d",
          "url": "https://github.com/lambdaclass/ethrex/commit/8c1812c8b0a657ac11392bda47bc88034bd10c67"
        },
        "date": 1751411525424,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006775289340101523,
            "unit": "Mgas/s"
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
          "id": "e46e4cbbfe3a6e95d16d3541086eddf7d55546b1",
          "message": "fix(l2): force JSON format for ProgramInput (#3397)\n\n**Motivation**\n\nThis is to avoid the old #3370 bug\n\n**Description**\n\n- creates newtype JSONProgramInput which is always serialized into JSON\nfirst\n- uses it for sp1 instead of the original ProgramInput",
          "timestamp": "2025-07-02T13:59:06Z",
          "tree_id": "e9d161bfb8333300d8756afb087e18955dad9b0c",
          "url": "https://github.com/lambdaclass/ethrex/commit/e46e4cbbfe3a6e95d16d3541086eddf7d55546b1"
        },
        "date": 1751470676721,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006479281553398058,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "94380962+sofiazcoaga@users.noreply.github.com",
            "name": "sofiazcoaga",
            "username": "sofiazcoaga"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "aa317274b1dbcc14b88823839e7525cafa3524e8",
          "message": "refactor(levm): consider all forks after Prague and not just Osaka (#3423)\n\n**Motivation**\n\nThis PR addresses issue #2773. \n\nFunctions `max_blobs_per_block()`,\n`get_blob_base_fee_update_fraction_value()` and\n`get_target_blob_gas_per_block_()` in `environment.rs` consider three\noptions: Prague fork, Osaka fork and other forks, where the first two\nhave the same course of action. The idea is to consider two points:\nprevious or posterior to Prague fork.\n\n**Description**\n\nChanges pattern matching to an `if` statement that checks whether we are\nprevious to Prague fork or past that.\n\nCloses #2773",
          "timestamp": "2025-07-02T17:34:08Z",
          "tree_id": "536fa64c7a8b0286fe9fe9b0cee6b64e667fd2db",
          "url": "https://github.com/lambdaclass/ethrex/commit/aa317274b1dbcc14b88823839e7525cafa3524e8"
        },
        "date": 1751479087265,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006386277511961722,
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
          "distinct": true,
          "id": "ef121af47cb619ad8d617fd96f469e06526ef55c",
          "message": "refactor(l1, l2): decouple L2 metrics logic from L1's & remove l2 feature flag from `ethrex-blockchain` (#3371)\n\n> [!WARNING]\n> Merge after https://github.com/lambdaclass/ethrex/pull/3367\n\n**Motivation**\n\nTo completely remove the `l2` feature flag from `cmd/ethrex` in favor of\nhaving a single binary for running ethrex (L1 and L2), there are some\nlocal dependencies from which to remove this feature first. These are:\n\n1. `ethrex-vm`.\n2. `ethrex-levm`.\n3. `ethrex-blockchain`. \n\n1 and 2 are removed in https://github.com/lambdaclass/ethrex/pull/3367,\nand 3 is meant to be removed in this PR.\n\n**Description**\n\nDecouples the L2 metrics logic from the L1's, allowing to remove the use\nof the `l2` feature flag from the crate `ethrex-blockchain`.\n\n- Creates a `crates/blockchain/metrics/l2` module with `metrics.rs` and\n`api.rs` submodules.\n- Makes use of this new module in `cmd/ethrex`.\n- Removes `l2` feature flag from `ethrex-blockchain` crate.\n- Removes the import of `ethrex-blockchain/l2` where needed.",
          "timestamp": "2025-07-02T17:52:19Z",
          "tree_id": "053cd741ce7397434b883f78d3c8dc20d8bc06db",
          "url": "https://github.com/lambdaclass/ethrex/commit/ef121af47cb619ad8d617fd96f469e06526ef55c"
        },
        "date": 1751480200131,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006844779487179487,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "94380962+sofiazcoaga@users.noreply.github.com",
            "name": "sofiazcoaga",
            "username": "sofiazcoaga"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "2983a9baead8000a97075bc1c683917db91f3f59",
          "message": "refactor(levm): obtain gas calculation params from auxiliar function (#3420)\n\n**Motivation** \n\nThis PR addresses\n[issue](https://github.com/lambdaclass/ethrex/issues/3095).\n\n**Description**\n\nOpcodes CALL, CALLCODE, DELEGATECALL and STATICCALL had each a custom\ngas calculation function but used the same input arguments and obtained\nthem with the same process.\n\nNow a new method called `get_call_gas_params()` includes these common\ncalculations and gets invoked by all opcodes handlers.",
          "timestamp": "2025-07-02T18:24:45Z",
          "tree_id": "a016c67fe51999cd1800a796edb74494eecd7fc7",
          "url": "https://github.com/lambdaclass/ethrex/commit/2983a9baead8000a97075bc1c683917db91f3f59"
        },
        "date": 1751482059850,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006775289340101523,
            "unit": "Mgas/s"
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
          "id": "1189f2f34bde1205ac7d3a1568ec28f536ea45df",
          "message": "chore(l2): re-enable risc0 (#3172)\n\n**Motivation**\n\nrisc0 support was temporarily deprecated because of incompatible\nversions between some required ethrex's dependencies and the risc0\ntoolchain.\n\nNow the toolchain uses a newer version so this problem should be solved,\nbut the backend needs some maintenance to get it working again.\n\n**Description**\n\n- update risc0 to latest version\n- update risc0's build script for the new version\n- refactor kzg verification into ethrex-common\n- support kzg verification with both kzg-rs and c-kzg (sp1 is only\ncompatible with kzg-rs, risc0 only with c-kzg)\n- fix wrong public inputs encoding\n- fix wrong image id encoding\n- add risc0 verification key (also called image id) as a contract\nvariable\n- add risc0 lint job and refactor jobs for other backends\n- add docs for local testing (deployment of risc0 contracts)\n\nCloses #2145",
          "timestamp": "2025-07-02T18:59:01Z",
          "tree_id": "26a8a49c495eb83e22a7b0df4c708bc87119278b",
          "url": "https://github.com/lambdaclass/ethrex/commit/1189f2f34bde1205ac7d3a1568ec28f536ea45df"
        },
        "date": 1751484268086,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0067071959798994975,
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
          "id": "eee2e9d5bd2c218e710c42b873e174332d21693a",
          "message": "refactor(l2): refactor privileged transactions (#3365)\n\n**Motivation**\n\nWe want to unify the terminology used to refer to sending privileged\ntransactions to the L2, since they are not just deposits.\n\nAlso, the `DepositInitiated` (now `PrivilegedTxSent`) event must be\ncleaned up: `l2MintTxHash` is irrelevant since it can be recomputed, and\nit doesn't make sense to index `amount` but not `from`.\n\nSome clean up (for example, removing `recipient`) was already done in\n#3320.\n\n**Description**\n\n* Removes the l2 transaction from the deposit event\n* Adds `indexed` to `from` and removes it from `address`\n* Renames deposit to 'privileged transactions' where more appropriate\n\nCloses #3233\n\n---------\n\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Jeremías Salomón <48994069+JereSalo@users.noreply.github.com>",
          "timestamp": "2025-07-02T19:56:25Z",
          "tree_id": "4b8a36c061ca344254acf3e14d89d0b515efed4c",
          "url": "https://github.com/lambdaclass/ethrex/commit/eee2e9d5bd2c218e710c42b873e174332d21693a"
        },
        "date": 1751488804852,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006510887804878049,
            "unit": "Mgas/s"
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
          "id": "95a28c6887bf8fa788044dd0d4b744f261409e52",
          "message": "fix(l2): risc0 replay job (#3448)",
          "timestamp": "2025-07-02T20:31:55Z",
          "tree_id": "d0205853ef4b83bdbd04fc69bd6d15b2a57abe9b",
          "url": "https://github.com/lambdaclass/ethrex/commit/95a28c6887bf8fa788044dd0d4b744f261409e52"
        },
        "date": 1751491173248,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006542803921568628,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "44068466+SDartayet@users.noreply.github.com",
            "name": "SDartayet",
            "username": "SDartayet"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "40a7deacf016a3856e432928b5a86d45234b602e",
          "message": "chore(l1): sync tooling fixes (#3064)\n\n**Motivation**\n\nAdding some minor fixes to the recently merged sync tooling.\n\n**Description**\n\nAdded some stuff to the tooling that was recently merged related to\nflamegraphs, also fixed one of the logs added which we noticed\nintroduced some noise to the info logs.\n\n---------\n\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-07-02T21:06:40Z",
          "tree_id": "97307fd199489700ebc9e19323f6f8325c49c9a1",
          "url": "https://github.com/lambdaclass/ethrex/commit/40a7deacf016a3856e432928b5a86d45234b602e"
        },
        "date": 1751492577491,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0067071959798994975,
            "unit": "Mgas/s"
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
          "id": "4f66425fae9d872e4d75fb3e6a1c52058247b8d4",
          "message": "fix(l1): incorrect tx size check causing peers to be rejected (#3450)\n\n**Motivation**\nA bug currently causes peers' pooled transactions to be incorrectly\nrejected due to a mismatch in transaction size validation.\n\n**Description**\nWhen a `PooledTransactions` message is received, the implementation\nvalidates that the transactions match the originally requested ones\n(which are requested by sending `GetPooledTransactions`).\n\nOne of these validations checks that the size of each received\ntransaction matches the expected size. However, the transaction size was\nbeing computed incorrectly, leading to false rejections.\n\nThis PR fixes the error by computing the transaction size in the right\nway. Now, the transaction size is computed according to the\n[specification](https://github.com/ethereum/devp2p/blob/master/caps/eth.md#newpooledtransactionhashes-0x08):\n\n> `txsizeₙ` refers to the length of the 'consensus encoding' of a typed\ntransaction, i.e. the byte size of `tx-type || tx-data` for typed\ntransactions, and the size of the RLP-encoded `legacy-tx` for non-typed\nlegacy transactions.\n\nTo achieve this, we now use the `encode_canonical_to_vec()` method,\nwhich returns the appropriate encoding for both typed and legacy\ntransactions. The length of this encoding is then used as the\ntransaction size.\n\nThis can be tested by setting up a localnet with `make localnet`,\nwaiting around 2 minutes and checking that there are no logs like the\nfollowing\n\n```bash\n\n2025-07-02T19:56:51.324981Z  WARN ethrex_p2p::rlpx::utils: [0x03dd…06fa(172.16.0.11:30303)]: disconnected from peer. Reason: Invalid pooled transaction size, differs from expected\n```\n\nCloses #3251",
          "timestamp": "2025-07-02T21:25:16Z",
          "tree_id": "d9b129f2aa29ec48b32138b2111a4f8ca7df5d64",
          "url": "https://github.com/lambdaclass/ethrex/commit/4f66425fae9d872e4d75fb3e6a1c52058247b8d4"
        },
        "date": 1751493251277,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006809857142857143,
            "unit": "Mgas/s"
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
          "id": "7ff4eeaf53f72e6cdc141e60ff98263eea80494a",
          "message": "perf(levm): use a stack pool (#3386)\n\n**Motivation**\n\nFixes #3385\n\nhttps://share.firefox.dev/44uRnnn\n\nThe perfomance gain from this pr cannot be seen with the factorial\nrecursive bench, because it doesn't reuse the stack, it always goes full\ndeep and then up.\n\nWith a fibonacci recursive bench it can be seen:\n\nMain\n\n![image](https://github.com/user-attachments/assets/e2fae3b0-1839-4105-afa1-8bbde4c216ae)\n\nPR\n\n![image](https://github.com/user-attachments/assets/1a12cf4b-66cf-45ce-a9dd-8079804bfb48)\n\nNeeds https://github.com/lambdaclass/ethrex/pull/3391 to show the perf\ngains in benches",
          "timestamp": "2025-07-03T11:06:14Z",
          "tree_id": "9a613b17e3e3e752a434ceb4136f474db26615b4",
          "url": "https://github.com/lambdaclass/ethrex/commit/7ff4eeaf53f72e6cdc141e60ff98263eea80494a"
        },
        "date": 1751542816183,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007062074074074074,
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
          "id": "ef3422302e9d726c275257580a81af67aa38e9f4",
          "message": "ci(l2): fix prover workflow (#3445)\n\n**Motivation**\n\nThe Integration Test Prover Sp1 workflow was broken due to several\nissues:\n- It was using outdated `contract_deployer` and `ethrex_l2` images.\n- There were permission issues on the `_work` directory.\n- `solc` was not installed.\n\nHere is a successful run of the workflow:\nhttps://github.com/lambdaclass/ethrex/actions/runs/16037084316/job/45251233852?pr=3445\n\nCloses: None",
          "timestamp": "2025-07-03T13:06:45Z",
          "tree_id": "80bf236de9f084bbabdc96c8a3809662ebb0ca77",
          "url": "https://github.com/lambdaclass/ethrex/commit/ef3422302e9d726c275257580a81af67aa38e9f4"
        },
        "date": 1751549389715,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0070249052631578945,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "estebandh@gmail.com",
            "name": "ElFantasma",
            "username": "ElFantasma"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "c6ad97cfc44c3f020aad5903523460ea0ed77d71",
          "message": "refactor(l1): spawned p2p (#3164)\n\nP2p connection processes are complex and error prone. Using `spawned` to\nclean it up and properly separate concurrency logic from business logic.\n\n**Description**\n\nReplaces the main_loop from `RLPxConnection` with a `spawned` process\nthat handles all the messages from and to the remote peer, as well as\nthe backend.\n\n---------\n\nCo-authored-by: Lucas Fiegl <iovoid@users.noreply.github.com>\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: Manuel Iñaki Bilbao <manuel.bilbao@lambdaclass.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>\nCo-authored-by: Jeremías Salomón <48994069+JereSalo@users.noreply.github.com>\nCo-authored-by: Mario Rugiero <mrugiero@gmail.com>\nCo-authored-by: MrAzteca <azteca1998@users.noreply.github.com>\nCo-authored-by: Edgar <git@edgl.dev>\nCo-authored-by: LeanSerra <46695152+LeanSerra@users.noreply.github.com>",
          "timestamp": "2025-07-03T13:14:32Z",
          "tree_id": "c65b59388161b372d09b873e05057699ee1b2c00",
          "url": "https://github.com/lambdaclass/ethrex/commit/c6ad97cfc44c3f020aad5903523460ea0ed77d71"
        },
        "date": 1751556410275,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006809857142857143,
            "unit": "Mgas/s"
          }
        ]
      },
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
          "id": "ca7e0a245c6e59d5b5d0bba2bce67b8e744b0243",
          "message": "ci(l2): ethrex replay risc0",
          "timestamp": "2025-07-03T16:01:52Z",
          "url": "https://github.com/lambdaclass/ethrex/pull/3464/commits/ca7e0a245c6e59d5b5d0bba2bce67b8e744b0243"
        },
        "date": 1751563708045,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006809857142857143,
            "unit": "Mgas/s"
          }
        ]
      },
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
          "id": "0c59b9954b1d183103e317d551f0bcb66e4810ca",
          "message": "ci(l2): ethrex replay risc0",
          "timestamp": "2025-07-03T21:16:59Z",
          "url": "https://github.com/lambdaclass/ethrex/pull/3464/commits/0c59b9954b1d183103e317d551f0bcb66e4810ca"
        },
        "date": 1751588203362,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006951729166666667,
            "unit": "Mgas/s"
          }
        ]
      },
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
          "id": "0c59b9954b1d183103e317d551f0bcb66e4810ca",
          "message": "ci(l2): ethrex replay risc0",
          "timestamp": "2025-07-03T21:16:59Z",
          "url": "https://github.com/lambdaclass/ethrex/pull/3464/commits/0c59b9954b1d183103e317d551f0bcb66e4810ca"
        },
        "date": 1751590634914,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012089963768115942,
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
          "id": "8f16da17176a34c5dec3a0284f064382430c5bfc",
          "message": "ci(l2): ethrex replay risc0 (#3464)\n\n**Motivation**\n\nRisc0 was failing to compile in our self hosted runner because the $PATH\ndidn't have nvcc\n\n\n**Description**\n\n- Add nvcc \n- Use `JSONProgramInput` for risc0 to solve bincode issue\n- Add `prove-risc0-gpu-ci` target to `cmd/ethrex_replay/Makefile` \n- Successful run\n[here](https://github.com/lambdaclass/ethrex/actions/runs/16062825245/job/45331954610)",
          "timestamp": "2025-07-04T13:34:18Z",
          "tree_id": "e733b726c2525d6f5af1377a6c8b2593ad851ce7",
          "url": "https://github.com/lambdaclass/ethrex/commit/8f16da17176a34c5dec3a0284f064382430c5bfc"
        },
        "date": 1751641585581,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012627549668874172,
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
          "id": "8f16da17176a34c5dec3a0284f064382430c5bfc",
          "message": "ci(l2): ethrex replay risc0 (#3464)\n\n**Motivation**\n\nRisc0 was failing to compile in our self hosted runner because the $PATH\ndidn't have nvcc\n\n\n**Description**\n\n- Add nvcc \n- Use `JSONProgramInput` for risc0 to solve bincode issue\n- Add `prove-risc0-gpu-ci` target to `cmd/ethrex_replay/Makefile` \n- Successful run\n[here](https://github.com/lambdaclass/ethrex/actions/runs/16062825245/job/45331954610)",
          "timestamp": "2025-07-04T13:34:18Z",
          "tree_id": "e733b726c2525d6f5af1377a6c8b2593ad851ce7",
          "url": "https://github.com/lambdaclass/ethrex/commit/8f16da17176a34c5dec3a0284f064382430c5bfc"
        },
        "date": 1751642259894,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006355866666666667,
            "unit": "Mgas/s"
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
          "distinct": false,
          "id": "4acb6a78bf85f53b36db8cf4197a765f753ad6b7",
          "message": "test(l1): fix EEST RLP tests (#3415)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- Execute `.rlp` files (each represent a block in execution-spec-tests)\nin the correct order\n- Set the canonical block right after executing it, so that if one of\nthe executed block fails then the canonical block is the previous of the\nfailed block. Before we only set the canonical block if all blocks\nexecuted succeeded.\n\n[Example\nRun](https://github.com/lambdaclass/ethrex/actions/runs/15984844429)\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3405",
          "timestamp": "2025-07-04T14:27:38Z",
          "tree_id": "75d33b5c23ab5e51fec7f6172510e3c112a21bf1",
          "url": "https://github.com/lambdaclass/ethrex/commit/4acb6a78bf85f53b36db8cf4197a765f753ad6b7"
        },
        "date": 1751649614674,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006607584158415842,
            "unit": "Mgas/s"
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
          "distinct": false,
          "id": "4acb6a78bf85f53b36db8cf4197a765f753ad6b7",
          "message": "test(l1): fix EEST RLP tests (#3415)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- Execute `.rlp` files (each represent a block in execution-spec-tests)\nin the correct order\n- Set the canonical block right after executing it, so that if one of\nthe executed block fails then the canonical block is the previous of the\nfailed block. Before we only set the canonical block if all blocks\nexecuted succeeded.\n\n[Example\nRun](https://github.com/lambdaclass/ethrex/actions/runs/15984844429)\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3405",
          "timestamp": "2025-07-04T14:27:38Z",
          "tree_id": "75d33b5c23ab5e51fec7f6172510e3c112a21bf1",
          "url": "https://github.com/lambdaclass/ethrex/commit/4acb6a78bf85f53b36db8cf4197a765f753ad6b7"
        },
        "date": 1751652236061,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012450858208955225,
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
          "distinct": true,
          "id": "ccfc231b5756c045b2d55febbb796ea63877d56a",
          "message": "ci(core): fix block benchmark ci (#3484)\n\n**Motivation**\n\nIts broken :(\n\n**Description**\n\n- The file `genesis-perf-ci.json` was moved and the reference was not\nupdated. This PR updates the reference\n- Here is a failing run in main\nhttps://github.com/lambdaclass/ethrex/actions/runs/16075239686/job/45368784296\n- Here is a test run in this pr that didn't panic when reading the file\nhttps://github.com/lambdaclass/ethrex/actions/runs/16075285386/job/45368928205.\n   - The run was cut short because it takes 40+ minutes to run",
          "timestamp": "2025-07-04T14:33:11Z",
          "tree_id": "98ae5f4311797a75c27363ff9b6ddb9d43b49a04",
          "url": "https://github.com/lambdaclass/ethrex/commit/ccfc231b5756c045b2d55febbb796ea63877d56a"
        },
        "date": 1751652934799,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006607584158415842,
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
          "distinct": true,
          "id": "ccfc231b5756c045b2d55febbb796ea63877d56a",
          "message": "ci(core): fix block benchmark ci (#3484)\n\n**Motivation**\n\nIts broken :(\n\n**Description**\n\n- The file `genesis-perf-ci.json` was moved and the reference was not\nupdated. This PR updates the reference\n- Here is a failing run in main\nhttps://github.com/lambdaclass/ethrex/actions/runs/16075239686/job/45368784296\n- Here is a test run in this pr that didn't panic when reading the file\nhttps://github.com/lambdaclass/ethrex/actions/runs/16075285386/job/45368928205.\n   - The run was cut short because it takes 40+ minutes to run",
          "timestamp": "2025-07-04T14:33:11Z",
          "tree_id": "98ae5f4311797a75c27363ff9b6ddb9d43b49a04",
          "url": "https://github.com/lambdaclass/ethrex/commit/ccfc231b5756c045b2d55febbb796ea63877d56a"
        },
        "date": 1751655486620,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001256809792843691,
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
          "id": "5abf85df328b22d2674a321d30a89dae033b01f6",
          "message": "fix(l2): gas used in replay block range (#3483)\n\n**Motivation**\n\nThe `gas_used` value in block range execution/proving was incorrect. We\nwere returning the gas used by only the first block.\n\n**Description**\n\n- Returns the total gas used across all blocks.\n- Also moves the `or_latest` function, as it doesn’t belong in `fetcher`\nanymore.\n\nCloses: None",
          "timestamp": "2025-07-04T14:57:07Z",
          "tree_id": "4fa8efbdb3ce22caed01d05826918a163151f9b5",
          "url": "https://github.com/lambdaclass/ethrex/commit/5abf85df328b22d2674a321d30a89dae033b01f6"
        },
        "date": 1751662334930,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012335785582255083,
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
          "id": "5abf85df328b22d2674a321d30a89dae033b01f6",
          "message": "fix(l2): gas used in replay block range (#3483)\n\n**Motivation**\n\nThe `gas_used` value in block range execution/proving was incorrect. We\nwere returning the gas used by only the first block.\n\n**Description**\n\n- Returns the total gas used across all blocks.\n- Also moves the `or_latest` function, as it doesn’t belong in `fetcher`\nanymore.\n\nCloses: None",
          "timestamp": "2025-07-04T14:57:07Z",
          "tree_id": "4fa8efbdb3ce22caed01d05826918a163151f9b5",
          "url": "https://github.com/lambdaclass/ethrex/commit/5abf85df328b22d2674a321d30a89dae033b01f6"
        },
        "date": 1751663063924,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006479281553398058,
            "unit": "Mgas/s"
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
          "id": "f6a839d10689bbf0c016dc7c486d73d485850b21",
          "message": "fix(l1): `RpcBlock` `uncles` field should have the hashes and not the block headers (#3245)\n\n**Motivation**\nFix inconsistencies between our RPC outputs and the spec.\nAccording to the spec endpoints such as `eth_getBlockByNumber` return a\nblock where the `uncles` field contains the hashes of the uncle blocks,\nwhile we return the full headers.\nThis has not been a problem for us as we have been mainly using\npost-merge blocks without uncles, but it will become a problem if we\nneed to export/import older blocks via rpc\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Change `uncles` field of `RpcBlock` from `Vec<BlockHeader>` to\n`Vec<H256`\n* (Bonus) Allow deserializing blocks without `base_fee_per_gas`\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses None",
          "timestamp": "2025-07-04T15:10:19Z",
          "tree_id": "4ece4b3ae184c29396de8151f4b8a22ab0a95a0b",
          "url": "https://github.com/lambdaclass/ethrex/commit/f6a839d10689bbf0c016dc7c486d73d485850b21"
        },
        "date": 1751666321077,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006575034482758621,
            "unit": "Mgas/s"
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
          "id": "0f0e27084645dc8fa4675a61ecfc2fe602cdcaf0",
          "message": "feat(l1): embed dev-mode genesis in binary (#3413)\n\n**Motivation**\n\nGiving the user a default dev-mode genesis block makes starting out with\n`ethrex` easy.\n\n**Description**\n\nThis PR:\n\n- Adds a new network option (`Network::LocalDevnet`).\n- Default to the new network when `--dev` is specified but no custom\nnetwork was specified.\n- Remove the genesis downloading step from install script.\n- Update the readme to reflect that no genesis file needs to be\nspecified; ethrex comes with batteries included 🦖\n\nCloses #3378",
          "timestamp": "2025-07-04T16:00:11Z",
          "tree_id": "2185d63d0fde7445a38117dbcc9e0c2398c6e0d1",
          "url": "https://github.com/lambdaclass/ethrex/commit/0f0e27084645dc8fa4675a61ecfc2fe602cdcaf0"
        },
        "date": 1751668153252,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006575034482758621,
            "unit": "Mgas/s"
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
          "id": "0f0e27084645dc8fa4675a61ecfc2fe602cdcaf0",
          "message": "feat(l1): embed dev-mode genesis in binary (#3413)\n\n**Motivation**\n\nGiving the user a default dev-mode genesis block makes starting out with\n`ethrex` easy.\n\n**Description**\n\nThis PR:\n\n- Adds a new network option (`Network::LocalDevnet`).\n- Default to the new network when `--dev` is specified but no custom\nnetwork was specified.\n- Remove the genesis downloading step from install script.\n- Update the readme to reflect that no genesis file needs to be\nspecified; ethrex comes with batteries included 🦖\n\nCloses #3378",
          "timestamp": "2025-07-04T16:00:11Z",
          "tree_id": "2185d63d0fde7445a38117dbcc9e0c2398c6e0d1",
          "url": "https://github.com/lambdaclass/ethrex/commit/0f0e27084645dc8fa4675a61ecfc2fe602cdcaf0"
        },
        "date": 1751670784521,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001216711030082042,
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
          "distinct": true,
          "id": "b08fa01cfcefc97428d5d9872aca7a7637dab6c3",
          "message": "fix(l2): revert to rust 1.87 (#3506)\n\n**Motivation**\n\nThe SP1 and RISC0 workflows are broken because they don't yet support\nRust 1.88.\n\n**Description**\n\nReverts the Rust version to 1.87.\n\n\nCloses None",
          "timestamp": "2025-07-07T16:02:04Z",
          "tree_id": "8ec748de442339ffc840eb543717650bdc405e65",
          "url": "https://github.com/lambdaclass/ethrex/commit/b08fa01cfcefc97428d5d9872aca7a7637dab6c3"
        },
        "date": 1751908116786,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012544473684210527,
            "unit": "Mgas/s"
          }
        ]
      }
    ]
  }
}