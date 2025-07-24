window.BENCHMARK_DATA = {
  "lastUpdate": 1753382865387,
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
          "id": "c63bbd7db56b60c495b03a675261db440d1ad7a2",
          "message": "feat(l1): archive sync (#3161)\n\n**Motivation**\nDownload the full state of a given block from an archive node. This will\nenable us to do full sync on mainnet starting from a post-merge block\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3115",
          "timestamp": "2025-07-07T16:37:29Z",
          "tree_id": "6e5c1ac8be1f20fd8a6f87389ab2d52287ed7e2f",
          "url": "https://github.com/lambdaclass/ethrex/commit/c63bbd7db56b60c495b03a675261db440d1ad7a2"
        },
        "date": 1751909976761,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209357020765,
            "range": "± 270475993",
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
          "id": "0637f3734e69a5c0fcdf1d972f7cebc0e55c04d5",
          "message": "ci(l2): make pr-main_l2_prover a required workflow (#3517)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-07T19:38:19Z",
          "tree_id": "2c235cd71be64af005ff264c24fdb0e2066757ff",
          "url": "https://github.com/lambdaclass/ethrex/commit/0637f3734e69a5c0fcdf1d972f7cebc0e55c04d5"
        },
        "date": 1751920825614,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211005059942,
            "range": "± 1189327693",
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
          "id": "ed8e61f04e5bed2f3b496da710b6c4524f1b661d",
          "message": "fix(l1): metrics exporter dashboard total peer count panel (#3470)\n\n**Motivation**\n\nEthereum Metrics Exporter dashboard is showing no peers when running a\nsync, as described in the issue #3104.\n\n**Description**\n\nThis pr fixes the rpc call handler for net_peerCount as described\n[here](https://ethereum.org/en/developers/docs/apis/json-rpc/#net_peercount).\n\nIt also introduces a new function for `PeerHandler` to access the\nconnected peers so the rpc call handler can get the amount.\n\nHere you can see how the panel looks like now:\n<img width=\"1425\" alt=\"Screenshot 2025-07-03 at 13 29 57\"\nsrc=\"https://github.com/user-attachments/assets/89c699a8-72bb-4a42-918a-c9e3ea6d3036\"\n/>\n\nTo run this you can go to tooling/sync and run make\nstart_hoodi_metrics_docker, then go to http://localhost:3001/ to see the\npanels.\n\nCloses #3468\n\n---------\n\nCo-authored-by: fmoletta <99273364+fmoletta@users.noreply.github.com>",
          "timestamp": "2025-07-07T19:39:22Z",
          "tree_id": "2664be10e47fd8ab522ad28be25f8e3c412498a1",
          "url": "https://github.com/lambdaclass/ethrex/commit/ed8e61f04e5bed2f3b496da710b6c4524f1b661d"
        },
        "date": 1751920880367,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 213072174755,
            "range": "± 995884438",
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
          "id": "e141e1004a011bffd5d2f754c8d64c9efd770c8d",
          "message": "chore(l2): add ERC20 failed deposit integration test (#3547)\n\n**Motivation**\n\nWe want to ensure if a deposit fails, the funds won't be lost.\n\n**Description**\n\nAdds an integration test for ERC20 failed deposit turning into a\nwithdrawal.\n\nCloses #3990",
          "timestamp": "2025-07-08T15:28:54Z",
          "tree_id": "268aed2a136e9b2adf6d415b9a552fbcc01491bd",
          "url": "https://github.com/lambdaclass/ethrex/commit/e141e1004a011bffd5d2f754c8d64c9efd770c8d"
        },
        "date": 1751992172463,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209863930802,
            "range": "± 570359218",
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
          "id": "752c20b5552cceab1ed2959488929639a96a8661",
          "message": "fix(l1,l2): eth client send blobs when calling eth_estimateGas  (#3540)\n\n**Motivation**\n\nWhen calling eth_estimateGas to estimate the gas for the L2 commitment\nthe call was reverting because the blob was not included in the call\n\n**Description**\n\n- Add a function to add the blobs to a GenericTransaction\n- Add the field \"blobs\" to the request if the blobs field is not empty\n\n---------\n\nCo-authored-by: Tomás Paradelo <112426153+tomip01@users.noreply.github.com>",
          "timestamp": "2025-07-08T16:32:38Z",
          "tree_id": "ca06e38d7ee9df3793f6335ef93231ecdfdd30c3",
          "url": "https://github.com/lambdaclass/ethrex/commit/752c20b5552cceab1ed2959488929639a96a8661"
        },
        "date": 1751995996512,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209527821851,
            "range": "± 527992986",
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
          "id": "d1ceb86cb32f8949ecd3fc279084f6921c3e757f",
          "message": "fix(l1): ignore unknown protocols in capability exchange (#3543)\n\n**Motivation**\n\nFailing due to a peer having extra capabilities can make us lose\nexceptional peers. Hence, we want to ignore any extra capabilities they\nhave.\n\n**Description**\n\nThis PR changes `Capability.protocol` to be an 8-byte array instead of a\nstring, allowing us to store any string we receive.",
          "timestamp": "2025-07-08T17:12:21Z",
          "tree_id": "b73d081e492be3f36630c76381bba95066e5b585",
          "url": "https://github.com/lambdaclass/ethrex/commit/d1ceb86cb32f8949ecd3fc279084f6921c3e757f"
        },
        "date": 1751998569223,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209724867206,
            "range": "± 374609234",
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
          "id": "715c2bbe2c6d139bb938ea87c6aa1a07ade060d6",
          "message": "refactor(levm): change returned error types to `InternalError` (#3322)\n\n**Motivation**\nFrom [#3063](https://github.com/lambdaclass/ethrex/issues/3063)\n\n> There are various cases in which we return an error with the\nExceptionalHalt type but they actually are InternalErrors, things that\nshouldn't ever happen and if they happen they should break.\nThis is not a critical issue since if the VM is working fine then it\nwon't ever enter to those cases, but it would be more precise if we\ncatalogued those errors as internals instead of saying that they revert\nexecution when they don't.\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\nIntroduces the following changes:\n* Replaces `PrecompileError` with `InternalError` in those cases in\nwhich an error is returned even though is not possible for the\ninstruction to fail, typically when slicing bytes whose size have been\nalready checked.\n* Removes the error types `EvaluationError` and `DefaultError` (which\nwere quite generic) from `PrecompileError` and adds specific and more\ndescriptive error types instead (`InvalidPoint`, `PointNotInTheCurve`,\netc).\n* Removes the `PrecompileError::GasConsumedOverflow` error type.\n\n\n\nCloses #3063\n\n---------\n\nCo-authored-by: Jeremías Salomón <48994069+JereSalo@users.noreply.github.com>\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-07-08T17:26:31Z",
          "tree_id": "6b8f7bd82899863be607fc39c39df00c6ebac941",
          "url": "https://github.com/lambdaclass/ethrex/commit/715c2bbe2c6d139bb938ea87c6aa1a07ade060d6"
        },
        "date": 1751999374747,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209830786007,
            "range": "± 763501282",
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
          "id": "53546f4e280e333ad80df31355bd1fc887991d10",
          "message": "fix(l1): abort `show_state_sync_progress` task  (#3406)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nThe `show_state_sync_progress` task used to run until all\n`state_sync_segment` tasks had signaled their conclusion via\n`end_segment` method. This could cause the task to hand indeterminately\nif one of the tasks failed. This PR aims to fix this by removing the\nresponsibility of signaling their end from `state_sync_segment` and\ninstead have `state_sync` method (the one that launched both\n`show_state_sync_progress` & the `state_sync_segment` tasks) be the one\nto end the `show_state_sync_progress` task via an abort\n**Description**\n* Remove method `StateSyncProgress::end_segment` & associated field\n* `show_state_sync_progress` is now an endless task\n* `state_sync` now aborts `show_state_sync_progress` when no longer\nneeded instead of waiting for it to finish\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-07-08T18:11:03Z",
          "tree_id": "db56a77d681fdf0503b3de056d2867e86bc95061",
          "url": "https://github.com/lambdaclass/ethrex/commit/53546f4e280e333ad80df31355bd1fc887991d10"
        },
        "date": 1752001949707,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211284593952,
            "range": "± 1287981424",
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
          "id": "c15c01ae92a2f736614192f65d1884539d9e6ed5",
          "message": "ci(l1,l2): remove `core` scope and improve PR labeling workflow (#3561)\n\n**Motivation**\n\n- Declutter `ethrex_l1` project and remove ambiguous `core` scope in\ntitle.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- Remove core scope because it is ambiguous. Replace it with `l1,l2`\n- Merge `pr_author.yaml` and `pr_label.yaml` into one file\n`pr_github_metadata.yaml`\n- Change rules of labeling because of preferences in our projects:\n- `ethrex_performance`: Will have PRs that have `perf` at the beginning\nof the title.\n  - `ethrex_l2`: Will have any PR that has in the title scope `l2`\n- `ethrex_l1`: Will have PRs that haven't been assigned to\n`ethrex_performance` or `ethrex_l2` that have `l1` or `levm` in their\ntitle.\n\nThe decisions were made according to the preferences of each team.\n`ethrex_l2` project will have anything that has to do with the L2\n`ethrex_l1` project will have things that touch only l1 stuff and\nnothing else so that we assure they truly belong to this project. Some\nPRs will be filtered out and will have to be added manually, but we\nprefer that rather over the clutter of having more PRs than necessary.\n\nCloses #3565",
          "timestamp": "2025-07-08T18:18:32Z",
          "tree_id": "40bae128432e6ab09b6688b548bf5e4f8870f8aa",
          "url": "https://github.com/lambdaclass/ethrex/commit/c15c01ae92a2f736614192f65d1884539d9e6ed5"
        },
        "date": 1752002363167,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207854312627,
            "range": "± 297757431",
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
          "id": "5a14d806d0c84aef0266de503cbd451cab599d8b",
          "message": "feat(l2): add L1From field to privileged transaction events (#3477)\n\n**Motivation**\n\nAs described on #3452, it is convenient for client applications to be\nable to search their sent privileged transactions.\n\n**Description**\n\nThis PR drops indexing from all PrivilegedTxSent fields and adds an\nindexed L1From member.\n \nCloses #3452\n\n---------\n\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-07-08T19:04:41Z",
          "tree_id": "54d832d221764ef90884589a6bd5db81bd0fed13",
          "url": "https://github.com/lambdaclass/ethrex/commit/5a14d806d0c84aef0266de503cbd451cab599d8b"
        },
        "date": 1752005173013,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209060859420,
            "range": "± 509584808",
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
          "id": "f07af68346980f2762f0a71cf0de7ba87c49642b",
          "message": "fix(l2): use github token to avoid rate limit (#3570)\n\n**Motivation**\n\nOur CI is failing at the `Install solc` step in almost all jobs due to a\n`rate limit` error.\n\n**Description**\n\nAuthenticates using a GitHub token to bypass the rate limit.\n\nCloses None",
          "timestamp": "2025-07-08T21:31:57Z",
          "tree_id": "9d743d53d18ef3e1cedc95540f53d0513e1d1176",
          "url": "https://github.com/lambdaclass/ethrex/commit/f07af68346980f2762f0a71cf0de7ba87c49642b"
        },
        "date": 1752013948690,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209254002792,
            "range": "± 621219268",
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
          "id": "8aa44c11650df469a2a89d215c9791da67403a4b",
          "message": "ci(l1): comment flaky devp2p test BasicFindnode (#3542)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nThis test fails very occasionally, here are a few runs in which it\nhappened:\n-\nhttps://github.com/lambdaclass/ethrex/actions/runs/16125250426/job/45501078767\n-\nhttps://github.com/lambdaclass/ethrex/actions/runs/16126040468/job/45503603345\n-\nhttps://github.com/lambdaclass/ethrex/actions/runs/16120603086/job/45485976155\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3549",
          "timestamp": "2025-07-08T23:18:40Z",
          "tree_id": "61b413b67618e34b836f1dc72f2729db6fd4c0da",
          "url": "https://github.com/lambdaclass/ethrex/commit/8aa44c11650df469a2a89d215c9791da67403a4b"
        },
        "date": 1752020473541,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209372926820,
            "range": "± 593704898",
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
          "id": "09dd2a27634849d96d500da4042781d1d4596a12",
          "message": "fix(l1): metrics exporter sync status, percent, distance and rate panels (#3456)\n\n**Motivation**\n\nEthereum Metrics Exporter is showing incorrect data for the sync status,\nsync percent, sync distance and sync rate panels when running a sync, as\ndescribed in the issue #3104.\n\n**Description**\n\nThis pr fixes the rpc call handler for eth_syncing as described\n[here](https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_syncing).\n\nHere you can see how the panels look like up to now:\n<img width=\"1429\" alt=\"Screenshot 2025-07-03 at 11 25 57\"\nsrc=\"https://github.com/user-attachments/assets/22646c5d-1ab8-4687-be66-56d2d8eb3fc3\"\n/>\n\nTo run this you can go to tooling/sync and run `make\nstart_hoodi_metrics_docker`, then go to http://localhost:3001/ to see\nthe panels.\n\nCloses #3325 and closes #3455",
          "timestamp": "2025-07-09T11:19:46Z",
          "tree_id": "4ca1ad3f11fa2a52a207afc9fa3c31e230ff2891",
          "url": "https://github.com/lambdaclass/ethrex/commit/09dd2a27634849d96d500da4042781d1d4596a12"
        },
        "date": 1752063722636,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 212124020997,
            "range": "± 579592183",
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
          "id": "d454a1b2940492bb4d43e1643f2ec8c97f276e46",
          "message": "perf(levm): improve sstore (#3555)\n\n**Motivation**\n\nLocally the sstore bench from\nhttps://github.com/lambdaclass/ethrex/pull/3552 goes from 2x worse to a\nbit better than revm\n\nGas benchmarks improve 2x\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-09T16:25:59Z",
          "tree_id": "36ea0d7b6740d61c8fd225e9f8c4abb054ad1e83",
          "url": "https://github.com/lambdaclass/ethrex/commit/d454a1b2940492bb4d43e1643f2ec8c97f276e46"
        },
        "date": 1752081971943,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208948964115,
            "range": "± 409252867",
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
          "id": "398a10878145cbb6e1657e2360dc24a0518fbee6",
          "message": "ci(l2): use correct toolchain in nix build (#3507)\n\n**Motivation**\n\nCurrently the rust version is the one in nixpkgs, which might not follow\nour upgrades.\n\n**Description**\n\nChange the build to rely on the toolchain file on the project root.\n\n---------\n\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>\nCo-authored-by: avilagaston9 <gaston.avila@lambdaclass.com>",
          "timestamp": "2025-07-10T13:50:44Z",
          "tree_id": "f5011011b112a406ce0326b0800a05603db9ca48",
          "url": "https://github.com/lambdaclass/ethrex/commit/398a10878145cbb6e1657e2360dc24a0518fbee6"
        },
        "date": 1752159241680,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211874830502,
            "range": "± 722590930",
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
          "id": "d396ca4b52b5ea3c69fd62a1887ada672c6930ef",
          "message": "fix(l2): avoid proving already proved batch (#3588)\n\n**Motivation**\nAvoid this situation:\n- Prover finishes proving batch n\n- Prover asks for batch to prove gets batch n again because:\n`let batch_to_verify = 1 + get_latest_sent_batch()` is still n because\nthe proof_sender dind't send the verification tx yet.\n- Verifier verifies batch n + 1\n- Prover is still proving batch n when it could start proving batch n +\n1\n\n\n**Description**\n\n- Before sending a new batch to prove check if we already have all\nneeded proofs for that batch stored in the DB in case we do send and\nempty response\n\nCloses #3545",
          "timestamp": "2025-07-10T15:51:44Z",
          "tree_id": "f17356b28dc850006c1b9694ba271d6e1128893c",
          "url": "https://github.com/lambdaclass/ethrex/commit/d396ca4b52b5ea3c69fd62a1887ada672c6930ef"
        },
        "date": 1752166385018,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210975651773,
            "range": "± 391100967",
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
          "id": "8dac7cb1d7d71ccff299f6c9888444bc56846fdd",
          "message": "fix(l2): seal a batch in a single DB transaction  (#3554)\n\n**Motivation**\n\nWhen deploying ethrex L2 some errors came up that are related to the\nseal_batch process not being done in a single DB transaction.\n\n**Description**\n\n- Move seal_batch to the `StoreEngineRollup` trait\n- For sql rollup store engine\n- Wrap all the DB write functions from the trait with a <name>_in_tx\nthat gets as an input an Option<Transaction> in case the transaction is\nSome then it uses the existing transaction, and does not commit. If its\nNone it creates a new transaction and commits at the end of the\nfunction.\n- Modify the `SQLStore` struct to hold two instances of `Connection` one\nfor reads and one for writes, the write connection is protected by a\nMutex to enforce a maximum of 1 to prevent this error:\n      ```\nfailed because of a rollup store error: Limbo Query error: SQLite\nfailure: `cannot start a transaction within a transaction`\n      ``` \n- Use `PRAGMA journal_mode=WAL` for [better\nconcurrency](https://sqlite.org/wal.html#concurrency)\n- For `libmdbx` , `redb` and `in-memory`\n   - Implement the `seal_batch` function \n- Refactor: remove all the functions that were exposed by `store.rs` and\nwere only part of seal_batch to prevent its usage outside of batch\nsealing.\n\n\nCloses #3546",
          "timestamp": "2025-07-10T16:09:07Z",
          "tree_id": "b7b1d653a7447a46b3c6a30eae3762bc6c4962d7",
          "url": "https://github.com/lambdaclass/ethrex/commit/8dac7cb1d7d71ccff299f6c9888444bc56846fdd"
        },
        "date": 1752167413510,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207995563180,
            "range": "± 397252186",
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
          "id": "dcb3c9cf5cc3072eddc35f1f2640d1a66baad894",
          "message": "perf(levm): improve blake2f  (#3503)\n\n**Motivation**\n\nCleaner code and better perfomance\n\n<!-- Why does this pull request exist? What are its goals? -->\n\nMain\n\n![image](https://github.com/user-attachments/assets/1112c9dc-7257-4c7f-a8ae-b26cc1190894)\n\npr\n\n![image](https://github.com/user-attachments/assets/7cbdbe56-98d6-41ce-bc6a-11ad18a31208)\n\n\nImproves blake2f 1 round mgas\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-10T16:31:46Z",
          "tree_id": "e34318e84d26a13bd37d346390e93cc12cae7640",
          "url": "https://github.com/lambdaclass/ethrex/commit/dcb3c9cf5cc3072eddc35f1f2640d1a66baad894"
        },
        "date": 1752168942196,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 219547089813,
            "range": "± 1040528118",
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
          "id": "187e8c27f9b9a22948cd82b0b3f79866c16ac489",
          "message": "chore(l2): add forced withdrawal integration test (#3541)\n\n**Motivation**\n\nWe want an integration test for forced withdrawals\n\n**Description**\n\nWithdraws through a privileged transaction.\n\nCloses #3394",
          "timestamp": "2025-07-10T18:10:57Z",
          "tree_id": "486f18735fda83de70f48ba4f654780e8515f3d9",
          "url": "https://github.com/lambdaclass/ethrex/commit/187e8c27f9b9a22948cd82b0b3f79866c16ac489"
        },
        "date": 1752174672542,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207462387985,
            "range": "± 483507681",
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
          "id": "9dab7c08cb8dbc86a5ae90d38faf2fc2d2c98064",
          "message": "feat(l2): monitor for ethrex L2 (#3410)\n\n**Description**\n\nThis PR introduces de ethrex monitor. A currently optional tool for node\noperators to monitor the L2 state.\n\nThe node can be monitored in two different tabs, the Overview tab and\nthe Logs tab. Both tabs have a help text line at the bottom to let know\nthe user how to interact with the current tab.\n\nThe Overview tab is composed of:\n- An ASCII ethrex logo.\n- A node status widget\n- A general chain status widget, which lists:\n    - Current batch (the batch being built by the Sequencer).\n    - Current block (the block being built by the Sequencer).\n    - Last committed batch.\n    - Last committed block.\n    - Last verified batch.\n    - Last verified block.\n- An L2 batches widget, which lists the last 50 L2 batches and their\ncurrent status, highlighting:\n    - L2 batch number.\n    - Number of blocks in the batch.\n    - Number of L2 to L1 messages in the batch.\n    - Commit tx hash (if committed).\n    - Verify tx hash (if verified).\n- An L2 blocks widget, which lists the last 50 L2 blocks, highlighting:\n    - L2 block number.\n    - Number of txs in the block.\n    - L2 block hash.\n    - L2 block coinbase (probably more relevant in based rollups).\n    - Gas consumed.\n    - Blob gas consumed.\n    - Size of the block. \n- A mempool widget, which lists the current 50 txs in the memool,\nhighlighting:\n    - Tx type (e.g. EIP1559, Privilege, etc).\n    - Tx hash.\n    - Tx sender.\n    - Tx nonce.\n- An L1 to L2 messages widget, which lists the last 50 L1 to L2 msgs and\ntheir status, highlighting:\n    - Message kind (e.g. deposit, message, etc).\n    - Message status (e.g. Processed on L2, etc).\n    - Message L1 tx hash.\n    - Message L2 tx hash\n    - Value\n- An L2 to L1 messages widget, which lists the last 50 L2 to L1 msgs and\ntheir status, highlighting:\n    - Message kind (e.g. withdrawal, message, etc).\n    - Message status (e.g. initiated, claimed, sent, delivered).\n    - Receiver on L1.\n    - Token L1 (if ERC20 withdrawal).\n    - Token L2 (if ERC20 withdrawal).\n    - L2 tx hash\n    - Value\n\nThe Logs tab shows the logs altogether or by crate. The log level could\nalso be adjusted in runtime.\n\n> [!NOTE]\n> 1. This feature is introduced as optional for now given its initial\nstate. Once mature enough, it will be default for operators.\n> 2. This initial version has some minor known flaws, but they were\nskipped in this PR on purpose:\n>     - #3512 .\n>     - #3513.\n>     - #3514.\n>     - #3515.\n>     - #3516.\n>     - No optimizations were done.\n\n**How to test**\n\n1. Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n2. Run a Sequencer (I suggest `make restart` in `crates/l2`).\n3. Run the prover with `make init-prover` in `crates/l2`.\n4. Run `make test` in `crates/l2`.\n\n**Showcase**\n\n*Overview*\n\n<img width=\"1512\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/0431b1f3-1a8f-49cf-9519-413ea3d3ed1a\"\n/>\n\n*Logs*\n\n<img width=\"1512\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/e0e6cdd7-1f8d-4278-8619-475cfaa14d4b\"\n/>",
          "timestamp": "2025-07-10T18:51:42Z",
          "tree_id": "e9c5ec2c406ad35b66a6b0943014497ccfe76e3b",
          "url": "https://github.com/lambdaclass/ethrex/commit/9dab7c08cb8dbc86a5ae90d38faf2fc2d2c98064"
        },
        "date": 1752177208715,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208169108689,
            "range": "± 933679142",
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
          "id": "f466fb8216f85442d763a8ed6a10a36f05e8c93f",
          "message": "feat(l2): proxied l2 system contracts (#3421)\n\n**Motivation**\n\nWe want to be able to upgrade L2 system contracts.\n\n**Description**\n\nThis makes it so that the L2 contracts themselves are proxies. Their\ninitial implementations are kept in the genesis for ease of deployment\nand to avoid keeping them empty in the first blocks.\n\nSince the proxies need to be embedded in the genesis, they can't be\ndeployed with a constructor, so their\n[ERC-1967](https://eips.ethereum.org/EIPS/eip-1967) slots are set\ndirectly.\n\nA function is added to the L1 CommonBridge to allow upgrading the L2\ncontracts. A special address (0xf000) is used to authenticate the\nupgrade.\n\nCloses #3345",
          "timestamp": "2025-07-11T12:27:27Z",
          "tree_id": "5da2a6cdd7e6ca4748dd0f07fac5768e8cfe3540",
          "url": "https://github.com/lambdaclass/ethrex/commit/f466fb8216f85442d763a8ed6a10a36f05e8c93f"
        },
        "date": 1752240560916,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207065743006,
            "range": "± 451079847",
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
          "id": "fa4246853cf52292fce47ca6cb405e538adee68c",
          "message": "ci(l1, l2): create reusable step to install rust (#3591)\n\nContinuation of https://github.com/lambdaclass/ethrex/pull/3318\n\n**Motivation**\n\nGitHub Variables are excluded from workflow runs triggered by PRs from\nforks, so we need to remove this variable dependency in order for\nexternal collaborators to send PRs and run the CI properly\n\n**Description**\n\n* The `Extract Rust version from rust-toolchain.toml` step (`id:\nrustver`) uses `grep` and `sed` to extract the rust version from the\n`rust-toolchain.toml` file that is in the root of the repository.\n* The `Install Rust` step utilizes the output of the previous step to\nsend the version to the `toolchain` parameter\n* Note that in some cases, I had to move the `Checkout` step further up\n(it's also good practice to put it as high up as possible) so the\n`rust-toolchain.toml` file is available to be read.\n\n---------\n\nCo-authored-by: Klaus Lungwitz <klaus.lungwitz@lambdaclass.com>\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>",
          "timestamp": "2025-07-11T13:02:23Z",
          "tree_id": "cc5d86727f7ab3a3c8d9b086fd654abe55abcbad",
          "url": "https://github.com/lambdaclass/ethrex/commit/fa4246853cf52292fce47ca6cb405e538adee68c"
        },
        "date": 1752242720516,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211607389389,
            "range": "± 491996718",
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
          "id": "3ce704c7b243aadc1a332c50d92bb5699d5c80e0",
          "message": "chore(levm): upgrade levm vs revm bench  (#3557)\n\n**Motivation**\n\n- Ci generated comment is not updating.\n- Using a porcentual difference higher than 10% for showing results is\nto high.\n\n**Description**\n\nThis pr tackles two minor issues. \n\n- It modifies the bash script that puts the comment together, so the\ncomment can be found and updated by the ci.\n- It also changes the margin of error for which the benchmarks should be\nshown. Now the benchmarks are shown for a porcentual difference higher\nthan 5%, otherwise you can check all the benchmarks on the Detailed\nResults drop-down tab. This change is introduced with the intention of\nsolving issue #3462\n\n---------\n\nCo-authored-by: Edgar Luque <git@edgl.dev>",
          "timestamp": "2025-07-11T13:54:46Z",
          "tree_id": "980ba59cfc5db764c33abdc4e5cd13bc2c7a9ccc",
          "url": "https://github.com/lambdaclass/ethrex/commit/3ce704c7b243aadc1a332c50d92bb5699d5c80e0"
        },
        "date": 1752245397857,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208177377382,
            "range": "± 575729139",
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
          "id": "05d3c1290649b1f3949d7376178be78fbb1cecbf",
          "message": "fix(levm): fix benchmark block execution ci (#3619)\n\n**Motivation**\n\nsee\nhttps://github.com/lambdaclass/ethrex/actions/runs/16266441472/job/45923106459?pr=3564\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-14T17:53:13Z",
          "tree_id": "7b8fbc2f30df44acf9fc51a9312de9411c4b9c87",
          "url": "https://github.com/lambdaclass/ethrex/commit/05d3c1290649b1f3949d7376178be78fbb1cecbf"
        },
        "date": 1752519287815,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208847344258,
            "range": "± 444468116",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "sfroment42@gmail.com",
            "name": "Sacha Froment",
            "username": "sfroment"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "053237663e3be3dd9eb02dbacba88d6e0ce54610",
          "message": "feat(l1): add From for Transaction -> GenericTransaction (#3227)\n\n**Motivation**\n\nAdding an easy way to get a GenericTransaction from any Transaction\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nAdding the 2 missing From and one for the enum\nThis will allow people who use the ethClient to make estimate_gas and\neth_call request, more easily and maybe other request in the future\nmight benefit from it\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\n\nBTW I don't know which scope I shall use\n\nSigned-off-by: Sacha Froment <sfroment42@gmail.com>\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-07-14T18:08:45Z",
          "tree_id": "b0c6b8443312ff2002a0844abe8e0d7579e19ce8",
          "url": "https://github.com/lambdaclass/ethrex/commit/053237663e3be3dd9eb02dbacba88d6e0ce54610"
        },
        "date": 1752520320572,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208952753552,
            "range": "± 489334814",
            "unit": "ns/iter"
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
          "distinct": false,
          "id": "318d68b1ad651c4df08ba03a8b65b27fe50adbff",
          "message": "fix(l1, l2): logs not appearing on subcommands (#3631)\n\n**Motivation**\n\nQuick bug fix that makes logs not appear\n\n**Description**\n\nThe function ```init_tracing(&opts)``` was being called after any\nsubcommands (import, export, etc) were read, causing these (specially\nthe import) not to output logs. This PR fixes that.",
          "timestamp": "2025-07-14T19:43:28Z",
          "tree_id": "21db8e93a6ae21ed8dea0b94c61966566a2010d4",
          "url": "https://github.com/lambdaclass/ethrex/commit/318d68b1ad651c4df08ba03a8b65b27fe50adbff"
        },
        "date": 1752525535836,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209780533571,
            "range": "± 422501665",
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
          "id": "7e97d4a42213231038801327a5485b720f3dcbde",
          "message": "docs(l1): add documentation on ethereum metrics exporter use (#3538)\n\n**Motivation**\n\nWe don't have proper documentation on running the metrics introduced for\nL1 in #3061\n\n**Description**\n\nThis pr includes a quick start on how to use the new targets to display\nmetrics for running a sync on holesky or hoodi, and a more detailed\ndescription in case you want to display metrics when syncing on another\nnetwork.\n\nCloses #3207",
          "timestamp": "2025-07-14T19:58:28Z",
          "tree_id": "302f57a1d2cecd1d75639aa68bc81c9f627bc936",
          "url": "https://github.com/lambdaclass/ethrex/commit/7e97d4a42213231038801327a5485b720f3dcbde"
        },
        "date": 1752526464608,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 213163132488,
            "range": "± 612003432",
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
          "id": "d874b90c05456847c4b0d50657916434b4600840",
          "message": "fix(levm): ignore DB storage values for destroyed accounts (#3617)\n\n**Motivation**\nWhen executing blocks in batches an account may be destroyed and created\nagain within the same batch. This can lead to errors as we might try to\nload a storage value from the DB (such as in an `SLOAD`) that doesn't\nexist in the newly created account but that used to be part of the now\ndestroyed account, leading to the incorrect value being loaded.\nThis was detected on sepolia testnet block range 3302786-3302799 where a\nan account was destructed via `SELFDESTRUCT` and then created 6 blocks\nlater via `CREATE`. The same transaction that created it then performed\nan `SSTORE` which was charged the default fee (100 gas) as the stored\nkey and value matched the ones in the previously destroyed storage\ninstead of charging the storage creation fee (2000 gas). The value was\npreviously fetched from the DB by an `SLOAD` operation.\nThis PR solves this issue by first checking if the account was destroyed\nbefore looking up a storage value in the DB (The `Store`). If an account\nwas destroyed then whatever was stored in the DB is no longer valid, so\nwe return the default value (as we would do if the key doesn't exist)\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* (`levm` crate)`GeneralizedDatabase::get_value_from_database`: check if\nthe account was destroyed before querying the DB. If the account was\ndestroyed return default value\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-14T21:39:10Z",
          "tree_id": "23f2aaec44dced688b3ec27ba5b502a6f41983e4",
          "url": "https://github.com/lambdaclass/ethrex/commit/d874b90c05456847c4b0d50657916434b4600840"
        },
        "date": 1752532453721,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210167237419,
            "range": "± 1043679222",
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
          "id": "6de7196718fcf89781c20a190872011cabc85c99",
          "message": "fix(l2): panic because of double init tracing (#3637)\n\n**Motivation**\n\nInit L2 was panicking because of a double call to init_tracing\n\n**Description**\n\n- Move back the init tracing call to after the subcommand execution\n- Inside the subcommands call init_tracing only if the subcommand is not\n`Subcommand::L2`",
          "timestamp": "2025-07-15T13:09:19Z",
          "tree_id": "367eb56892cd70c2b727e9330f073a618c389e94",
          "url": "https://github.com/lambdaclass/ethrex/commit/6de7196718fcf89781c20a190872011cabc85c99"
        },
        "date": 1752588277813,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209776915502,
            "range": "± 1259799176",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "906de695154909601de4c10a883cc822509dc270",
          "message": "feat(l2): monitor add delay to scroll (#3616)\n\n**Motivation**\nMonitor scroll goes too fast\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nAdded a delay for the log scroll\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n**How to Test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n- Press Tab to change the Tab\n- Scroll Up and Down to test the scroll\n\nCloses\nhttps://github.com/orgs/lambdaclass/projects/37/views/10?pane=issue&itemId=118809801&issue=lambdaclass%7Cethrex%7C3514",
          "timestamp": "2025-07-15T14:01:38Z",
          "tree_id": "ad406a83542279b38ac48a3d0e98b93574f00c0d",
          "url": "https://github.com/lambdaclass/ethrex/commit/906de695154909601de4c10a883cc822509dc270"
        },
        "date": 1752591481304,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210018829230,
            "range": "± 425972831",
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
          "id": "b0a5da487e8a2ffc4f174a3d5629bdb1e581e7a0",
          "message": "ci(l1): try running hive tests in CI with levm (#3566)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- Run most recent hive tests in CI with LEVM.\n- I had to comment out 2 of them because they don't pass, it was\nexpected since we were running tests that were 6 months old so things\nhave changed.\n\n---------\n\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-07-15T14:25:58Z",
          "tree_id": "aa7582b6c137ea6e00b405c391832b9f826d9898",
          "url": "https://github.com/lambdaclass/ethrex/commit/b0a5da487e8a2ffc4f174a3d5629bdb1e581e7a0"
        },
        "date": 1752592900415,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209321850418,
            "range": "± 1199286237",
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
          "id": "f8a6168341db73d3a593b94e0e0f0a50c1044168",
          "message": "feat(l1): peer scoring for snap requests (#3334)\n\n**Motivation**\nIntegrate and adapt the peer scoring introduced by #2115 for snap\nrequests.\nFor eth requests, we consider failure to return requested data as a peer\nfailure, but with snap the data we request is not guaranteed to be\navailable (as it might have become stale during the sync cycle) so we\ncannot asume that an empty response is a bad response that should be\npenalized. For snap requests this PR collects the ids of the peers we\nattempted to request data from, and once we get a successful peer\nresponse we confirm that the data was indeed available and reward the\nresponsive peer while penalizing the previous unresponsive peers\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Collect ids of peers on each snap request retry and penalize and\nreward peers accordingly upon a successful peer response\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3118",
          "timestamp": "2025-07-15T14:29:34Z",
          "tree_id": "98d4bd1b3523d36f75886638eca8394cb47f9400",
          "url": "https://github.com/lambdaclass/ethrex/commit/f8a6168341db73d3a593b94e0e0f0a50c1044168"
        },
        "date": 1752593057527,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208924546190,
            "range": "± 467483655",
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
          "id": "fd98ef02d3634246651f8879e9d70feb1dd0653a",
          "message": "fix(l2): install solc in missing workflows (#3649)\n\n**Motivation**\n\nIn #3443, we missed installing solc in some workflows.\n\nCloses None",
          "timestamp": "2025-07-15T21:20:05Z",
          "tree_id": "66735758ea212d38ae32deee8ccf38901cad506a",
          "url": "https://github.com/lambdaclass/ethrex/commit/fd98ef02d3634246651f8879e9d70feb1dd0653a"
        },
        "date": 1752618146539,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210593029392,
            "range": "± 416815067",
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
          "id": "5c7a30485164c7db8ed43304a4577a0d0451cc54",
          "message": "feat(l2): add support for web3signer (#2714)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nMany operators will want to use a remote signer instead of having the\nprivate keys on the same server as the sequencer.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nReplace all uses of a private key with a new `Signer` enum. This signer\ncan be either `Local` or `Remote` and can be lately extended. This aims\nto standardise the way all kind of messages are signed across the L2 and\nfacilitate the setup via flags or environment\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\n---------\n\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: fedacking <francisco.gauna@lambdaclass.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>",
          "timestamp": "2025-07-15T23:34:22Z",
          "tree_id": "166bed55b2d252034634dd4fb89fe704a900bb8e",
          "url": "https://github.com/lambdaclass/ethrex/commit/5c7a30485164c7db8ed43304a4577a0d0451cc54"
        },
        "date": 1752625953942,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210342433098,
            "range": "± 948167720",
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
          "id": "ea331e09542d0ffd819d81af32d7a192a3b80f6a",
          "message": "perf(levm): add sstore bench, allow unoptimized bench contracts and improve bench makefile (#3552)\n\n**Motivation**\n\n- Adds a sstore benchmark, however we need to disable solc optimizations\nfor this contract otherwise it removes most code.\n- Improved the makefile adding a command to samply an individual\nbenchmark\n\nhttps://share.firefox.dev/44MVD2V",
          "timestamp": "2025-07-16T06:07:39Z",
          "tree_id": "05d165ee245374bc2320e881bc6c28a6c30b1895",
          "url": "https://github.com/lambdaclass/ethrex/commit/ea331e09542d0ffd819d81af32d7a192a3b80f6a"
        },
        "date": 1752649451048,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 214578696934,
            "range": "± 514225987",
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
          "id": "8a568cabc9875a7667dd4bf5ce881ec6f26f1e82",
          "message": "refactor(l2): remove expects in L2 monitor (#3615)\n\n**Motivation**\n\nWe want to handle errors gracefully.\n\n**Description**\n\nRemoves usage of .expect\n\n**How to test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n\nCloses #3535\n\n---------\n\nCo-authored-by: Tomás Paradelo <112426153+tomip01@users.noreply.github.com>",
          "timestamp": "2025-07-16T15:02:36Z",
          "tree_id": "5803a4e78ee60df8c6ab4713467b393e4d4cfac4",
          "url": "https://github.com/lambdaclass/ethrex/commit/8a568cabc9875a7667dd4bf5ce881ec6f26f1e82"
        },
        "date": 1752681535326,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211114097547,
            "range": "± 403796172",
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
          "id": "faa3dec1f9358872ac18b09e9bd994a80cb1231b",
          "message": "feat(l1): decouple size of execution batch from header request size during full-sync (#3074)\n\n**Motivation**\nAllow us to configure the amount of blocks to execute in a single batch\nduring full sync. Currently, the only way to do this is by changing the\namount of block headers we ask for in each request.\nIn order to achieve this, this PR proposes adding the enum\n`BlockSyncState` with variants for Full and Snap sync so we can separate\nbehaviors between each mode and also allow each mode to keep its\nseparate state. This is key as we will need to persist headers and\nbodies through various fetch requests so we can build custom-sized\nexecution batches.\nIt also replaces the previous configurable env var `BlOCK_HEADER_LIMIT`\nwith `EXECUTE_BLOCK_BATCH`\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Add `BlockSyncState` enum as a way to differentiate between each sync\nmode's state during block syncing phase.\n* Refactor `request_and_validate_block_bodies`: it now receives a slice\nof headers and returns the requested block bodies instead of the full\nblocks. This allowed us to completely get rid of header cloning.\n* `validate_block_body` now receives a reference to the head & body\ninstead of the full block (as a result of refactoring its only user)\n* `Store::add_block_headers` now only receives the headers (This lets us\nsimplify caller code)\n* Removed `search_head` variable as having both current & search head\nserves no purpose.\n* Abtract current_head selection into `BlockSyncState::get_current_head`\n* Fix bug in condition used to decide wether to switch from snap to full\nsync\n* `start_sync` no longer receives `current_head`\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #2894\n\n---------\n\nCo-authored-by: SDartayet <44068466+SDartayet@users.noreply.github.com>\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-07-16T15:55:06Z",
          "tree_id": "32e65b17e7d3493c84eea672f20281cf2df62aaa",
          "url": "https://github.com/lambdaclass/ethrex/commit/faa3dec1f9358872ac18b09e9bd994a80cb1231b"
        },
        "date": 1752684688631,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208732062852,
            "range": "± 605900980",
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
          "id": "77b2819356aa7b7c8c380cc6e1f9a430c4d7f2bb",
          "message": "fix(l2): flamegraph and benchmark workflows (#3652)\n\n**Motivation**\n\nInstall solc to `flamegraph-reth` job. Successful run\n[here](https://github.com/lambdaclass/ethrex/actions/runs/16305540602/job/46050676065?pr=3652).\nCloses None",
          "timestamp": "2025-07-16T17:28:06Z",
          "tree_id": "7ba35bdbcc440aaceedaf28e067ee5f8b95399b0",
          "url": "https://github.com/lambdaclass/ethrex/commit/77b2819356aa7b7c8c380cc6e1f9a430c4d7f2bb"
        },
        "date": 1752690287100,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 214261390548,
            "range": "± 281392533",
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
          "id": "5839db8a7bec92df065433fed6ba6918142876fd",
          "message": "fix(l1): set base fee per gas only if we're past London hardfork (#3659)\n\n**Motivation**\n\nWe found that we are computing the mainnet genesis block hash wrong.\n\n**Description**\n\nThis PR changes our `Genesis::get_block_header` function to only set the\n`base_fee_per_gas` if we're past the London hardfork, which introduced\nthis field. I also included some tests to verify the hash of each public\nnetwork matches some hardcoded values, checked against `geth`.",
          "timestamp": "2025-07-16T17:29:18Z",
          "tree_id": "01606b85d8cd095fec44bab8c407c1161820b624",
          "url": "https://github.com/lambdaclass/ethrex/commit/5839db8a7bec92df065433fed6ba6918142876fd"
        },
        "date": 1752690328383,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211550201925,
            "range": "± 636088920",
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
          "id": "da2a576606ba61563712c4bdd5cf9ac2041292ff",
          "message": "fix(l2): use GITHUB_TOKEN for sp1up and rzup actions (#3655)\n\n**Motivation**\n\nThe SP1 toolchain fails sometimes to install with a generic \"Failed to\nfetch releases list\" error, but the cause may be an [API rate\nlimit](https://github.com/succinctlabs/sp1/issues/2320#issuecomment-2955903435)\n\nThe Risc0 toolchain has the same problem, explicitly returning an API\nrate limit error.\n\nWe bypass this by authenticating using the `GITHUB_TOKEN`",
          "timestamp": "2025-07-16T17:40:57Z",
          "tree_id": "8de80b667d3234524c46387e1ff9abfe9787da58",
          "url": "https://github.com/lambdaclass/ethrex/commit/da2a576606ba61563712c4bdd5cf9ac2041292ff"
        },
        "date": 1752691009511,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211276035301,
            "range": "± 590085126",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "tomas.arjovsky@lambdaclass.com",
            "name": "Tomás Arjovsky",
            "username": "Arkenan"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "d6548ff9a907773e67edff83ea7b687ffefdaa3d",
          "message": "feat(l1): add gas used diff in error (#3644)\n\n**Motivation**\n\nWhen there's a gas mismatch, there's no way to tell what is the mismatch\nunless we print it manually. If it's present in the error, at least it\nappears in the test logs when it happens.\n\n**Description**\n\n- Adds used and expected fields for the GasUsedMismatchError.\n- Adds the fields in the block gas post-exec validation.",
          "timestamp": "2025-07-16T17:44:51Z",
          "tree_id": "bb7f853ecbc13f3c7b7d56fe3dce4a5025f9c716",
          "url": "https://github.com/lambdaclass/ethrex/commit/d6548ff9a907773e67edff83ea7b687ffefdaa3d"
        },
        "date": 1752691245313,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 206741197974,
            "range": "± 302712237",
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
          "id": "91a41410d3020aadaba8379656bcbdfc1114d3cc",
          "message": "feat(l2): implement address aliasing (#3451)\n\n**Motivation**\n\nWe want to prevent l1 contracts from forging transactions for existing\nl2 contracts.\n\n**Description**\n\nKeeps faked source addresses. Implements checks described on #3424.\n\nCloses #3424\n\n---------\n\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>",
          "timestamp": "2025-07-16T18:04:08Z",
          "tree_id": "e3c4a08556fe2d5e20afb42ca55a16c81b098104",
          "url": "https://github.com/lambdaclass/ethrex/commit/91a41410d3020aadaba8379656bcbdfc1114d3cc"
        },
        "date": 1752692477723,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 212352555477,
            "range": "± 220069186",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "f61eecdc5b15f3b35a14edc1ddab871c8ed64468",
          "message": "feat(l2): monitor handle index slicing (#3611)\n\n**Motivation**\nMonitor had unhandled index slicing in its code.\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nAdded new variants for `MonitorError` and used them to remove the index\nslicing in the monitor\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n**How to test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses\nhttps://github.com/orgs/lambdaclass/projects/37/views/10?pane=issue&itemId=118823704&issue=lambdaclass%7Cethrex%7C3537",
          "timestamp": "2025-07-16T18:41:49Z",
          "tree_id": "e64b43c58d56f1187206e6b98a9a253805277cfb",
          "url": "https://github.com/lambdaclass/ethrex/commit/f61eecdc5b15f3b35a14edc1ddab871c8ed64468"
        },
        "date": 1752694683339,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211248039446,
            "range": "± 255429460",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "3cf9507c3b63fe81929bb8ae2fd32de3fa049078",
          "message": "feat(l2): make monitor quit (#3622)\n\n**Motivation**\nWhen the monitor is quitted with `Shift + Q` it closes the monitor but\ndoes not end the process\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nChanged the L2 task initialization to use `JoinSet` instead of a\n`TaskTracker`, so it can be joined and end the process if it ended.\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n**How to test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n- Press `Shift + Q` to close the monitor\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses\nhttps://github.com/orgs/lambdaclass/projects/37/views/10?pane=issue&itemId=118808771&issue=lambdaclass%7Cethrex%7C3512",
          "timestamp": "2025-07-16T20:07:24Z",
          "tree_id": "b08bdd2932e3c056a571269355a21b4a1bbfb496",
          "url": "https://github.com/lambdaclass/ethrex/commit/3cf9507c3b63fe81929bb8ae2fd32de3fa049078"
        },
        "date": 1752699861277,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208531755326,
            "range": "± 598358898",
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
          "id": "c7c89a4fb4109c85f04babfd1fad805c7c40fb09",
          "message": "feat(l2): make contract compilation in the SDK optional (#3665)\n\n**Motivation**\n\n#3443, caused `solc` to be a compile-time dependency of the client.\nSince the proxy bytecode is only needed in `deploy_with_proxy`, which is\nonly used by the `deployer`, this PR makes contract compilation\noptional, via an env var.\n\n**Description**\n\n- Modifies `sdk/build.rs` to check whether `COMPILE_CONTRACTS` env var\nis set before trying to compile the proxy.\n- Creates a new error `ProxyBytecodeNotFound`, which is returned if\n`deploy_with_proxy` is called without compiling the contract.\n- Removes the installation of `solc` from workflows and Dockerfiles\nwhere it is no longer needed\n\nCloses #3654",
          "timestamp": "2025-07-16T22:08:17Z",
          "tree_id": "b795bcc727701a80c595d0e5f08cab0c95f414fc",
          "url": "https://github.com/lambdaclass/ethrex/commit/c7c89a4fb4109c85f04babfd1fad805c7c40fb09"
        },
        "date": 1752706998528,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208193047892,
            "range": "± 1208961541",
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
          "id": "d8aaed209910719f7f482fd6e3b2f33aefb1aba3",
          "message": "chore(l1, l2): add claude/gemini files to .gitignore (#3653)",
          "timestamp": "2025-07-17T11:04:04Z",
          "tree_id": "c29a6ea2921f69d1cc96417821b0fc03dd1163cb",
          "url": "https://github.com/lambdaclass/ethrex/commit/d8aaed209910719f7f482fd6e3b2f33aefb1aba3"
        },
        "date": 1752753993425,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209493098609,
            "range": "± 354976637",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "tomas.arjovsky@lambdaclass.com",
            "name": "Tomás Arjovsky",
            "username": "Arkenan"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "bc82ed12aee8e7d627a0ac52cbbd8287084b51b2",
          "message": "ci(l1): disable block builcing bench until it's fixed (#3670)\n\n**Motivation**\n\nThe benchmark doesn't work and it's blocking all prs",
          "timestamp": "2025-07-17T11:06:00Z",
          "tree_id": "f6f3fbc8ccbaec48aef841363a5d8a271f0b2e0f",
          "url": "https://github.com/lambdaclass/ethrex/commit/bc82ed12aee8e7d627a0ac52cbbd8287084b51b2"
        },
        "date": 1752754147405,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210925825367,
            "range": "± 817282704",
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
          "id": "f21fe24bf2e2c2fc62aa9be3db6e0da0f491bcc9",
          "message": "fix(l1): fix double tracer initialization in block execution benchmark (#3671)\n\n**Motivation**\n\nCurrently the block execution benchmark is\n[broken](https://github.com/lambdaclass/ethrex/actions/runs/16344656297/job/46175367153?pr=3590)\nas a result of calling `init_tracing` twice.\n\n**Description**\n\nThis happens because, when the `--removedb` flag is used, RemoveDB is\ncalled as a command, which initializes the logger again.\n\nThis PR calls removedb directly instead.",
          "timestamp": "2025-07-17T13:11:53Z",
          "tree_id": "ebdc1d6f14e1bfb5b07a3a65e9449bb2dc729dd3",
          "url": "https://github.com/lambdaclass/ethrex/commit/f21fe24bf2e2c2fc62aa9be3db6e0da0f491bcc9"
        },
        "date": 1752761331719,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211147036442,
            "range": "± 953700413",
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
          "id": "c0e0ce2933c1c72d943771abd58563355081c09f",
          "message": "ci(l1): support multiple hive versions depending on simulation. (#3661)\n\n**Motivation**\nWe want to get rid of our hive fork and use the upstream. Unfortunately,\nwe can't completely rely on it yet because it would break.\n\n**Description**\n- While we fix the upstream, lets rely on two versions of Hive, our fork\none and the upstream",
          "timestamp": "2025-07-17T13:31:45Z",
          "tree_id": "3365e4d99a9a881624b7f54dfd3e3d7e9295904a",
          "url": "https://github.com/lambdaclass/ethrex/commit/c0e0ce2933c1c72d943771abd58563355081c09f"
        },
        "date": 1752762464241,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211570268110,
            "range": "± 505787130",
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
          "id": "22b64308b7b0badb3e78279b12f8b36f69bd0642",
          "message": "perf(levm): new memory model (#3564)\n\n**Motivation**\n\nGas benchmarks show an 23% improvement on opcode based timings and 12%\non end to end.\n30% improvement in mgas for mstore (before unsafe)\n\nAfter adding unsafe we see a 30% improvement on top of the mstore\nimprovements and overall general improvements on other opcodes.",
          "timestamp": "2025-07-17T14:04:33Z",
          "tree_id": "eeb024c2f8db6140858e55a60a1250ff8fa4cd1b",
          "url": "https://github.com/lambdaclass/ethrex/commit/22b64308b7b0badb3e78279b12f8b36f69bd0642"
        },
        "date": 1752764509341,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209933147441,
            "range": "± 321652934",
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
          "id": "165b94c9a2069d2a4bb9c320f26554b0190c4e30",
          "message": "perf(levm): add AVX256 implementation of BLAKE2 (#3590)\n\n**Motivation**\n\nTo improve BLAKE2 performance.\n\n**Description**\n\nWhy AVX256 instead of AVX512? Mainly that\n[AVX512](https://github.com/rust-lang/rust/issues/111137) intrinsics are\nstill experimental.\n\nCreates a common/crypto module to house blake2. We should consider\nmoving here other cryptographic operations currently inside\nprecompiles.rs.\n\nIf avx2 is available, a permute-with-gather implementation is used.\n\nUsage of unsafe is required for SIMD loads and stores. It should be\nreviewed that alignment requirements are satisfied and that no\nout-of-bounds operations are possible.\n\nNote that aside from the obvious ones with \"load\" or \"store\" in the\nname, gather also represents a series of memory loads.\n\nUnsafe is also required to call the first avx2-enabled function, since\nwe must first ensure avx2 is actually available on the target CPU.\n\n** Benchmarks **\n\n### PR\n\n|Title|Max (MGas/s)|p50 (MGas/s)|p95 (MGas/s)|p99 (MGas/s)|Min (MGas/s)|\n\n|----|--------------|--------------|-------------|--------------|--------------|\nBlake1MRounds|120.19|93.97|93.38|99.85|91.54\nBlake1Round|226.42|175.09|170.08|166.83|166.82\nBlake1KRounds|122.36|97.28|96.09|100.90|95.87\nBlake10MRounds|174.36|110.78|104.15|124.33|103.89\n\n### Main\n\n|Title|Max (MGas/s)|p50 (MGas/s)|p95 (MGas/s)|p99 (MGas/s)|Min (MGas/s)|\n\n|----|--------------|--------------|-------------|--------------|--------------|\nBlake1MRounds|80.79|63.04|62.57|67.80|62.50\nBlake1Round|223.59|174.93|168.21|159.38|159.33\nBlake1KRounds|83.75|66.59|65.88|68.37|64.76\nBlake10MRounds|117.79|77.21|69.63|83.19|69.05",
          "timestamp": "2025-07-17T14:34:10Z",
          "tree_id": "065dfa16f0769f1776aae5132e7f7e58e22fde93",
          "url": "https://github.com/lambdaclass/ethrex/commit/165b94c9a2069d2a4bb9c320f26554b0190c4e30"
        },
        "date": 1752766209105,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208306400295,
            "range": "± 349221026",
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
          "id": "14ef9bfde463af72598fe43a515c334fea6aedfb",
          "message": "fix(l2): `get_batch` failing if in validium mode (#3680)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nRollup store's`get_batch` fails when in validium mode as it's not\nfinding any blob (currently in validium mode we don't generate blobs).\nThis makes features like `ethrex_getBatchByNumber` unusable in validium\nmode.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nAccept empty blobs bundle when retrieving batches from rollup store.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-07-17T15:25:15Z",
          "tree_id": "585d1a479356a69e449df5a5763ea36b8a040686",
          "url": "https://github.com/lambdaclass/ethrex/commit/14ef9bfde463af72598fe43a515c334fea6aedfb"
        },
        "date": 1752769218907,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207747748494,
            "range": "± 475475744",
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
          "id": "df3710b203a0214e243a157f46147d48b2d9d38a",
          "message": "ci(l1,l2): remove ethrex replay from releases  (#3663)\n\n**Motivation**\n\nWe don't want to make releases for ethrex replay\n\n**Description**\n\n- Remove matrix.binary from ci and only build ethrex and prover binaries\n- Update docs on how to run ethrex-replay\n- Successful run\n[here](https://github.com/lambdaclass/ethrex/actions/runs/16346228196/job/46180528866)",
          "timestamp": "2025-07-17T15:36:58Z",
          "tree_id": "e74c7e272ffd46bdc24eae77eb489c4cc5e1e7a2",
          "url": "https://github.com/lambdaclass/ethrex/commit/df3710b203a0214e243a157f46147d48b2d9d38a"
        },
        "date": 1752770005252,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210077193054,
            "range": "± 780667061",
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
          "distinct": true,
          "id": "229b791477f203dde019f36d5d62be182139d63a",
          "message": "refactor(l1): make ethrex-only github actions faster (#3648)\n\n**Motivation**\n\nRunning the ethrex_only github actions job seems to be slower than those\nthat use other execution clients as well\n\n**Description**\n\nThere were 2 main reasons why this job was slower compared to the others\n- The ethrex_only job includes the EOA and BLOB transactions assertoor\nplaybooks, which are the ones being run in the other two github jobs\n- The slot time of 12 sec was making the test take to long\n\nThe slot time was modified and now the tests take 10 minutes instead of\nthe original 18\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3628",
          "timestamp": "2025-07-17T16:21:11Z",
          "tree_id": "41a3b691e05e3ff267354c99a2f50f9b45e9edc7",
          "url": "https://github.com/lambdaclass/ethrex/commit/229b791477f203dde019f36d5d62be182139d63a"
        },
        "date": 1752772658466,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 206822221894,
            "range": "± 359622622",
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
          "id": "23191af7468828cb68d161143b460a6f25c96181",
          "message": "perf(levm): improve sstore perfomance  further (#3657)\n\n**Motivation**\nImproves sstore perfomance\n\nRequires #3564\n\nFrom 1100 to over 2200\n\n<img width=\"1896\" height=\"281\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/7f5697a3-048c-4554-91bb-22839bb91d95\"\n/>\n\nThe main change is going from Hashmaps to BTreeMaps.\n\nThey are more efficient for the type of storages we use, for small\ndatasets (1k~100k i would say) they overperform hashmaps due to avoiding\nentirely the hashing cost, which seemed to be the biggest factor.\n\nThis changes comes with 2 other minor changes, like a more efficient\nu256 to big endian and a change to backup_storage_slot.",
          "timestamp": "2025-07-17T17:29:21Z",
          "tree_id": "ad97fe646e7b6d407f1bfc5a7afa50ca64c47427",
          "url": "https://github.com/lambdaclass/ethrex/commit/23191af7468828cb68d161143b460a6f25c96181"
        },
        "date": 1752776709277,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211269286552,
            "range": "± 478563653",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me+git@droak.sh",
            "name": "Oak",
            "username": "d-roak"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "8272969a64958abfed0f7085cb7c4d684f2202df",
          "message": "docs(l2, levm): move crates docs to root docs (#3303)\n\n**Motivation**\nDocs are sparsed across the repo. This PR puts everything in the same\nplace\n\n**Description**\n- Added the docs that lived under `/crates/*` in the root `/docs`. Used\nthe same file structure\n- Deleted all instances of docs under `/crates/*`\n\n\nCloses: none\n\nSigned-off-by: droak <me+git@droak.sh>",
          "timestamp": "2025-07-17T18:10:12Z",
          "tree_id": "17e6af6a4ed0b436216ded6e99f2ed41de21d5c1",
          "url": "https://github.com/lambdaclass/ethrex/commit/8272969a64958abfed0f7085cb7c4d684f2202df"
        },
        "date": 1752779128761,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209901785745,
            "range": "± 499411249",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me+git@droak.sh",
            "name": "Oak",
            "username": "d-roak"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "96c7eeeabfc03ea6b8a20b92f5310cb59d7a63c8",
          "message": "docs(l2): add quotes on init-prover command (#3304)\n\n**Motivation**\nCopy paste on the command provided in the docs doesn't work\n\n**Description**\n- Added quotes to the command mentioned\n\nCloses: none",
          "timestamp": "2025-07-17T18:18:56Z",
          "tree_id": "f41598ce40f7d42ba9c46c352b21d5f3ea010eed",
          "url": "https://github.com/lambdaclass/ethrex/commit/96c7eeeabfc03ea6b8a20b92f5310cb59d7a63c8"
        },
        "date": 1752779778471,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 218209245887,
            "range": "± 1568813520",
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
          "id": "ce87d548d3a30d75bfe4e5a1ae2cf242a316f3f3",
          "message": "feat(l2): add chainid to public inputs (#3605)\n\n**Motivation**\n\nCurrently we don't verify blocks are executed with the correct chain id.\nThis allows malicious sequencers to replay transactions from other\nnetworks.\n\n**Description**\n\nAdds chainid to the public inputs.\n\nCloses #3586\n\n---------\n\nCo-authored-by: fedacking <francisco.gauna@lambdaclass.com>",
          "timestamp": "2025-07-17T18:44:59Z",
          "tree_id": "4823927b668a80a7016f28f6d1d9813bcc57df30",
          "url": "https://github.com/lambdaclass/ethrex/commit/ce87d548d3a30d75bfe4e5a1ae2cf242a316f3f3"
        },
        "date": 1752781229193,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209704740925,
            "range": "± 443477274",
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
          "id": "6876c91b7f93ebbf65124410582e6f89513f9768",
          "message": "perf(levm): codecopy perf improvement (#3675)\n\n**Motivation**\n\nImproves from 200 mgas to 790, this bench was made with this pr along\nmemory, sstore and opcodes ones.\n\nA 295% increase in perf.\n\nRequires the pr #3564 \n\n**Description**",
          "timestamp": "2025-07-18T05:06:43Z",
          "tree_id": "295ee8748a84fa9fc38dc54a27f6f57ac03c6626",
          "url": "https://github.com/lambdaclass/ethrex/commit/6876c91b7f93ebbf65124410582e6f89513f9768"
        },
        "date": 1752818574266,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209991999050,
            "range": "± 536409015",
            "unit": "ns/iter"
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
          "id": "b38493e92ea2602d73e0a840bfae103011105285",
          "message": "refactor(l1, l2): metrics folder (#3346)\n\n**Motivation**\n\nHaving all our metrics dashboards in one folder, so it's all better\norganized.\n\n**Description**\n\nThis PR moves all the dashboards and yaml files needed to set up the\nPrometheus and Grafana containers into one folder at the root.\n\nCloses #3181\n\n---------\n\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-07-18T08:56:10Z",
          "tree_id": "52f41fa0b6b4c2b4b76f577d91652b6854afd484",
          "url": "https://github.com/lambdaclass/ethrex/commit/b38493e92ea2602d73e0a840bfae103011105285"
        },
        "date": 1752832319059,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211134435450,
            "range": "± 649571186",
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
          "id": "d0cb238de7d942202283a47fb8f937144cf595eb",
          "message": "perf(levm): use a lookup table for opcode execution (#3669)\n\n**Motivation**\n\nThe current match is not the fastest option, a function lookup table is\nbetter.\n\nOn gas benchmarks:\n\n- 24% further improvement on MSTORE on top of the new memory model pr\n- Improvement on all gas benches\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3353",
          "timestamp": "2025-07-18T10:37:32Z",
          "tree_id": "0245ea0a64492e20d5dac49e92e1aae31311c387",
          "url": "https://github.com/lambdaclass/ethrex/commit/d0cb238de7d942202283a47fb8f937144cf595eb"
        },
        "date": 1752838432272,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210050718364,
            "range": "± 648313586",
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
          "id": "dcb9deef55336159f21694ab4ac824c0ee801a83",
          "message": "perf(levm): improve block related opcodes and add stack push1 and pop1 (#3704)\n\n**Motivation**\n\nUsing ##3669 as base\n\nBlobhash:\nMgas goes from base p95 500 to 637\nAdding a dedicated push1 method to stack increases it to 711 mgas.\n\nAdding push1 and pop1 to other block opcodes improves mgas by about\n60-100 on their benchmarks.\n\nWill add another pr adding push1 to more opcodes",
          "timestamp": "2025-07-18T11:23:08Z",
          "tree_id": "8b31b05f699432ed712f201b0f023fab9bb4a783",
          "url": "https://github.com/lambdaclass/ethrex/commit/dcb9deef55336159f21694ab4ac824c0ee801a83"
        },
        "date": 1752841132193,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211027792111,
            "range": "± 351470998",
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
          "id": "584ff5c346a9baa29f075c818a12a2297bafeeb0",
          "message": "fix(l1): remove \"p2p\" capability from supported capabilities list (#3571)\n\n**Motivation**\n\nEven if the spec calls it the \"p2p\" capability, it isn't included in the\n`capabilities` array by our peers, but implicitly in the message, with\nthe version specified in the `protocolVersion` field.\n\n**Description**\n\nThis PR removes the \"p2p\" capability from the capability list exchanged\nthrough Hello RLPx messages. It also removes the unused code related to\nit.",
          "timestamp": "2025-07-18T14:43:19Z",
          "tree_id": "3b9e011b3537031595480502820c5dc1e309d999",
          "url": "https://github.com/lambdaclass/ethrex/commit/584ff5c346a9baa29f075c818a12a2297bafeeb0"
        },
        "date": 1752853169286,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 212176728173,
            "range": "± 1164775792",
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
          "id": "5f138d0fc9bc695a8e3865ab911ee75f5707ca0d",
          "message": "ci(l2): cargo.lock check was done before compiling (#3713)\n\n**Motivation**\n\nThe check for if the Cargo.lock was not committed was being done before\ncompiling the zkvm\n\n**Description**\n\n- Move the check to after running cargo clippy",
          "timestamp": "2025-07-18T15:13:22Z",
          "tree_id": "efd06ead27ae2c21187f9a35de926827ff18b339",
          "url": "https://github.com/lambdaclass/ethrex/commit/5f138d0fc9bc695a8e3865ab911ee75f5707ca0d"
        },
        "date": 1752855004378,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209024604121,
            "range": "± 280192752",
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
          "id": "c833309c67f23d1833adc7d0592322a51f89ad5f",
          "message": "fix(l2): tdx to L2 prover client communication (#3599)\n\n**Motivation**\n\nThe communication between the tdx prover client and the l2 prover server\nis currently broken because the nix build does not set the\n`VERGEN_GIT_SHA` variable.\n\n**Description**\n- Modify the ci to use run the image built with nix with QEMU, instead\nof building the quote-gen as a binary and running it as a regular\nbinary.\n- Add `gitRev` as a parameter to nix build\n   - Use this parameter to set the env var `VERGEN_GIT_SHA`\n- Add a check to make sure the length of the commit hash is the same one\nthat Vergen emits.\n- Modify build.rs to get the value from the env var and if the value is\nnot set then it uses Vergen\n- Remove build.rs files for zk provers and tdx quote-gen because the\nvalues are added when building l2 crate.\n- Successful run:\n[here](https://github.com/lambdaclass/ethrex/actions/runs/16275888354/job/45954574993?pr=3599)\n\nCloses #3587",
          "timestamp": "2025-07-18T15:18:09Z",
          "tree_id": "49606faca75b48602d58b35a2b5cc83c15bf805c",
          "url": "https://github.com/lambdaclass/ethrex/commit/c833309c67f23d1833adc7d0592322a51f89ad5f"
        },
        "date": 1752855623894,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207145155988,
            "range": "± 354516416",
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
          "id": "43ed0ffe6efa430b1517d61ab940604eb09f16dd",
          "message": "perf(levm): improve precompiles by avoiding 0 value transfers (#3715)\n\nImprove precompiles by avoiding 0 value transfers\n\nThis idea was suggested by @JereSalo \n\n<img width=\"1690\" height=\"321\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/a2e70d40-9588-4ab6-9005-4e6c3a2a726d\"\n/>",
          "timestamp": "2025-07-18T16:13:05Z",
          "tree_id": "a59e9d10e2850c4714c8b080c9412a2281589687",
          "url": "https://github.com/lambdaclass/ethrex/commit/43ed0ffe6efa430b1517d61ab940604eb09f16dd"
        },
        "date": 1752858613947,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207570929690,
            "range": "± 392327228",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "b9be27f28cc83c491787634fe4f6c25d608b8a92",
          "message": "refactor(l2): use spawned for monitor (#3635)\n\n**Motivation**\nRefactor monitor so that it uses spawned crate\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nCurrently the monitor is a tokio task, refactor it to use spawned\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n**How to test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nDepends on https://github.com/lambdaclass/ethrex/pull/3622\n\nCloses\nhttps://github.com/orgs/lambdaclass/projects/37/views/10?pane=issue&itemId=118809943&issue=lambdaclass%7Cethrex%7C3515\n\n---------\n\nCo-authored-by: Esteban Dimitroff Hodi <esteban.dimitroff@lambdaclass.com>\nCo-authored-by: ElFantasma <estebandh@gmail.com>",
          "timestamp": "2025-07-18T16:43:25Z",
          "tree_id": "0c29b3a639142d5794559cc8118f6ffe745d88af",
          "url": "https://github.com/lambdaclass/ethrex/commit/b9be27f28cc83c491787634fe4f6c25d608b8a92"
        },
        "date": 1752860377062,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210887572622,
            "range": "± 452025328",
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
          "id": "afb42a55a189abd969b62e46b066a819185fafef",
          "message": "chore(l1): include Merkle step in metrics logs (#3630)\n\nIn preparation for performance analysis tooling, we need to include\nMerkle-tree specific measures.\nRigorously speaking, this includes most of the data preparation before\ncommitting to the DB. The code assumes Merkle-tree operations to be the\nbottleneck there.\n\nThe notebook part will be handled in a separate PR based on this branch,\nso we can keep the logic in it simpler by only handling one version of\nthe logs at a time. It also should make it easier to review.\n\nBased on #3274\nCoauthored-by: @Arkenan\n\nPart of: #3331",
          "timestamp": "2025-07-18T16:46:13Z",
          "tree_id": "f48445b5455b560bd98288480929a48e01fd7a9e",
          "url": "https://github.com/lambdaclass/ethrex/commit/afb42a55a189abd969b62e46b066a819185fafef"
        },
        "date": 1752860586939,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209821384176,
            "range": "± 503832231",
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
          "id": "4fc03c98fd813c7cb18753f83a8d4702713c6b29",
          "message": "perf(levm): improve most opcodes by using stack pop1 push1 (#3705)\n\n**Motivation**\n\nThe methods pop1 and push1 added in\nhttps://github.com/lambdaclass/ethrex/pull/3704 show that they increase\nperfomance in benchmarks, this pr addes them to all other opcodes",
          "timestamp": "2025-07-18T16:48:43Z",
          "tree_id": "e2b2b0431e9deb469134e6f933a7bec20028c3a4",
          "url": "https://github.com/lambdaclass/ethrex/commit/4fc03c98fd813c7cb18753f83a8d4702713c6b29"
        },
        "date": 1752860799422,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 211254886551,
            "range": "± 597374231",
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
          "id": "baf5803c56eba68abef46706bd64c97ce64aba47",
          "message": "refactor(levm): make runner more usefull for debugging (#3601)\n\n**Motivation**\n\nMake runner more usefull for debugging. \n- When running specific tests, stop showing all the directories parsed\nto get the tests, and only show the ones where the selected tests are\nin.\n- Give more detail about the failing tests.\n- Don't saturate console output with endless log messages.\n\n**Description**\n\nThis pr makes changes in the runners code, now when parsing the tests\nonly the relative paths of the directories parsed are printed, not just\nthe names of the directories. (Closes #3578)\n\nTo see this change you can run:\n\n```bash\ncd cmd/ef_tests/state\nmake test-levm\n```\n\nThis command downloads and runs all tests only for levm. If you want to\nrun specific tests you can run:\n\n```bash\ncd cmd/ef_tests/state\ncargo test --package ef_tests-state --test all --release -- --forks Prague,Cancun --tests blob_tx_attribute_calldata_opcodes.json\n```\n\nIt also incorporates a new `paths` flag for running tests given their\npaths. The flag is to parse directly a test file instead of going over\nall directories trying to find the file only by its name. (This also\nCloses #3578)\n\nYou can try it by running:\n\n```bash\ncd cmd/ef_tests/state\ncargo test --package ef_tests-state --test all --release -- --forks Prague,Cancun --summary  --paths --tests LegacyTests/Cancun/GeneralStateTests/Shanghai/stEIP3855-push0/push0.json,GeneralStateTests/Shanghai/stEIP3855-push0/push0.json,GeneralStateTests/stBadOpcode/invalidAddr.json,LegacyTests/Cancun/GeneralStateTests/stBadOpcode/invalidAddr.json\n```\n\nThis should be the same as running the example we have in the read me:\n\n```bash\ncd cmd/ef_tests/state\ncargo test --package ef_tests-state --test all --release -- --forks Prague,Cancun --summary --tests push0.json,invalidAddr.json\n```\n\nFor failing tests, now the code shows the name and the path of the\nfailing tests, and it mentions the runner and the line where the error\nwas thrown. (Closes #3579)\nAlso, to provide more information about the tests, this pr also includes\nparsing more information from the test's .json in the `EFTestReport`\nstruct. This information is saved in the fields `description`, `url` and\n`reference_spec`. `url` and `reference_spec` refer to the url to the\npython test, and to the repo of the EIP the tests refer to,\nrespectively. It also lists\n[https://ethereum-tests.readthedocs.io/en/latest/test_types/gstate_tests.html#](https://ethereum-tests.readthedocs.io/en/latest/test_types/gstate_tests.html#)\nas a resource for understanding the tests. (Closes #3581)\n\nNow when a test fails it looks like this:\n```\nstate_tests/frontier/precompiles/precompile_absence: 6/6 (100.00%)\nstate_tests/cancun/eip6780_selfdestruct/dynamic_create2_selfdestruct_collision: 12/12 (100.00%)\nGeneralStateTests/stTransitionTest: 12/12 (100.00%)\nstate_tests/cancun/eip4844_blobs/blob_txs: 1173/1174 (99.91%)\n\n\nRunning failed tests with REVM...\nRe-running failed tests with REVM 1/1 - 00:00\nFailing Test: \nTest name: tests/cancun/eip4844_blobs/test_blob_txs.py::test_blob_tx_attribute_calldata_opcodes[fork_Cancun-state_test-tx_gas_500000-empty-opcode_CALLDATACOPY]\nTest path: /Users/camiladiielsi/Documents/ethrex/cmd/ef_tests/state/vectors/state_tests/cancun/eip4844_blobs/blob_txs_blob_tx_attribute_calldata_opcodes.json\n\nTest description: Test calldata related opcodes to verify their behavior is not affected by blobs.\n\n    - CALLDATALOAD\n    - CALLDATASIZE\n    - CALLDATACOPY\n\nNote: The following links may help when debugging `ef-tests`:\n- https://ethereum-tests.readthedocs.io/en/latest/test_types/gstate_tests.html#\n- Test reference spec: https://github.com/ethereum/EIPs/blob/master/EIPS/eip-4844.md\n- Test url: https://github.com/ethereum/execution-spec-tests/tree/v4.5.0/tests/cancun/eip4844_blobs/test_blob_txs.py#L1218\n\nError: Internal(MainRunnerInternal(\"Non-internal error raised when executing revm. This should not happen: Err(Internal(Custom(\\\"Error at LEVM::get_state_transitions() thrown in REVM runner line: 391 when executing ensure_post_state()\\\")))\"))\nerror: test failed, to rerun pass `-p ef_tests-state --test all`\n```\n(Note: the error shown was fabricated by changing the post hash value of\nthe test.)\n\nAnother change introduced is that now there are no more endless log\nmessages, the messages are overwritten with actualized information of\nthe tests that are being executed. (Closes #3580) You can see that by\nrunning any of the commands mentioned.\n\nThis solution is meant to be a temporary fix since there's a new runner\nbeing programmed from scratch to improve the user experience.",
          "timestamp": "2025-07-18T18:20:27Z",
          "tree_id": "b6823ee0a0d7af9bc64ced7939b1b34fe6222a76",
          "url": "https://github.com/lambdaclass/ethrex/commit/baf5803c56eba68abef46706bd64c97ce64aba47"
        },
        "date": 1752866193448,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 210347514164,
            "range": "± 1722701120",
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
          "id": "fa7a8b5083607a60e462305797be8ea06e306a20",
          "message": "ci(l1): comment out flaky findnode test (#3699)\n\nI thought this test had already been commented out before but maybe it\nwas reintroduced, I don't know. It's flaky though.\n[Example\nrun](https://github.com/lambdaclass/ethrex/actions/runs/16356024570/job/46215135795?pr=3626)",
          "timestamp": "2025-07-18T19:19:41Z",
          "tree_id": "7bdf26b8bea2a56a896280c98800be6313628a0f",
          "url": "https://github.com/lambdaclass/ethrex/commit/fa7a8b5083607a60e462305797be8ea06e306a20"
        },
        "date": 1752869769963,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209643134431,
            "range": "± 638634859",
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
          "id": "a7c1554b0e7d90a58dd8d17c1f4e0709727bcf0d",
          "message": "refactor(l1,l2): remove k256 crate  (#3689)\n\n**Motivation**\n\nHaving both k256 and secp256k1 crates makes it so that sp1 patches don't\ncompile\n\n**Description**\n\n- Remove k256 from the workspace\n- Migrate all functions from k256 to use secp256k1 crate",
          "timestamp": "2025-07-18T19:43:52Z",
          "tree_id": "a0448274d014503f744bd56b85166c9ac49383fc",
          "url": "https://github.com/lambdaclass/ethrex/commit/a7c1554b0e7d90a58dd8d17c1f4e0709727bcf0d"
        },
        "date": 1752871234653,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207733524355,
            "range": "± 421094072",
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
          "id": "552bc18da6f30075a0b833cf755441497ad8805e",
          "message": "ci(l1): fix assertoor tests  (#3718)\n\n**Motivation**\n\nWe were using an old kurtosis version and when the\n`ethrex-only-different-cl` job was failing because of this\n\n**Description**\n\nIn this PR the kurtosis version used in Github actions was updated to\nlatest from 1.6 (up to now the current version is 1.10.2)\nAlso the el and cl images used in those jobs were removed, by doing this\nthe default one is used\nThis PR changes the `lambda/ethereum-package` branch into main.\n\n⚠️ **Depends on this\n[PR](https://github.com/lambdaclass/ethereum-package/pull/16)** ⚠️\n\n\nCloses #3712",
          "timestamp": "2025-07-18T20:16:35Z",
          "tree_id": "6c9056b04628842c517fc3e06d29558793b00943",
          "url": "https://github.com/lambdaclass/ethrex/commit/552bc18da6f30075a0b833cf755441497ad8805e"
        },
        "date": 1752873182824,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207259697929,
            "range": "± 648030357",
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
          "id": "894ac4965366c7e956dcc105fb118fd54257d173",
          "message": "feat(l2): privileged transaction inclusion deadline (#3427)\n\n**Motivation**\n\nTo prevent the sequencer from censoring transactions, we want to force\nit to include at least some of them.\n\n**Description**\n\nThis PR introduces a deadline after which either `INCLUSION_BATCH_SIZE`\n(or all pending transactions, if there are less) privileged transactions\nare included, or the batch is rejected.\n\nCloses #3230",
          "timestamp": "2025-07-18T20:33:42Z",
          "tree_id": "28cbcf100c80d1d325cfb9dc2464fe929feb32c9",
          "url": "https://github.com/lambdaclass/ethrex/commit/894ac4965366c7e956dcc105fb118fd54257d173"
        },
        "date": 1752874178496,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 208030310237,
            "range": "± 452013088",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "62400508+juan518munoz@users.noreply.github.com",
            "name": "juan518munoz",
            "username": "juan518munoz"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "a94a05cbefa95d78e1636ab6222182eeca237c0b",
          "message": "refactor(l1): replace rlpx listener functions with `spawned` implementation (#3504)\n\nRemove `spawn_listener` and `spawn_broadcast_listener` functions in\nfavour of `spawned`'s helper functions for its GenServers.\n\nNote: this PR should not be merged until [related spawned\nchanges](https://github.com/lambdaclass/spawned/pull/20) are available\non a release.\n\nCloses #3387",
          "timestamp": "2025-07-18T20:39:01Z",
          "tree_id": "e9232714cba7726018f3130c7087cd13876b3e63",
          "url": "https://github.com/lambdaclass/ethrex/commit/a94a05cbefa95d78e1636ab6222182eeca237c0b"
        },
        "date": 1752874558139,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 212693819346,
            "range": "± 359091162",
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
          "distinct": true,
          "id": "d93521e7397fbab87d4f8a431f06e8132fd35b40",
          "message": "chore(l1): move network_params_ethrex_only.yaml to the fixtures folder  (#3729)\n\n**Motivation**\n\nThe localnet ethrex_only was used for CI, now it isn't but the\nnetwork_params_ethrex_only.yaml was kept for testing but it still was in\nthe `.github/config/assertoor` folder\n\n**Description**\n\nIn PR #3324 the file was moved into `fixtures/network`\nChanged path to file in the `Makefile` so that `make\nlocalnet-ethrex-only` works again\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3625",
          "timestamp": "2025-07-18T21:07:02Z",
          "tree_id": "3c98c36d78e26ba06ccccd3dd5c51fa89503c507",
          "url": "https://github.com/lambdaclass/ethrex/commit/d93521e7397fbab87d4f8a431f06e8132fd35b40"
        },
        "date": 1752876226132,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 209142808308,
            "range": "± 1592141185",
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
          "id": "db2b5a52032ac4e7c6934e92c2f259db8e545910",
          "message": "feat(l1): improve rebuild progress tracking (#2428)\n\n**Motivation**\nPreviously, state and storage rebuilding took approximately the same\ntime, so showing state trie rebuild progress was enough to keep the user\nupdated. After some recent changes storage rebuilding is taking a bit\nlonger, making its progress more relevant.\nAlso, the estimated finish time calculation takes into account all time\nsince start, which means estimations at the start will be abnormally\nhigh as time is spent waiting for data to be downloaded instead of pure\nrebuilding time.\nThis PR tracks the average speed and remaining storages to rebuild and\nperiodically shows them. It will only show the estimated finish time if\nthe state sync is complete, and it will show the average rebuild speed\nas debug tracing.\nIt also counts the time spend rebuilding storages instead of the time\nthat has passed since the rebuild started when performing time\nestimation\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Periodically show storage tries rebuild stats (speed/remaining for\ndebug, estimated finish time if state sync is finished)\n* Count time taken during rebuild instead of total time taken when\nestimating rebuild finish times\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-18T21:26:37Z",
          "tree_id": "16144ff626bcf651a6fab3598ece2b7389c8f45f",
          "url": "https://github.com/lambdaclass/ethrex/commit/db2b5a52032ac4e7c6934e92c2f259db8e545910"
        },
        "date": 1752877308973,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 205533662870,
            "range": "± 782561225",
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
          "id": "ce5c47df70fa92c91814f36df65c01a090b19de1",
          "message": "fix(l2): estimate gas in call_to_contract_with_deposit (#3734)\n\n**Motivation**\n\nThe CI is failing on\n[main](https://github.com/lambdaclass/ethrex/actions/runs/16376083320/job/46276248732)\nwith the following error:\n\n```\nthread 'l2_integration_test' panicked at crates/l2/tests/tests.rs:1604:65:\ncalled `Option::unwrap()` on a `None` value\n```\n\nThis is because we were using a hardcoded `gas_limit` for the\n`l1_to_l2_tx` in the `call_to_contract_with_deposit` test, and sometimes\nthe tx fails due to the gas limit being exceeded. Then, the expected\nlogs of are never created.\n\n**Description**\n\n- Replaces the hardcoded `gas limit` with `None` to allow the SDK to\nestimate the value.\n\nCloses None",
          "timestamp": "2025-07-18T22:01:22Z",
          "tree_id": "77ef3295f487c841398d311894bdbbe16ec60cc8",
          "url": "https://github.com/lambdaclass/ethrex/commit/ce5c47df70fa92c91814f36df65c01a090b19de1"
        },
        "date": 1752879358612,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 207461544676,
            "range": "± 348294554",
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
          "distinct": true,
          "id": "f83a4f9f22c138921781be5a3dda82bcb09bae09",
          "message": "perf(l1): use rayon for recover address (#3709)\n\n**Motivation**\nThis logic was originally introduced in #2268 but was mistakenly removed\nin a refactor\n[PR](https://github.com/lambdaclass/ethrex/pull/3082/files#diff-6ca74a0741dab646bb82b83636f9513d38a1c66b9db52dae8e20a0ec2fe6c1a3L239-L241).\nWe're adding it back because it improves the performance of our\nbenchmarks.\n\n**Description**\n\nAdd `par_iter` to `recover_address` function \n\nBiggest changes\n- ETH transfers (+117.96%)\n- Gas-Pop (+36.35%)\n- Push0 (+33.08%): Push de ceros al stack mucho más eficiente\n- Timestamp (+20.26%)\n- CoinBase (+17.18%)\n- Caller (+15.19%)\n- GasLimit (-14.33%)\n- BlobHash (-12.97%)\n\nThis change affects most of the benchmark tests, but it restores logic\nthat had previously been part of the codebase.\n\ncloses #3725",
          "timestamp": "2025-07-21T14:53:07Z",
          "tree_id": "cf5b10ad7fe07b1e6f4e95114c7230ad623fa3ed",
          "url": "https://github.com/lambdaclass/ethrex/commit/f83a4f9f22c138921781be5a3dda82bcb09bae09"
        },
        "date": 1753112989017,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 168562004534,
            "range": "± 376659506",
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
          "id": "b9f189573533b771f82ba45ef7bd65daefd02a55",
          "message": "refactor(l2): remove blockByNumber (#3752)\n\n**Motivation**\n\nWhile reviewing areas for simplification, I found that `BlockByNumber`\nis not being used.\n\n**Description**\n\nRemoves `BlockByNumber`\n\nCloses #3748",
          "timestamp": "2025-07-21T19:08:00Z",
          "tree_id": "030f712bb4c97a9bb320513b3614f976638c92cd",
          "url": "https://github.com/lambdaclass/ethrex/commit/b9f189573533b771f82ba45ef7bd65daefd02a55"
        },
        "date": 1753127758216,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 168274212566,
            "range": "± 567365076",
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
          "id": "93a885595f00e092fd597e03270b214da85114a2",
          "message": "fix(levm): preemptively resize memory before executing call (#3592)\n\n**Motivation**\nWhen executing a `CALL` opcode, a transfer might take place. In this\ncase the instruction does contain a return data offset and a return data\nsize but as we don't have return data to write into memory we don't\nexpand the memory.\nThis can cause problems with other opcodes later on (such as MSTORE,\nMLOAD, etc) which calculate their gas cost based on the difference\nbetween the current size of the memory and the new size, making them\nmore expensive as the memory will be smaller due to return data from\ntransfers not being accounted for.\nThis PR aims to solve this by preemptively resizing the memory before\nexecuting the call, so that the memory gets expanded even if no return\ndata is written to it.\nThis bug was found on Sepolia transaction:\nhttps://sepolia.etherscan.io/tx/0xa1765d420522a40d59d15f8dee1bf095499be687d6e1a7c978fc87eb85bce948\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Preemptively resize memory before executing a call in opcode `CALL`\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n**Questions**\nShould this behaviour also apply to other call types?\nCloses #issue_number",
          "timestamp": "2025-07-21T21:12:15Z",
          "tree_id": "4e31ff7d7cf6aea3a6a0e6f216926845008e439f",
          "url": "https://github.com/lambdaclass/ethrex/commit/93a885595f00e092fd597e03270b214da85114a2"
        },
        "date": 1753135255784,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 165019947162,
            "range": "± 413787128",
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
          "id": "ec0a3eb6b536dda5668dff993369e9067f8709dd",
          "message": "chore(levm): parallelize parsing ef state tests (#3722)\n\n**Motivation**\n\nEf test parsing is slow, this parallelizes it making it faster\n\n\nRan in 2m0.225s",
          "timestamp": "2025-07-22T09:08:06Z",
          "tree_id": "791d9a8fffafcc8a4268db6202ce229dac132476",
          "url": "https://github.com/lambdaclass/ethrex/commit/ec0a3eb6b536dda5668dff993369e9067f8709dd"
        },
        "date": 1753177922911,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 167823259546,
            "range": "± 260249867",
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
          "id": "e283db20a41622318fda4869992e08591911625e",
          "message": "feat(levm): execute arbitrary bytecode (#3626)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\nHave a runner for LEVM that expects some **inputs** like Transaction,\nFork, etc. in an `json` file and **bytecode in mnemonics** in another\nfile. Stack and memory can be preloaded within the `json`.\nMore info in the `README.md`\n\nSidenote: I had to do a refactor in LEVM setup because for me to be able\nto alter the stack and memory before executing these have to be\ninitialized in the `new()`, thing that we weren't doing. So we now\ninitialize the first callframe there and not in `execute()`.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3583\n\n---------\n\nCo-authored-by: fmoletta <99273364+fmoletta@users.noreply.github.com>\nCo-authored-by: Edgar <git@edgl.dev>",
          "timestamp": "2025-07-22T13:16:10Z",
          "tree_id": "f5786f587ea0b2b549d9262913b775de1a103a34",
          "url": "https://github.com/lambdaclass/ethrex/commit/e283db20a41622318fda4869992e08591911625e"
        },
        "date": 1753192881343,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 169400848999,
            "range": "± 284934748",
            "unit": "ns/iter"
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
          "id": "2ce46bf32443be74f0d2ee8b0d5759f9c04219cb",
          "message": "refactor(levm): rewrite of state EF tests runner first iteration (#3642)\n\n**Motivation**\n\nRelated issue: #3496. \n\nThe idea is to incrementally develop a new EF Test runner (for state\ntests) that can eventually replace the current one. The main goal of the\nnew runner is to be easy to understand and as straightforward as\npossible, also making it possible to easily add any new requirement.\n\n**How to run** \nA target in the makefile was included. You can, then, from\n`ethrex/cmd/ef_tests/state/` run `make run-new-runner`. If no specific\npath is passed, it will parse anything in the `./vectors` folder.\nOtherwise you can do, for example:\n`make run-new-runner TESTS_PATH=./vectors/GeneralStateTests/Cancun` to\nspecify a path.\n\nThis command assumes you have the `vectors` directory downloaded, if not\nrun `make download-evm-ef-tests` previously.\n\n**Considerations**\n\nThe main changes are: \n- The new `Test` and `TestCase` structures in types. \n- The runner and parser simplified flows. \n\nFiles that should not be reviewed as they are full or partial copies of\nthe original files:\n- `runner_v2/deserialize.rs`\n- `runner_v2/utils.rs`\n\nThis iteration excludes report-related code, option flags and other\npossible test case errors to be considered that will be included later.\nChecks are performed only on exceptions and root hash.",
          "timestamp": "2025-07-22T18:32:15Z",
          "tree_id": "1364dd940e94383958d73b23545152bd053470bf",
          "url": "https://github.com/lambdaclass/ethrex/commit/2ce46bf32443be74f0d2ee8b0d5759f9c04219cb"
        },
        "date": 1753211785245,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 170461481232,
            "range": "± 582474291",
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
          "id": "b41d878a318aeaf8dcbd7c2292569fa697282a76",
          "message": "fix(l2): fix L1 proof sender's wallet/signer (#3747)\n\n**Motivation**\n\nThe L1 proof sender was broken in #2714 by creating an invalid ethers'\n`Wallet`\n[here](github.com/lambdaclass/ethrex/pull/2714/files#r2216602944). This\nPR fixes it but only allows running the proof sender with a local\nsigner.\n\nTo support a remote signer we must investigate if there's a way to\ncreate an ethers' signer that uses web3signer.\n\nThanks @avilagaston9 for noticing the bug!",
          "timestamp": "2025-07-22T19:31:59Z",
          "tree_id": "20136d8c01aeca6066b9529a2e69154d4cacc679",
          "url": "https://github.com/lambdaclass/ethrex/commit/b41d878a318aeaf8dcbd7c2292569fa697282a76"
        },
        "date": 1753215445373,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 169428522229,
            "range": "± 322104276",
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
          "id": "cfe00d33bd3464bd5cd625978d92a0f1f8068f63",
          "message": "fix(l2): enable bls12_381,k256 & ecdsa sp1 precompiles (#3691)\n\n**Motivation**\n\nThe patch for bls12_381 precompile is not being applied because we are\nimporting the crate from our fork.\nAlso two other patches that were previously not compiling after #3689 is\nmerged can now be reenabled\n\n**Description**\n\n-\n[Forked](https://github.com/lambdaclass/bls12_381-patch/tree/expose-fp-struct)\nthe patch from sp1 and updated it with the same changes we have on the\nmain crate fork\n- Uncommented the previously commented patches\n\n**How to check**\n\n```\ncd crates/l2/prover/zkvm/interface/sp1\n```\nbls12_381\n```\ncargo tree -p bls12_381\n```\nreturns `bls12_381 v0.8.0\n(https://github.com/lambdaclass/bls12_381-patch/?branch=expose-fp-struct#f2242f78)`\n\necdsa\n```\ncargo tree -p ecdsa\n```\nreturns `ecdsa v0.16.9\n(https://github.com/sp1-patches/signatures?tag=patch-16.9-sp1-4.1.0#1880299a)`\n\nk256\n```\ncargo tree -p k256\n```\nreturns `k256 v0.13.4\n(https://github.com/sp1-patches/elliptic-curves?tag=patch-k256-13.4-sp1-5.0.0#f7d8998e)`\n\nComparing this to main that it either returns no patch or errors out",
          "timestamp": "2025-07-22T19:50:33Z",
          "tree_id": "556520e376bd9a93a8bfb4cc139da53a6e4531d0",
          "url": "https://github.com/lambdaclass/ethrex/commit/cfe00d33bd3464bd5cd625978d92a0f1f8068f63"
        },
        "date": 1753216479511,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 170390827733,
            "range": "± 1386689472",
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
          "id": "c36b343d8508b62b7de0a7d87bc58a026278704a",
          "message": "fix(levm): memory bug when storing data (#3774)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nWe didn't realize that #3564 introduced a bug when storing data of\nlength zero. This aims to fix it.\nI also delete a resize check that's completely unnecessary\n\nTested the fix and it works. I now am able to execute the blocks\nmentioned in the issue of this PR without any problems at all.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3775",
          "timestamp": "2025-07-22T20:11:38Z",
          "tree_id": "db0e155e11ab6e5d6808c62ec2e43bfed8834f9a",
          "url": "https://github.com/lambdaclass/ethrex/commit/c36b343d8508b62b7de0a7d87bc58a026278704a"
        },
        "date": 1753217740041,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 169231622767,
            "range": "± 661858953",
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
          "id": "4a3a5aec56e6b6a96942ad161b32fd2f50ccd5c7",
          "message": "refactor(l2): apply fcu only on the last block of the batch for the block fetcher (#3782)\n\n**Motivation**\n\nWith the actual implementation of the block fetcher, we apply a fork\nchoice update for every block. This is not the optimal way since we can\napply only on the last block.\n\n**Description**\n\n- Move the `apply_fork_choice` call after the loop and only call it with\nthe last block\n- Add new type of error `EmptyBatchError`",
          "timestamp": "2025-07-22T21:03:49Z",
          "tree_id": "710ac64388c952a5a69f10f5b9a1eda132ef6c9a",
          "url": "https://github.com/lambdaclass/ethrex/commit/4a3a5aec56e6b6a96942ad161b32fd2f50ccd5c7"
        },
        "date": 1753220909490,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 170091441577,
            "range": "± 656600594",
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
          "distinct": false,
          "id": "3f60642861576555f50dd330410eb75c49188447",
          "message": "feat(l2): based P2P (#2999)\n\n**Motivation**\n\nThis PR follows #2931 . We implement some basic functionality to\ncommunicate L2 based nodes via P2P.\n\n**Description**\n\n- Add new capability to the RLPx called `Based`.\n- Add new Message `NewBlock`.\n  - Behaves similar to the message `Transactions`.\n- Every interval we look to the new blocks produced and send them to the\npeer.\n- Add this message to the allowed ones to be broadcasted via the P2P\nnetwork.\n- When receiving this message we implemented a queue to be able to\nreceive them in disorder. Once a continuos interval of blocks is in the\nqueue we store them in order.\n- Add new message `BatchSealed`\n- Every interval we look in the `store_rollup` if a new batch has been\nsealed and then we send it to the peer.\n- Add this message to the allowed ones to be broadcasted via the P2P\nnetwork.\n- This two new messages are signed by the lead sequencer who proposed\nthe blocks and the batches. Every node must verify this signature\ncorrespond to the lead sequencer\n- Change `BlockFetcher` to not add a block received via the L1 if it\nalready has been received via P2P, and vice versa.\n- Add a new `SequencingStatus`: `Syncing`. It is for nodes that are not\nup to date to the last committed batch.\n\n**How to test**\n\nRead the `Run Locally` section from `crates/l2/based/README.md` to run 3\nnodes and register 2 of them as Sequencers. It is important that you\nassign different values in the nodes:\n- `--http.port <PORT>`\n- `--committer.l1-private-key <PRIVATE_KEY>`\n- `--proof-coordinator.port <PORT>`\n- `--p2p.port <P2P_PORT>`\n- `--discovery.port <PORT>`\n\n> [!TIP]\n> To enrich the review, I strongly suggest you read the documentation in\n`crates/l2/based/docs`.\n\n---------\n\nCo-authored-by: Leandro Serra <leandro.serra@lambdaclass.com>\nCo-authored-by: ilitteri <ilitteri@fi.uba.ar>\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: fkrause98 <fkrausear@gmail.com>\nCo-authored-by: Francisco Krause Arnim <56402156+fkrause98@users.noreply.github.com>",
          "timestamp": "2025-07-22T22:04:21Z",
          "tree_id": "2fbb970513ea40b72556f60e771be7c88d6540a4",
          "url": "https://github.com/lambdaclass/ethrex/commit/3f60642861576555f50dd330410eb75c49188447"
        },
        "date": 1753224520479,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 168942697642,
            "range": "± 457770537",
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
          "id": "c1778eadda9854a3824aac5f304204150c14a97b",
          "message": "chore(l1): change logs in hive to info by default (#3767)\n\n**Motivation**\n\nIn the PR #2975 the default value for the `make run-hive` was changed to\nerror. I propose changing this to info (3), as we usually run make hive\nto try to see a problem with the test. For the CI I propose we change it\nto log level error (1), as we can't actually look at those logs.\n\n**Description**\n\n- Changed makefile `SIM_LOG_LEVEL` default value to 3 (info)\n- Added to the ci workflows `--sim.loglevel 1` which corresponds to\nerror.",
          "timestamp": "2025-07-23T13:51:18Z",
          "tree_id": "c226e0a9f7ec2b05e2e4c8136af012522784660a",
          "url": "https://github.com/lambdaclass/ethrex/commit/c1778eadda9854a3824aac5f304204150c14a97b"
        },
        "date": 1753281295838,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 165884922401,
            "range": "± 207154120",
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
          "id": "4edd454bf4df8dad51b2c32a810a89cd2a9479a6",
          "message": "chore(l1): avoid running EF blockchain tests on `make test` (#3772)\n\n**Motivation**\nThey take a some time and `make test` should be more of a healthcheck\nimo. They run in the CI anyway.",
          "timestamp": "2025-07-23T15:25:54Z",
          "tree_id": "bc9f06de3dbdad519a7d50581577dea43afb1fa8",
          "url": "https://github.com/lambdaclass/ethrex/commit/4edd454bf4df8dad51b2c32a810a89cd2a9479a6"
        },
        "date": 1753287063429,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 167899094786,
            "range": "± 390332596",
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
          "id": "e2cb314efc88038727816005e66b3ee99def5c8c",
          "message": "feat(levm): subcommand for converting mnemonics into bytecode and accepting both kinds as arguments (#3786)\n\n**Motivation**\n\n- Add code related features to levm runner\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- Accept both raw bytecode and mnemonics as arguments for the `--code`\nflag in the `.txt` file\n- Add `--emit-bytes` for converting mnemonics into a new bytecode file\nthat can then be used for running the transaction without parsing the\nvalues.\n\nCloses #3788",
          "timestamp": "2025-07-23T15:42:14Z",
          "tree_id": "fcad77c30da72b4e4322322f55b3bc04ba4e1bd9",
          "url": "https://github.com/lambdaclass/ethrex/commit/e2cb314efc88038727816005e66b3ee99def5c8c"
        },
        "date": 1753288038087,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 168008058749,
            "range": "± 506737880",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "56092489+ColoCarletti@users.noreply.github.com",
            "name": "Joaquin Carletti",
            "username": "ColoCarletti"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "8408fe0854a66e0a510b0a6bf474dda20edd38de",
          "message": "perf(levm): migrate EcAdd and EcMul to Arkworks (#3719)\n\nThis PR improves the performance of the precompiles by switching to\nArkworks.\nIn particular, scalar multiplication on the BN254 curve is significantly\nfaster in Arkworks compared to Lambdaworks.\n\ncloses #3726\n\n---------\n\nCo-authored-by: Leandro Serra <leandro.serra@lambdaclass.com>",
          "timestamp": "2025-07-23T16:04:55Z",
          "tree_id": "780cbcf4c7f07b65b63ff07011ea6247e03377cc",
          "url": "https://github.com/lambdaclass/ethrex/commit/8408fe0854a66e0a510b0a6bf474dda20edd38de"
        },
        "date": 1753289352430,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 166954074952,
            "range": "± 387087755",
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
          "id": "1802f66ed21aff9ca45056ad9a0a6a81b6a4a2b0",
          "message": "feat(l1): notebook for high-level profiling (#3633)\n\nIntroduce a new notebook to analyze contribution of eaxh part of the\nblock production process to its overall time, producing graphs for\nvisual clarity.\nInstructions included in the README.\n\nBased on #3274\nCoauthored-by: @Arkenan\n\nPart of: #3331",
          "timestamp": "2025-07-23T17:15:17Z",
          "tree_id": "90f55d482f41009e1f0aab974c2f11afaaef03e1",
          "url": "https://github.com/lambdaclass/ethrex/commit/1802f66ed21aff9ca45056ad9a0a6a81b6a4a2b0"
        },
        "date": 1753293592187,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 169250483198,
            "range": "± 911093676",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "62400508+juan518munoz@users.noreply.github.com",
            "name": "juan518munoz",
            "username": "juan518munoz"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "31808c9e890a3af68e659735c63dcbb47df85a56",
          "message": "chore(l1,l2): bump spawned version to `0.2.1` (#3780)\n\n**Motivation**\n\nUpdate Spawned to accomodate new Actor interface.\n\n**Description**\n\nSince [spawned `0.2.0`](https://github.com/lambdaclass/spawned/pull/35)\nthe state and GenServer is \"the same\".",
          "timestamp": "2025-07-23T18:10:26Z",
          "tree_id": "1e3122f81bfb5a1e4cfcd914eae36c824d663bfc",
          "url": "https://github.com/lambdaclass/ethrex/commit/31808c9e890a3af68e659735c63dcbb47df85a56"
        },
        "date": 1753296913974,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 166022088539,
            "range": "± 512383533",
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
          "id": "212d72a92a0c1ca9a718e3c46bafd1e7fe5ab163",
          "message": "fix(l2): join verifier task (#3781)\n\n**Motivation**\n\nThe `join()` of the verifier task was accidentally removed in #3635.\n\n**Description**\n\nThis PR is a quick fix that restores the removed `join()`. The verifier\ntask is being replaced by spawned in #3761.\n\nCloses None",
          "timestamp": "2025-07-23T21:44:07Z",
          "tree_id": "49a9893bb669d69fe5e9328bdb23cec1380dfa41",
          "url": "https://github.com/lambdaclass/ethrex/commit/212d72a92a0c1ca9a718e3c46bafd1e7fe5ab163"
        },
        "date": 1753309652217,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 166317527640,
            "range": "± 354129657",
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
          "id": "614cc6d0300718b727304672d93a2ddf6adaf21d",
          "message": "docs(l1): move install instructions to new section and embed script one-liner (#3505)\n\n**Motivation**\n\nSince the install script just builds from source using a `cargo install`\none-liner, it's preferable to show that instead of having to download\nand run an install script.\n\n**Description**\n\nThis PR removes the install script, embedding the one-liner inside the\ndocs. It also moves the installation instructions to the book, linking\nto it in the readme, and expands them with instructions on how to build\nfrom source or download the pre-built binaries.\n\n---------\n\nCo-authored-by: Manuel Iñaki Bilbao <manuel.bilbao@lambdaclass.com>",
          "timestamp": "2025-07-23T21:47:54Z",
          "tree_id": "0b46ef2d7f648cf19cf1c02cfa8af0c4501391a5",
          "url": "https://github.com/lambdaclass/ethrex/commit/614cc6d0300718b727304672d93a2ddf6adaf21d"
        },
        "date": 1753309894719,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 169012552199,
            "range": "± 557548655",
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
          "id": "67cd8bea1ce06c8a875599f420a1ca05f528aa07",
          "message": "feat(l2): embed contracts in deployer and system_contracts_updater (#3604)\n\n**Motivation**\n\nThis PR embeds the bytecode of the contracts used in the `deployer` and\n`system_contracts_updater` as constants within the resulting binaries.\n\n**Description**\n\n- Adds a `build.rs` script under `crates/l2/contracts/bin/build.rs` that\ndownloads all necessary dependencies and compiles all required\ncontracts.\n- Modifies `deployer` and `system_contracts_updater` to import the\nresulting bytecodes as constants using `include_bytes!`, instead of\ncompiling them at runtime.\n- Removes the `download_contract_deps` function from the SDK, as it was\nonly cloning the same two repositories and was used even when only one\nwas needed.\n- Updates the `compile_contract` function in the SDK to accept a list of\n`remappings`.\n- Adds `deploy_contract_from_bytecode` and\n`deploy_with_proxy_from_bytecode` functions to the SDK.\n- Updates tests to work with the new SDK API.\n\n> [!NOTE]\n> The new `build.rs` script checks if `COMPILE_CONTRACTS` is set to\ndecide whether to compile the contracts.\n> This prevents `cargo check --workspace` from requiring `solc` as a\ndependency.\n\nCloses #3380",
          "timestamp": "2025-07-24T12:52:01Z",
          "tree_id": "181b3933b4e4d0214fffc3f5448d06d614709de8",
          "url": "https://github.com/lambdaclass/ethrex/commit/67cd8bea1ce06c8a875599f420a1ca05f528aa07"
        },
        "date": 1753364241494,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 167882348176,
            "range": "± 329419272",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "df3a9bd81724520f527cc837775419629eebcfec",
          "message": "feat(l2): enhance monitor performance (#3757)\n\n**Motivation**\nIf a sequencer runs for a long time, it stops, and we run it again\nactivating the monitor, it takes a long time to start and is slow.\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nMakes the monitor load and work faster by simplifying the batches\nprocessing.\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n**How to Test**\n\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Let the sequencer ran for some time (at least 60 batches)\n- Kill the sequencer\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run `make init-l2-no-metrics`\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-07-24T14:16:59Z",
          "tree_id": "aea4bef1fd38b28adda61ffe55e827444f640da9",
          "url": "https://github.com/lambdaclass/ethrex/commit/df3a9bd81724520f527cc837775419629eebcfec"
        },
        "date": 1753369289952,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 165309285062,
            "range": "± 251370086",
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
          "id": "8d7a9096401de0e6ff01c6e66e19513e0c522264",
          "message": "refactor(l2): improve naming and standardize arguments in l2 tests (#3790)\n\n**Motivation**\n\nCurrently the L2 tests:\n* use unintuitive names (eth_client vs proposer_client, meaning l1 and\nl2)\n* do not have a consistent ordering of parameters\n* are inconsistent on when things (bridge address and rich private key)\nare given as parameter vs obtained from a function\n\n**Description**\n\nThis PR improves that, and gets the \"noisy\" changes out of the way for\nfurther improvements.\n\nThe rich private key was kept as a parameter to allow giving different\nones (in the future, this would allow parallelizing the tests). The\nbridge address now always uses the function, since it won't change in\nthe middle of the test.",
          "timestamp": "2025-07-24T14:31:19Z",
          "tree_id": "1923a7ff48eef83a37502db6250daa76a844c694",
          "url": "https://github.com/lambdaclass/ethrex/commit/8d7a9096401de0e6ff01c6e66e19513e0c522264"
        },
        "date": 1753370175014,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 166468929973,
            "range": "± 462604060",
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
          "id": "7c3fffcd507ef0deb49e61a45535d6e6db0366be",
          "message": "chore(l2): bump sp1 version to 5.0.8 (#3737)\n\n**Motivation**\n\nSome PRs that updated the Cargo.lock and bumped sp1 to 5.0.8 were\nfailing because sp1up was installing version 5.0.0.\n\n**Description**\n\n- Bump and lock all versions of sp1 to 5.0.8",
          "timestamp": "2025-07-24T14:58:11Z",
          "tree_id": "61796f6914bb141eccf48801f6433f68534c9961",
          "url": "https://github.com/lambdaclass/ethrex/commit/7c3fffcd507ef0deb49e61a45535d6e6db0366be"
        },
        "date": 1753371815258,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 168108905026,
            "range": "± 470740999",
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
          "distinct": false,
          "id": "7e6185d658f7b4f4871f56f044e39aa26528ab11",
          "message": "perf(levm): add shortcut for precompile calls (#3802)\n\n**Motivation**\n\nCurrently, calls to precompiles generate a callframe (including a stack\nand a new memory).\n\n**Description**\n\nAvoid creating call frames for precompiles.",
          "timestamp": "2025-07-24T15:36:42Z",
          "tree_id": "c1f806a73e4a7f1e7ef1c030fd1caee99ffb8a2c",
          "url": "https://github.com/lambdaclass/ethrex/commit/7e6185d658f7b4f4871f56f044e39aa26528ab11"
        },
        "date": 1753374128994,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 168179598139,
            "range": "± 1851720420",
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
          "id": "3a786b384cbbc2ffd66c613d2ae613cfc16c277c",
          "message": "perf(l2): avoid cloning all fields from ExecutionWitnessResult  (#3765)\n\n**Motivation**\n\nWhen proving a large batch performance was being affected because we\nwere cloning the entire ExecutionWitnessResult struct, this meant\ncloning all the BlockHeaders, Code and ChainConfig for every block.\n\n**Description**\n\n- Wrap ExecutionWitnessResult in a struct that has an inner field\nArc<Mutex<ExecutionWitnessResult>> that implements VmDatabase trait,\nwhich can be cheaply cloned\n- Remove all the Arc<Mutex<>> from ExecutionWitnessResult, remove the\nVmDatabase trait implementation, remove the derive Clone.\n\n**Perf Metrics**\n\ncommand:\n\n```\nTRACE_FILE=output.json TRACE_SAMPLE_RATE=100 RUST_BACKTRACE=full cargo run --release --features \"sp1,l2\" -- execute batch --rpc-url RPC_URL --network 65536300 13\n```\n\nspecs:\n```\nModel Identifier: MacBookAir10,1\nChip: Apple M1\nTotal Number of Cores: 8 (4 performance and 4 efficiency)\nMemory: 16 GB\nSystem Firmware Version: 11881.121.1\nOS Loader Version: 11881.121.1\n```\n\ncommits:\nThis branch (commit 16420ed)\nMain (commit ce5c47df7)\n\n- Time:\n    Main:    `Elapsed: 147.28s`\n    This branch:    `Elapsed: 67.07s`\n- Samply\n    Main: https://share.firefox.dev/3H2Hd5A\n    This branch: https://share.firefox.dev/40yeDzP",
          "timestamp": "2025-07-24T15:46:20Z",
          "tree_id": "0b98b16f157af9b0d0328bff1eea0720401c6c6e",
          "url": "https://github.com/lambdaclass/ethrex/commit/3a786b384cbbc2ffd66c613d2ae613cfc16c277c"
        },
        "date": 1753374640903,
        "tool": "cargo",
        "benches": [
          {
            "name": "Block import/Block import ERC20 transfers",
            "value": 167380057089,
            "range": "± 256642640",
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
        "date": 1751911368042,
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
          "id": "c63bbd7db56b60c495b03a675261db440d1ad7a2",
          "message": "feat(l1): archive sync (#3161)\n\n**Motivation**\nDownload the full state of a given block from an archive node. This will\nenable us to do full sync on mainnet starting from a post-merge block\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3115",
          "timestamp": "2025-07-07T16:37:29Z",
          "tree_id": "6e5c1ac8be1f20fd8a6f87389ab2d52287ed7e2f",
          "url": "https://github.com/lambdaclass/ethrex/commit/c63bbd7db56b60c495b03a675261db440d1ad7a2"
        },
        "date": 1751915002331,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006640457711442786,
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
          "id": "c63bbd7db56b60c495b03a675261db440d1ad7a2",
          "message": "feat(l1): archive sync (#3161)\n\n**Motivation**\nDownload the full state of a given block from an archive node. This will\nenable us to do full sync on mainnet starting from a post-merge block\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3115",
          "timestamp": "2025-07-07T16:37:29Z",
          "tree_id": "6e5c1ac8be1f20fd8a6f87389ab2d52287ed7e2f",
          "url": "https://github.com/lambdaclass/ethrex/commit/c63bbd7db56b60c495b03a675261db440d1ad7a2"
        },
        "date": 1751916510494,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012474130841121495,
            "unit": "Mgas/s"
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
          "id": "0637f3734e69a5c0fcdf1d972f7cebc0e55c04d5",
          "message": "ci(l2): make pr-main_l2_prover a required workflow (#3517)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-07T19:38:19Z",
          "tree_id": "2c235cd71be64af005ff264c24fdb0e2066757ff",
          "url": "https://github.com/lambdaclass/ethrex/commit/0637f3734e69a5c0fcdf1d972f7cebc0e55c04d5"
        },
        "date": 1751918731738,
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
          "id": "0637f3734e69a5c0fcdf1d972f7cebc0e55c04d5",
          "message": "ci(l2): make pr-main_l2_prover a required workflow (#3517)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-07T19:38:19Z",
          "tree_id": "2c235cd71be64af005ff264c24fdb0e2066757ff",
          "url": "https://github.com/lambdaclass/ethrex/commit/0637f3734e69a5c0fcdf1d972f7cebc0e55c04d5"
        },
        "date": 1751922602603,
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
          "id": "ed8e61f04e5bed2f3b496da710b6c4524f1b661d",
          "message": "fix(l1): metrics exporter dashboard total peer count panel (#3470)\n\n**Motivation**\n\nEthereum Metrics Exporter dashboard is showing no peers when running a\nsync, as described in the issue #3104.\n\n**Description**\n\nThis pr fixes the rpc call handler for net_peerCount as described\n[here](https://ethereum.org/en/developers/docs/apis/json-rpc/#net_peercount).\n\nIt also introduces a new function for `PeerHandler` to access the\nconnected peers so the rpc call handler can get the amount.\n\nHere you can see how the panel looks like now:\n<img width=\"1425\" alt=\"Screenshot 2025-07-03 at 13 29 57\"\nsrc=\"https://github.com/user-attachments/assets/89c699a8-72bb-4a42-918a-c9e3ea6d3036\"\n/>\n\nTo run this you can go to tooling/sync and run make\nstart_hoodi_metrics_docker, then go to http://localhost:3001/ to see the\npanels.\n\nCloses #3468\n\n---------\n\nCo-authored-by: fmoletta <99273364+fmoletta@users.noreply.github.com>",
          "timestamp": "2025-07-07T19:39:22Z",
          "tree_id": "2664be10e47fd8ab522ad28be25f8e3c412498a1",
          "url": "https://github.com/lambdaclass/ethrex/commit/ed8e61f04e5bed2f3b496da710b6c4524f1b661d"
        },
        "date": 1751925963308,
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
          "id": "ed8e61f04e5bed2f3b496da710b6c4524f1b661d",
          "message": "fix(l1): metrics exporter dashboard total peer count panel (#3470)\n\n**Motivation**\n\nEthereum Metrics Exporter dashboard is showing no peers when running a\nsync, as described in the issue #3104.\n\n**Description**\n\nThis pr fixes the rpc call handler for net_peerCount as described\n[here](https://ethereum.org/en/developers/docs/apis/json-rpc/#net_peercount).\n\nIt also introduces a new function for `PeerHandler` to access the\nconnected peers so the rpc call handler can get the amount.\n\nHere you can see how the panel looks like now:\n<img width=\"1425\" alt=\"Screenshot 2025-07-03 at 13 29 57\"\nsrc=\"https://github.com/user-attachments/assets/89c699a8-72bb-4a42-918a-c9e3ea6d3036\"\n/>\n\nTo run this you can go to tooling/sync and run make\nstart_hoodi_metrics_docker, then go to http://localhost:3001/ to see the\npanels.\n\nCloses #3468\n\n---------\n\nCo-authored-by: fmoletta <99273364+fmoletta@users.noreply.github.com>",
          "timestamp": "2025-07-07T19:39:22Z",
          "tree_id": "2664be10e47fd8ab522ad28be25f8e3c412498a1",
          "url": "https://github.com/lambdaclass/ethrex/commit/ed8e61f04e5bed2f3b496da710b6c4524f1b661d"
        },
        "date": 1751927411731,
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
          "id": "e141e1004a011bffd5d2f754c8d64c9efd770c8d",
          "message": "chore(l2): add ERC20 failed deposit integration test (#3547)\n\n**Motivation**\n\nWe want to ensure if a deposit fails, the funds won't be lost.\n\n**Description**\n\nAdds an integration test for ERC20 failed deposit turning into a\nwithdrawal.\n\nCloses #3990",
          "timestamp": "2025-07-08T15:28:54Z",
          "tree_id": "268aed2a136e9b2adf6d415b9a552fbcc01491bd",
          "url": "https://github.com/lambdaclass/ethrex/commit/e141e1004a011bffd5d2f754c8d64c9efd770c8d"
        },
        "date": 1751990183801,
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
          "id": "e141e1004a011bffd5d2f754c8d64c9efd770c8d",
          "message": "chore(l2): add ERC20 failed deposit integration test (#3547)\n\n**Motivation**\n\nWe want to ensure if a deposit fails, the funds won't be lost.\n\n**Description**\n\nAdds an integration test for ERC20 failed deposit turning into a\nwithdrawal.\n\nCloses #3990",
          "timestamp": "2025-07-08T15:28:54Z",
          "tree_id": "268aed2a136e9b2adf6d415b9a552fbcc01491bd",
          "url": "https://github.com/lambdaclass/ethrex/commit/e141e1004a011bffd5d2f754c8d64c9efd770c8d"
        },
        "date": 1751994546349,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012532694835680751,
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
          "id": "752c20b5552cceab1ed2959488929639a96a8661",
          "message": "fix(l1,l2): eth client send blobs when calling eth_estimateGas  (#3540)\n\n**Motivation**\n\nWhen calling eth_estimateGas to estimate the gas for the L2 commitment\nthe call was reverting because the blob was not included in the call\n\n**Description**\n\n- Add a function to add the blobs to a GenericTransaction\n- Add the field \"blobs\" to the request if the blobs field is not empty\n\n---------\n\nCo-authored-by: Tomás Paradelo <112426153+tomip01@users.noreply.github.com>",
          "timestamp": "2025-07-08T16:32:38Z",
          "tree_id": "ca06e38d7ee9df3793f6335ef93231ecdfdd30c3",
          "url": "https://github.com/lambdaclass/ethrex/commit/752c20b5552cceab1ed2959488929639a96a8661"
        },
        "date": 1752000558696,
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
          "id": "752c20b5552cceab1ed2959488929639a96a8661",
          "message": "fix(l1,l2): eth client send blobs when calling eth_estimateGas  (#3540)\n\n**Motivation**\n\nWhen calling eth_estimateGas to estimate the gas for the L2 commitment\nthe call was reverting because the blob was not included in the call\n\n**Description**\n\n- Add a function to add the blobs to a GenericTransaction\n- Add the field \"blobs\" to the request if the blobs field is not empty\n\n---------\n\nCo-authored-by: Tomás Paradelo <112426153+tomip01@users.noreply.github.com>",
          "timestamp": "2025-07-08T16:32:38Z",
          "tree_id": "ca06e38d7ee9df3793f6335ef93231ecdfdd30c3",
          "url": "https://github.com/lambdaclass/ethrex/commit/752c20b5552cceab1ed2959488929639a96a8661"
        },
        "date": 1752009228021,
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
          "id": "d1ceb86cb32f8949ecd3fc279084f6921c3e757f",
          "message": "fix(l1): ignore unknown protocols in capability exchange (#3543)\n\n**Motivation**\n\nFailing due to a peer having extra capabilities can make us lose\nexceptional peers. Hence, we want to ignore any extra capabilities they\nhave.\n\n**Description**\n\nThis PR changes `Capability.protocol` to be an 8-byte array instead of a\nstring, allowing us to store any string we receive.",
          "timestamp": "2025-07-08T17:12:21Z",
          "tree_id": "b73d081e492be3f36630c76381bba95066e5b585",
          "url": "https://github.com/lambdaclass/ethrex/commit/d1ceb86cb32f8949ecd3fc279084f6921c3e757f"
        },
        "date": 1752011731352,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012639507575757576,
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
          "id": "d1ceb86cb32f8949ecd3fc279084f6921c3e757f",
          "message": "fix(l1): ignore unknown protocols in capability exchange (#3543)\n\n**Motivation**\n\nFailing due to a peer having extra capabilities can make us lose\nexceptional peers. Hence, we want to ignore any extra capabilities they\nhave.\n\n**Description**\n\nThis PR changes `Capability.protocol` to be an 8-byte array instead of a\nstring, allowing us to store any string we receive.",
          "timestamp": "2025-07-08T17:12:21Z",
          "tree_id": "b73d081e492be3f36630c76381bba95066e5b585",
          "url": "https://github.com/lambdaclass/ethrex/commit/d1ceb86cb32f8949ecd3fc279084f6921c3e757f"
        },
        "date": 1752012569899,
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
          "id": "715c2bbe2c6d139bb938ea87c6aa1a07ade060d6",
          "message": "refactor(levm): change returned error types to `InternalError` (#3322)\n\n**Motivation**\nFrom [#3063](https://github.com/lambdaclass/ethrex/issues/3063)\n\n> There are various cases in which we return an error with the\nExceptionalHalt type but they actually are InternalErrors, things that\nshouldn't ever happen and if they happen they should break.\nThis is not a critical issue since if the VM is working fine then it\nwon't ever enter to those cases, but it would be more precise if we\ncatalogued those errors as internals instead of saying that they revert\nexecution when they don't.\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\nIntroduces the following changes:\n* Replaces `PrecompileError` with `InternalError` in those cases in\nwhich an error is returned even though is not possible for the\ninstruction to fail, typically when slicing bytes whose size have been\nalready checked.\n* Removes the error types `EvaluationError` and `DefaultError` (which\nwere quite generic) from `PrecompileError` and adds specific and more\ndescriptive error types instead (`InvalidPoint`, `PointNotInTheCurve`,\netc).\n* Removes the `PrecompileError::GasConsumedOverflow` error type.\n\n\n\nCloses #3063\n\n---------\n\nCo-authored-by: Jeremías Salomón <48994069+JereSalo@users.noreply.github.com>\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-07-08T17:26:31Z",
          "tree_id": "6b8f7bd82899863be607fc39c39df00c6ebac941",
          "url": "https://github.com/lambdaclass/ethrex/commit/715c2bbe2c6d139bb938ea87c6aa1a07ade060d6"
        },
        "date": 1752013414280,
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
          "id": "715c2bbe2c6d139bb938ea87c6aa1a07ade060d6",
          "message": "refactor(levm): change returned error types to `InternalError` (#3322)\n\n**Motivation**\nFrom [#3063](https://github.com/lambdaclass/ethrex/issues/3063)\n\n> There are various cases in which we return an error with the\nExceptionalHalt type but they actually are InternalErrors, things that\nshouldn't ever happen and if they happen they should break.\nThis is not a critical issue since if the VM is working fine then it\nwon't ever enter to those cases, but it would be more precise if we\ncatalogued those errors as internals instead of saying that they revert\nexecution when they don't.\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\nIntroduces the following changes:\n* Replaces `PrecompileError` with `InternalError` in those cases in\nwhich an error is returned even though is not possible for the\ninstruction to fail, typically when slicing bytes whose size have been\nalready checked.\n* Removes the error types `EvaluationError` and `DefaultError` (which\nwere quite generic) from `PrecompileError` and adds specific and more\ndescriptive error types instead (`InvalidPoint`, `PointNotInTheCurve`,\netc).\n* Removes the `PrecompileError::GasConsumedOverflow` error type.\n\n\n\nCloses #3063\n\n---------\n\nCo-authored-by: Jeremías Salomón <48994069+JereSalo@users.noreply.github.com>\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-07-08T17:26:31Z",
          "tree_id": "6b8f7bd82899863be607fc39c39df00c6ebac941",
          "url": "https://github.com/lambdaclass/ethrex/commit/715c2bbe2c6d139bb938ea87c6aa1a07ade060d6"
        },
        "date": 1752018324519,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012556274694261525,
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
          "id": "53546f4e280e333ad80df31355bd1fc887991d10",
          "message": "fix(l1): abort `show_state_sync_progress` task  (#3406)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nThe `show_state_sync_progress` task used to run until all\n`state_sync_segment` tasks had signaled their conclusion via\n`end_segment` method. This could cause the task to hand indeterminately\nif one of the tasks failed. This PR aims to fix this by removing the\nresponsibility of signaling their end from `state_sync_segment` and\ninstead have `state_sync` method (the one that launched both\n`show_state_sync_progress` & the `state_sync_segment` tasks) be the one\nto end the `show_state_sync_progress` task via an abort\n**Description**\n* Remove method `StateSyncProgress::end_segment` & associated field\n* `show_state_sync_progress` is now an endless task\n* `state_sync` now aborts `show_state_sync_progress` when no longer\nneeded instead of waiting for it to finish\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-07-08T18:11:03Z",
          "tree_id": "db56a77d681fdf0503b3de056d2867e86bc95061",
          "url": "https://github.com/lambdaclass/ethrex/commit/53546f4e280e333ad80df31355bd1fc887991d10"
        },
        "date": 1752021796597,
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
          "id": "53546f4e280e333ad80df31355bd1fc887991d10",
          "message": "fix(l1): abort `show_state_sync_progress` task  (#3406)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nThe `show_state_sync_progress` task used to run until all\n`state_sync_segment` tasks had signaled their conclusion via\n`end_segment` method. This could cause the task to hand indeterminately\nif one of the tasks failed. This PR aims to fix this by removing the\nresponsibility of signaling their end from `state_sync_segment` and\ninstead have `state_sync` method (the one that launched both\n`show_state_sync_progress` & the `state_sync_segment` tasks) be the one\nto end the `show_state_sync_progress` task via an abort\n**Description**\n* Remove method `StateSyncProgress::end_segment` & associated field\n* `show_state_sync_progress` is now an endless task\n* `state_sync` now aborts `show_state_sync_progress` when no longer\nneeded instead of waiting for it to finish\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-07-08T18:11:03Z",
          "tree_id": "db56a77d681fdf0503b3de056d2867e86bc95061",
          "url": "https://github.com/lambdaclass/ethrex/commit/53546f4e280e333ad80df31355bd1fc887991d10"
        },
        "date": 1752026914291,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012532694835680751,
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
          "id": "c15c01ae92a2f736614192f65d1884539d9e6ed5",
          "message": "ci(l1,l2): remove `core` scope and improve PR labeling workflow (#3561)\n\n**Motivation**\n\n- Declutter `ethrex_l1` project and remove ambiguous `core` scope in\ntitle.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- Remove core scope because it is ambiguous. Replace it with `l1,l2`\n- Merge `pr_author.yaml` and `pr_label.yaml` into one file\n`pr_github_metadata.yaml`\n- Change rules of labeling because of preferences in our projects:\n- `ethrex_performance`: Will have PRs that have `perf` at the beginning\nof the title.\n  - `ethrex_l2`: Will have any PR that has in the title scope `l2`\n- `ethrex_l1`: Will have PRs that haven't been assigned to\n`ethrex_performance` or `ethrex_l2` that have `l1` or `levm` in their\ntitle.\n\nThe decisions were made according to the preferences of each team.\n`ethrex_l2` project will have anything that has to do with the L2\n`ethrex_l1` project will have things that touch only l1 stuff and\nnothing else so that we assure they truly belong to this project. Some\nPRs will be filtered out and will have to be added manually, but we\nprefer that rather over the clutter of having more PRs than necessary.\n\nCloses #3565",
          "timestamp": "2025-07-08T18:18:32Z",
          "tree_id": "40bae128432e6ab09b6688b548bf5e4f8870f8aa",
          "url": "https://github.com/lambdaclass/ethrex/commit/c15c01ae92a2f736614192f65d1884539d9e6ed5"
        },
        "date": 1752027750227,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006640457711442786,
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
          "id": "c15c01ae92a2f736614192f65d1884539d9e6ed5",
          "message": "ci(l1,l2): remove `core` scope and improve PR labeling workflow (#3561)\n\n**Motivation**\n\n- Declutter `ethrex_l1` project and remove ambiguous `core` scope in\ntitle.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- Remove core scope because it is ambiguous. Replace it with `l1,l2`\n- Merge `pr_author.yaml` and `pr_label.yaml` into one file\n`pr_github_metadata.yaml`\n- Change rules of labeling because of preferences in our projects:\n- `ethrex_performance`: Will have PRs that have `perf` at the beginning\nof the title.\n  - `ethrex_l2`: Will have any PR that has in the title scope `l2`\n- `ethrex_l1`: Will have PRs that haven't been assigned to\n`ethrex_performance` or `ethrex_l2` that have `l1` or `levm` in their\ntitle.\n\nThe decisions were made according to the preferences of each team.\n`ethrex_l2` project will have anything that has to do with the L2\n`ethrex_l1` project will have things that touch only l1 stuff and\nnothing else so that we assure they truly belong to this project. Some\nPRs will be filtered out and will have to be added manually, but we\nprefer that rather over the clutter of having more PRs than necessary.\n\nCloses #3565",
          "timestamp": "2025-07-08T18:18:32Z",
          "tree_id": "40bae128432e6ab09b6688b548bf5e4f8870f8aa",
          "url": "https://github.com/lambdaclass/ethrex/commit/c15c01ae92a2f736614192f65d1884539d9e6ed5"
        },
        "date": 1752032621800,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012651488151658769,
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
          "id": "5a14d806d0c84aef0266de503cbd451cab599d8b",
          "message": "feat(l2): add L1From field to privileged transaction events (#3477)\n\n**Motivation**\n\nAs described on #3452, it is convenient for client applications to be\nable to search their sent privileged transactions.\n\n**Description**\n\nThis PR drops indexing from all PrivilegedTxSent fields and adds an\nindexed L1From member.\n \nCloses #3452\n\n---------\n\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-07-08T19:04:41Z",
          "tree_id": "54d832d221764ef90884589a6bd5db81bd0fed13",
          "url": "https://github.com/lambdaclass/ethrex/commit/5a14d806d0c84aef0266de503cbd451cab599d8b"
        },
        "date": 1752033449239,
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
          "id": "5a14d806d0c84aef0266de503cbd451cab599d8b",
          "message": "feat(l2): add L1From field to privileged transaction events (#3477)\n\n**Motivation**\n\nAs described on #3452, it is convenient for client applications to be\nable to search their sent privileged transactions.\n\n**Description**\n\nThis PR drops indexing from all PrivilegedTxSent fields and adds an\nindexed L1From member.\n \nCloses #3452\n\n---------\n\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>",
          "timestamp": "2025-07-08T19:04:41Z",
          "tree_id": "54d832d221764ef90884589a6bd5db81bd0fed13",
          "url": "https://github.com/lambdaclass/ethrex/commit/5a14d806d0c84aef0266de503cbd451cab599d8b"
        },
        "date": 1752035911012,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012603701605288008,
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
          "id": "f07af68346980f2762f0a71cf0de7ba87c49642b",
          "message": "fix(l2): use github token to avoid rate limit (#3570)\n\n**Motivation**\n\nOur CI is failing at the `Install solc` step in almost all jobs due to a\n`rate limit` error.\n\n**Description**\n\nAuthenticates using a GitHub token to bypass the rate limit.\n\nCloses None",
          "timestamp": "2025-07-08T21:31:57Z",
          "tree_id": "9d743d53d18ef3e1cedc95540f53d0513e1d1176",
          "url": "https://github.com/lambdaclass/ethrex/commit/f07af68346980f2762f0a71cf0de7ba87c49642b"
        },
        "date": 1752038381463,
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
          "distinct": true,
          "id": "f07af68346980f2762f0a71cf0de7ba87c49642b",
          "message": "fix(l2): use github token to avoid rate limit (#3570)\n\n**Motivation**\n\nOur CI is failing at the `Install solc` step in almost all jobs due to a\n`rate limit` error.\n\n**Description**\n\nAuthenticates using a GitHub token to bypass the rate limit.\n\nCloses None",
          "timestamp": "2025-07-08T21:31:57Z",
          "tree_id": "9d743d53d18ef3e1cedc95540f53d0513e1d1176",
          "url": "https://github.com/lambdaclass/ethrex/commit/f07af68346980f2762f0a71cf0de7ba87c49642b"
        },
        "date": 1752039192707,
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
          "id": "8aa44c11650df469a2a89d215c9791da67403a4b",
          "message": "ci(l1): comment flaky devp2p test BasicFindnode (#3542)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nThis test fails very occasionally, here are a few runs in which it\nhappened:\n-\nhttps://github.com/lambdaclass/ethrex/actions/runs/16125250426/job/45501078767\n-\nhttps://github.com/lambdaclass/ethrex/actions/runs/16126040468/job/45503603345\n-\nhttps://github.com/lambdaclass/ethrex/actions/runs/16120603086/job/45485976155\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3549",
          "timestamp": "2025-07-08T23:18:40Z",
          "tree_id": "61b413b67618e34b836f1dc72f2729db6fd4c0da",
          "url": "https://github.com/lambdaclass/ethrex/commit/8aa44c11650df469a2a89d215c9791da67403a4b"
        },
        "date": 1752041668897,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012544473684210527,
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
          "id": "8aa44c11650df469a2a89d215c9791da67403a4b",
          "message": "ci(l1): comment flaky devp2p test BasicFindnode (#3542)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nThis test fails very occasionally, here are a few runs in which it\nhappened:\n-\nhttps://github.com/lambdaclass/ethrex/actions/runs/16125250426/job/45501078767\n-\nhttps://github.com/lambdaclass/ethrex/actions/runs/16126040468/job/45503603345\n-\nhttps://github.com/lambdaclass/ethrex/actions/runs/16120603086/job/45485976155\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3549",
          "timestamp": "2025-07-08T23:18:40Z",
          "tree_id": "61b413b67618e34b836f1dc72f2729db6fd4c0da",
          "url": "https://github.com/lambdaclass/ethrex/commit/8aa44c11650df469a2a89d215c9791da67403a4b"
        },
        "date": 1752042496628,
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
          "id": "09dd2a27634849d96d500da4042781d1d4596a12",
          "message": "fix(l1): metrics exporter sync status, percent, distance and rate panels (#3456)\n\n**Motivation**\n\nEthereum Metrics Exporter is showing incorrect data for the sync status,\nsync percent, sync distance and sync rate panels when running a sync, as\ndescribed in the issue #3104.\n\n**Description**\n\nThis pr fixes the rpc call handler for eth_syncing as described\n[here](https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_syncing).\n\nHere you can see how the panels look like up to now:\n<img width=\"1429\" alt=\"Screenshot 2025-07-03 at 11 25 57\"\nsrc=\"https://github.com/user-attachments/assets/22646c5d-1ab8-4687-be66-56d2d8eb3fc3\"\n/>\n\nTo run this you can go to tooling/sync and run `make\nstart_hoodi_metrics_docker`, then go to http://localhost:3001/ to see\nthe panels.\n\nCloses #3325 and closes #3455",
          "timestamp": "2025-07-09T11:19:46Z",
          "tree_id": "4ca1ad3f11fa2a52a207afc9fa3c31e230ff2891",
          "url": "https://github.com/lambdaclass/ethrex/commit/09dd2a27634849d96d500da4042781d1d4596a12"
        },
        "date": 1752061642710,
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
          "id": "09dd2a27634849d96d500da4042781d1d4596a12",
          "message": "fix(l1): metrics exporter sync status, percent, distance and rate panels (#3456)\n\n**Motivation**\n\nEthereum Metrics Exporter is showing incorrect data for the sync status,\nsync percent, sync distance and sync rate panels when running a sync, as\ndescribed in the issue #3104.\n\n**Description**\n\nThis pr fixes the rpc call handler for eth_syncing as described\n[here](https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_syncing).\n\nHere you can see how the panels look like up to now:\n<img width=\"1429\" alt=\"Screenshot 2025-07-03 at 11 25 57\"\nsrc=\"https://github.com/user-attachments/assets/22646c5d-1ab8-4687-be66-56d2d8eb3fc3\"\n/>\n\nTo run this you can go to tooling/sync and run `make\nstart_hoodi_metrics_docker`, then go to http://localhost:3001/ to see\nthe panels.\n\nCloses #3325 and closes #3455",
          "timestamp": "2025-07-09T11:19:46Z",
          "tree_id": "4ca1ad3f11fa2a52a207afc9fa3c31e230ff2891",
          "url": "https://github.com/lambdaclass/ethrex/commit/09dd2a27634849d96d500da4042781d1d4596a12"
        },
        "date": 1752064149502,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012485799812909262,
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
          "id": "d454a1b2940492bb4d43e1643f2ec8c97f276e46",
          "message": "perf(levm): improve sstore (#3555)\n\n**Motivation**\n\nLocally the sstore bench from\nhttps://github.com/lambdaclass/ethrex/pull/3552 goes from 2x worse to a\nbit better than revm\n\nGas benchmarks improve 2x\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-09T16:25:59Z",
          "tree_id": "36ea0d7b6740d61c8fd225e9f8c4abb054ad1e83",
          "url": "https://github.com/lambdaclass/ethrex/commit/d454a1b2940492bb4d43e1643f2ec8c97f276e46"
        },
        "date": 1752082860574,
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
          "id": "d454a1b2940492bb4d43e1643f2ec8c97f276e46",
          "message": "perf(levm): improve sstore (#3555)\n\n**Motivation**\n\nLocally the sstore bench from\nhttps://github.com/lambdaclass/ethrex/pull/3552 goes from 2x worse to a\nbit better than revm\n\nGas benchmarks improve 2x\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-09T16:25:59Z",
          "tree_id": "36ea0d7b6740d61c8fd225e9f8c4abb054ad1e83",
          "url": "https://github.com/lambdaclass/ethrex/commit/d454a1b2940492bb4d43e1643f2ec8c97f276e46"
        },
        "date": 1752085363649,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001240457249070632,
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
          "id": "398a10878145cbb6e1657e2360dc24a0518fbee6",
          "message": "ci(l2): use correct toolchain in nix build (#3507)\n\n**Motivation**\n\nCurrently the rust version is the one in nixpkgs, which might not follow\nour upgrades.\n\n**Description**\n\nChange the build to rely on the toolchain file on the project root.\n\n---------\n\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>\nCo-authored-by: avilagaston9 <gaston.avila@lambdaclass.com>",
          "timestamp": "2025-07-10T13:50:44Z",
          "tree_id": "f5011011b112a406ce0326b0800a05603db9ca48",
          "url": "https://github.com/lambdaclass/ethrex/commit/398a10878145cbb6e1657e2360dc24a0518fbee6"
        },
        "date": 1752160208327,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006640457711442786,
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
          "id": "398a10878145cbb6e1657e2360dc24a0518fbee6",
          "message": "ci(l2): use correct toolchain in nix build (#3507)\n\n**Motivation**\n\nCurrently the rust version is the one in nixpkgs, which might not follow\nour upgrades.\n\n**Description**\n\nChange the build to rely on the toolchain file on the project root.\n\n---------\n\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>\nCo-authored-by: avilagaston9 <gaston.avila@lambdaclass.com>",
          "timestamp": "2025-07-10T13:50:44Z",
          "tree_id": "f5011011b112a406ce0326b0800a05603db9ca48",
          "url": "https://github.com/lambdaclass/ethrex/commit/398a10878145cbb6e1657e2360dc24a0518fbee6"
        },
        "date": 1752162714847,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012347197039777984,
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
          "id": "d396ca4b52b5ea3c69fd62a1887ada672c6930ef",
          "message": "fix(l2): avoid proving already proved batch (#3588)\n\n**Motivation**\nAvoid this situation:\n- Prover finishes proving batch n\n- Prover asks for batch to prove gets batch n again because:\n`let batch_to_verify = 1 + get_latest_sent_batch()` is still n because\nthe proof_sender dind't send the verification tx yet.\n- Verifier verifies batch n + 1\n- Prover is still proving batch n when it could start proving batch n +\n1\n\n\n**Description**\n\n- Before sending a new batch to prove check if we already have all\nneeded proofs for that batch stored in the DB in case we do send and\nempty response\n\nCloses #3545",
          "timestamp": "2025-07-10T15:51:44Z",
          "tree_id": "f17356b28dc850006c1b9694ba271d6e1128893c",
          "url": "https://github.com/lambdaclass/ethrex/commit/d396ca4b52b5ea3c69fd62a1887ada672c6930ef"
        },
        "date": 1752166020997,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012416111627906977,
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
          "id": "d396ca4b52b5ea3c69fd62a1887ada672c6930ef",
          "message": "fix(l2): avoid proving already proved batch (#3588)\n\n**Motivation**\nAvoid this situation:\n- Prover finishes proving batch n\n- Prover asks for batch to prove gets batch n again because:\n`let batch_to_verify = 1 + get_latest_sent_batch()` is still n because\nthe proof_sender dind't send the verification tx yet.\n- Verifier verifies batch n + 1\n- Prover is still proving batch n when it could start proving batch n +\n1\n\n\n**Description**\n\n- Before sending a new batch to prove check if we already have all\nneeded proofs for that batch stored in the DB in case we do send and\nempty response\n\nCloses #3545",
          "timestamp": "2025-07-10T15:51:44Z",
          "tree_id": "f17356b28dc850006c1b9694ba271d6e1128893c",
          "url": "https://github.com/lambdaclass/ethrex/commit/d396ca4b52b5ea3c69fd62a1887ada672c6930ef"
        },
        "date": 1752166870330,
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
          "id": "8dac7cb1d7d71ccff299f6c9888444bc56846fdd",
          "message": "fix(l2): seal a batch in a single DB transaction  (#3554)\n\n**Motivation**\n\nWhen deploying ethrex L2 some errors came up that are related to the\nseal_batch process not being done in a single DB transaction.\n\n**Description**\n\n- Move seal_batch to the `StoreEngineRollup` trait\n- For sql rollup store engine\n- Wrap all the DB write functions from the trait with a <name>_in_tx\nthat gets as an input an Option<Transaction> in case the transaction is\nSome then it uses the existing transaction, and does not commit. If its\nNone it creates a new transaction and commits at the end of the\nfunction.\n- Modify the `SQLStore` struct to hold two instances of `Connection` one\nfor reads and one for writes, the write connection is protected by a\nMutex to enforce a maximum of 1 to prevent this error:\n      ```\nfailed because of a rollup store error: Limbo Query error: SQLite\nfailure: `cannot start a transaction within a transaction`\n      ``` \n- Use `PRAGMA journal_mode=WAL` for [better\nconcurrency](https://sqlite.org/wal.html#concurrency)\n- For `libmdbx` , `redb` and `in-memory`\n   - Implement the `seal_batch` function \n- Refactor: remove all the functions that were exposed by `store.rs` and\nwere only part of seal_batch to prevent its usage outside of batch\nsealing.\n\n\nCloses #3546",
          "timestamp": "2025-07-10T16:09:07Z",
          "tree_id": "b7b1d653a7447a46b3c6a30eae3762bc6c4962d7",
          "url": "https://github.com/lambdaclass/ethrex/commit/8dac7cb1d7d71ccff299f6c9888444bc56846fdd"
        },
        "date": 1752174912717,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012591811320754717,
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
          "id": "8dac7cb1d7d71ccff299f6c9888444bc56846fdd",
          "message": "fix(l2): seal a batch in a single DB transaction  (#3554)\n\n**Motivation**\n\nWhen deploying ethrex L2 some errors came up that are related to the\nseal_batch process not being done in a single DB transaction.\n\n**Description**\n\n- Move seal_batch to the `StoreEngineRollup` trait\n- For sql rollup store engine\n- Wrap all the DB write functions from the trait with a <name>_in_tx\nthat gets as an input an Option<Transaction> in case the transaction is\nSome then it uses the existing transaction, and does not commit. If its\nNone it creates a new transaction and commits at the end of the\nfunction.\n- Modify the `SQLStore` struct to hold two instances of `Connection` one\nfor reads and one for writes, the write connection is protected by a\nMutex to enforce a maximum of 1 to prevent this error:\n      ```\nfailed because of a rollup store error: Limbo Query error: SQLite\nfailure: `cannot start a transaction within a transaction`\n      ``` \n- Use `PRAGMA journal_mode=WAL` for [better\nconcurrency](https://sqlite.org/wal.html#concurrency)\n- For `libmdbx` , `redb` and `in-memory`\n   - Implement the `seal_batch` function \n- Refactor: remove all the functions that were exposed by `store.rs` and\nwere only part of seal_batch to prevent its usage outside of batch\nsealing.\n\n\nCloses #3546",
          "timestamp": "2025-07-10T16:09:07Z",
          "tree_id": "b7b1d653a7447a46b3c6a30eae3762bc6c4962d7",
          "url": "https://github.com/lambdaclass/ethrex/commit/8dac7cb1d7d71ccff299f6c9888444bc56846fdd"
        },
        "date": 1752175753096,
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
          "id": "dcb3c9cf5cc3072eddc35f1f2640d1a66baad894",
          "message": "perf(levm): improve blake2f  (#3503)\n\n**Motivation**\n\nCleaner code and better perfomance\n\n<!-- Why does this pull request exist? What are its goals? -->\n\nMain\n\n![image](https://github.com/user-attachments/assets/1112c9dc-7257-4c7f-a8ae-b26cc1190894)\n\npr\n\n![image](https://github.com/user-attachments/assets/7cbdbe56-98d6-41ce-bc6a-11ad18a31208)\n\n\nImproves blake2f 1 round mgas\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-10T16:31:46Z",
          "tree_id": "e34318e84d26a13bd37d346390e93cc12cae7640",
          "url": "https://github.com/lambdaclass/ethrex/commit/dcb3c9cf5cc3072eddc35f1f2640d1a66baad894"
        },
        "date": 1752176591310,
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
          "id": "dcb3c9cf5cc3072eddc35f1f2640d1a66baad894",
          "message": "perf(levm): improve blake2f  (#3503)\n\n**Motivation**\n\nCleaner code and better perfomance\n\n<!-- Why does this pull request exist? What are its goals? -->\n\nMain\n\n![image](https://github.com/user-attachments/assets/1112c9dc-7257-4c7f-a8ae-b26cc1190894)\n\npr\n\n![image](https://github.com/user-attachments/assets/7cbdbe56-98d6-41ce-bc6a-11ad18a31208)\n\n\nImproves blake2f 1 round mgas\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-10T16:31:46Z",
          "tree_id": "e34318e84d26a13bd37d346390e93cc12cae7640",
          "url": "https://github.com/lambdaclass/ethrex/commit/dcb3c9cf5cc3072eddc35f1f2640d1a66baad894"
        },
        "date": 1752181840686,
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
          "id": "187e8c27f9b9a22948cd82b0b3f79866c16ac489",
          "message": "chore(l2): add forced withdrawal integration test (#3541)\n\n**Motivation**\n\nWe want an integration test for forced withdrawals\n\n**Description**\n\nWithdraws through a privileged transaction.\n\nCloses #3394",
          "timestamp": "2025-07-10T18:10:57Z",
          "tree_id": "486f18735fda83de70f48ba4f654780e8515f3d9",
          "url": "https://github.com/lambdaclass/ethrex/commit/187e8c27f9b9a22948cd82b0b3f79866c16ac489"
        },
        "date": 1752187118347,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012544473684210527,
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
          "id": "187e8c27f9b9a22948cd82b0b3f79866c16ac489",
          "message": "chore(l2): add forced withdrawal integration test (#3541)\n\n**Motivation**\n\nWe want an integration test for forced withdrawals\n\n**Description**\n\nWithdraws through a privileged transaction.\n\nCloses #3394",
          "timestamp": "2025-07-10T18:10:57Z",
          "tree_id": "486f18735fda83de70f48ba4f654780e8515f3d9",
          "url": "https://github.com/lambdaclass/ethrex/commit/187e8c27f9b9a22948cd82b0b3f79866c16ac489"
        },
        "date": 1752187948364,
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
          "id": "9dab7c08cb8dbc86a5ae90d38faf2fc2d2c98064",
          "message": "feat(l2): monitor for ethrex L2 (#3410)\n\n**Description**\n\nThis PR introduces de ethrex monitor. A currently optional tool for node\noperators to monitor the L2 state.\n\nThe node can be monitored in two different tabs, the Overview tab and\nthe Logs tab. Both tabs have a help text line at the bottom to let know\nthe user how to interact with the current tab.\n\nThe Overview tab is composed of:\n- An ASCII ethrex logo.\n- A node status widget\n- A general chain status widget, which lists:\n    - Current batch (the batch being built by the Sequencer).\n    - Current block (the block being built by the Sequencer).\n    - Last committed batch.\n    - Last committed block.\n    - Last verified batch.\n    - Last verified block.\n- An L2 batches widget, which lists the last 50 L2 batches and their\ncurrent status, highlighting:\n    - L2 batch number.\n    - Number of blocks in the batch.\n    - Number of L2 to L1 messages in the batch.\n    - Commit tx hash (if committed).\n    - Verify tx hash (if verified).\n- An L2 blocks widget, which lists the last 50 L2 blocks, highlighting:\n    - L2 block number.\n    - Number of txs in the block.\n    - L2 block hash.\n    - L2 block coinbase (probably more relevant in based rollups).\n    - Gas consumed.\n    - Blob gas consumed.\n    - Size of the block. \n- A mempool widget, which lists the current 50 txs in the memool,\nhighlighting:\n    - Tx type (e.g. EIP1559, Privilege, etc).\n    - Tx hash.\n    - Tx sender.\n    - Tx nonce.\n- An L1 to L2 messages widget, which lists the last 50 L1 to L2 msgs and\ntheir status, highlighting:\n    - Message kind (e.g. deposit, message, etc).\n    - Message status (e.g. Processed on L2, etc).\n    - Message L1 tx hash.\n    - Message L2 tx hash\n    - Value\n- An L2 to L1 messages widget, which lists the last 50 L2 to L1 msgs and\ntheir status, highlighting:\n    - Message kind (e.g. withdrawal, message, etc).\n    - Message status (e.g. initiated, claimed, sent, delivered).\n    - Receiver on L1.\n    - Token L1 (if ERC20 withdrawal).\n    - Token L2 (if ERC20 withdrawal).\n    - L2 tx hash\n    - Value\n\nThe Logs tab shows the logs altogether or by crate. The log level could\nalso be adjusted in runtime.\n\n> [!NOTE]\n> 1. This feature is introduced as optional for now given its initial\nstate. Once mature enough, it will be default for operators.\n> 2. This initial version has some minor known flaws, but they were\nskipped in this PR on purpose:\n>     - #3512 .\n>     - #3513.\n>     - #3514.\n>     - #3515.\n>     - #3516.\n>     - No optimizations were done.\n\n**How to test**\n\n1. Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n2. Run a Sequencer (I suggest `make restart` in `crates/l2`).\n3. Run the prover with `make init-prover` in `crates/l2`.\n4. Run `make test` in `crates/l2`.\n\n**Showcase**\n\n*Overview*\n\n<img width=\"1512\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/0431b1f3-1a8f-49cf-9519-413ea3d3ed1a\"\n/>\n\n*Logs*\n\n<img width=\"1512\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/e0e6cdd7-1f8d-4278-8619-475cfaa14d4b\"\n/>",
          "timestamp": "2025-07-10T18:51:42Z",
          "tree_id": "e9c5ec2c406ad35b66a6b0943014497ccfe76e3b",
          "url": "https://github.com/lambdaclass/ethrex/commit/9dab7c08cb8dbc86a5ae90d38faf2fc2d2c98064"
        },
        "date": 1752189844294,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006295905660377359,
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
          "id": "9dab7c08cb8dbc86a5ae90d38faf2fc2d2c98064",
          "message": "feat(l2): monitor for ethrex L2 (#3410)\n\n**Description**\n\nThis PR introduces de ethrex monitor. A currently optional tool for node\noperators to monitor the L2 state.\n\nThe node can be monitored in two different tabs, the Overview tab and\nthe Logs tab. Both tabs have a help text line at the bottom to let know\nthe user how to interact with the current tab.\n\nThe Overview tab is composed of:\n- An ASCII ethrex logo.\n- A node status widget\n- A general chain status widget, which lists:\n    - Current batch (the batch being built by the Sequencer).\n    - Current block (the block being built by the Sequencer).\n    - Last committed batch.\n    - Last committed block.\n    - Last verified batch.\n    - Last verified block.\n- An L2 batches widget, which lists the last 50 L2 batches and their\ncurrent status, highlighting:\n    - L2 batch number.\n    - Number of blocks in the batch.\n    - Number of L2 to L1 messages in the batch.\n    - Commit tx hash (if committed).\n    - Verify tx hash (if verified).\n- An L2 blocks widget, which lists the last 50 L2 blocks, highlighting:\n    - L2 block number.\n    - Number of txs in the block.\n    - L2 block hash.\n    - L2 block coinbase (probably more relevant in based rollups).\n    - Gas consumed.\n    - Blob gas consumed.\n    - Size of the block. \n- A mempool widget, which lists the current 50 txs in the memool,\nhighlighting:\n    - Tx type (e.g. EIP1559, Privilege, etc).\n    - Tx hash.\n    - Tx sender.\n    - Tx nonce.\n- An L1 to L2 messages widget, which lists the last 50 L1 to L2 msgs and\ntheir status, highlighting:\n    - Message kind (e.g. deposit, message, etc).\n    - Message status (e.g. Processed on L2, etc).\n    - Message L1 tx hash.\n    - Message L2 tx hash\n    - Value\n- An L2 to L1 messages widget, which lists the last 50 L2 to L1 msgs and\ntheir status, highlighting:\n    - Message kind (e.g. withdrawal, message, etc).\n    - Message status (e.g. initiated, claimed, sent, delivered).\n    - Receiver on L1.\n    - Token L1 (if ERC20 withdrawal).\n    - Token L2 (if ERC20 withdrawal).\n    - L2 tx hash\n    - Value\n\nThe Logs tab shows the logs altogether or by crate. The log level could\nalso be adjusted in runtime.\n\n> [!NOTE]\n> 1. This feature is introduced as optional for now given its initial\nstate. Once mature enough, it will be default for operators.\n> 2. This initial version has some minor known flaws, but they were\nskipped in this PR on purpose:\n>     - #3512 .\n>     - #3513.\n>     - #3514.\n>     - #3515.\n>     - #3516.\n>     - No optimizations were done.\n\n**How to test**\n\n1. Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n2. Run a Sequencer (I suggest `make restart` in `crates/l2`).\n3. Run the prover with `make init-prover` in `crates/l2`.\n4. Run `make test` in `crates/l2`.\n\n**Showcase**\n\n*Overview*\n\n<img width=\"1512\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/0431b1f3-1a8f-49cf-9519-413ea3d3ed1a\"\n/>\n\n*Logs*\n\n<img width=\"1512\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/e0e6cdd7-1f8d-4278-8619-475cfaa14d4b\"\n/>",
          "timestamp": "2025-07-10T18:51:42Z",
          "tree_id": "e9c5ec2c406ad35b66a6b0943014497ccfe76e3b",
          "url": "https://github.com/lambdaclass/ethrex/commit/9dab7c08cb8dbc86a5ae90d38faf2fc2d2c98064"
        },
        "date": 1752192334614,
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
          "id": "f466fb8216f85442d763a8ed6a10a36f05e8c93f",
          "message": "feat(l2): proxied l2 system contracts (#3421)\n\n**Motivation**\n\nWe want to be able to upgrade L2 system contracts.\n\n**Description**\n\nThis makes it so that the L2 contracts themselves are proxies. Their\ninitial implementations are kept in the genesis for ease of deployment\nand to avoid keeping them empty in the first blocks.\n\nSince the proxies need to be embedded in the genesis, they can't be\ndeployed with a constructor, so their\n[ERC-1967](https://eips.ethereum.org/EIPS/eip-1967) slots are set\ndirectly.\n\nA function is added to the L1 CommonBridge to allow upgrading the L2\ncontracts. A special address (0xf000) is used to authenticate the\nupgrade.\n\nCloses #3345",
          "timestamp": "2025-07-11T12:27:27Z",
          "tree_id": "5da2a6cdd7e6ca4748dd0f07fac5768e8cfe3540",
          "url": "https://github.com/lambdaclass/ethrex/commit/f466fb8216f85442d763a8ed6a10a36f05e8c93f"
        },
        "date": 1752238570404,
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
          "id": "f466fb8216f85442d763a8ed6a10a36f05e8c93f",
          "message": "feat(l2): proxied l2 system contracts (#3421)\n\n**Motivation**\n\nWe want to be able to upgrade L2 system contracts.\n\n**Description**\n\nThis makes it so that the L2 contracts themselves are proxies. Their\ninitial implementations are kept in the genesis for ease of deployment\nand to avoid keeping them empty in the first blocks.\n\nSince the proxies need to be embedded in the genesis, they can't be\ndeployed with a constructor, so their\n[ERC-1967](https://eips.ethereum.org/EIPS/eip-1967) slots are set\ndirectly.\n\nA function is added to the L1 CommonBridge to allow upgrading the L2\ncontracts. A special address (0xf000) is used to authenticate the\nupgrade.\n\nCloses #3345",
          "timestamp": "2025-07-11T12:27:27Z",
          "tree_id": "5da2a6cdd7e6ca4748dd0f07fac5768e8cfe3540",
          "url": "https://github.com/lambdaclass/ethrex/commit/f466fb8216f85442d763a8ed6a10a36f05e8c93f"
        },
        "date": 1752241129775,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012324395198522623,
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
          "id": "05d3c1290649b1f3949d7376178be78fbb1cecbf",
          "message": "fix(levm): fix benchmark block execution ci (#3619)\n\n**Motivation**\n\nsee\nhttps://github.com/lambdaclass/ethrex/actions/runs/16266441472/job/45923106459?pr=3564\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-14T17:53:13Z",
          "tree_id": "7b8fbc2f30df44acf9fc51a9312de9411c4b9c87",
          "url": "https://github.com/lambdaclass/ethrex/commit/05d3c1290649b1f3949d7376178be78fbb1cecbf"
        },
        "date": 1752522238273,
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
          "id": "05d3c1290649b1f3949d7376178be78fbb1cecbf",
          "message": "fix(levm): fix benchmark block execution ci (#3619)\n\n**Motivation**\n\nsee\nhttps://github.com/lambdaclass/ethrex/actions/runs/16266441472/job/45923106459?pr=3564\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-14T17:53:13Z",
          "tree_id": "7b8fbc2f30df44acf9fc51a9312de9411c4b9c87",
          "url": "https://github.com/lambdaclass/ethrex/commit/05d3c1290649b1f3949d7376178be78fbb1cecbf"
        },
        "date": 1752528903941,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0061793148148148146,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "sfroment42@gmail.com",
            "name": "Sacha Froment",
            "username": "sfroment"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "053237663e3be3dd9eb02dbacba88d6e0ce54610",
          "message": "feat(l1): add From for Transaction -> GenericTransaction (#3227)\n\n**Motivation**\n\nAdding an easy way to get a GenericTransaction from any Transaction\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nAdding the 2 missing From and one for the enum\nThis will allow people who use the ethClient to make estimate_gas and\neth_call request, more easily and maybe other request in the future\nmight benefit from it\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\n\nBTW I don't know which scope I shall use\n\nSigned-off-by: Sacha Froment <sfroment42@gmail.com>\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-07-14T18:08:45Z",
          "tree_id": "b0c6b8443312ff2002a0844abe8e0d7579e19ce8",
          "url": "https://github.com/lambdaclass/ethrex/commit/053237663e3be3dd9eb02dbacba88d6e0ce54610"
        },
        "date": 1752531604560,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012347197039777984,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "sfroment42@gmail.com",
            "name": "Sacha Froment",
            "username": "sfroment"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "053237663e3be3dd9eb02dbacba88d6e0ce54610",
          "message": "feat(l1): add From for Transaction -> GenericTransaction (#3227)\n\n**Motivation**\n\nAdding an easy way to get a GenericTransaction from any Transaction\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nAdding the 2 missing From and one for the enum\nThis will allow people who use the ethClient to make estimate_gas and\neth_call request, more easily and maybe other request in the future\nmight benefit from it\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\n\nBTW I don't know which scope I shall use\n\nSigned-off-by: Sacha Froment <sfroment42@gmail.com>\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-07-14T18:08:45Z",
          "tree_id": "b0c6b8443312ff2002a0844abe8e0d7579e19ce8",
          "url": "https://github.com/lambdaclass/ethrex/commit/053237663e3be3dd9eb02dbacba88d6e0ce54610"
        },
        "date": 1752536782573,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006039511312217195,
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
          "distinct": false,
          "id": "318d68b1ad651c4df08ba03a8b65b27fe50adbff",
          "message": "fix(l1, l2): logs not appearing on subcommands (#3631)\n\n**Motivation**\n\nQuick bug fix that makes logs not appear\n\n**Description**\n\nThe function ```init_tracing(&opts)``` was being called after any\nsubcommands (import, export, etc) were read, causing these (specially\nthe import) not to output logs. This PR fixes that.",
          "timestamp": "2025-07-14T19:43:28Z",
          "tree_id": "21db8e93a6ae21ed8dea0b94c61966566a2010d4",
          "url": "https://github.com/lambdaclass/ethrex/commit/318d68b1ad651c4df08ba03a8b65b27fe50adbff"
        },
        "date": 1752562209598,
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
            "email": "44068466+SDartayet@users.noreply.github.com",
            "name": "SDartayet",
            "username": "SDartayet"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "318d68b1ad651c4df08ba03a8b65b27fe50adbff",
          "message": "fix(l1, l2): logs not appearing on subcommands (#3631)\n\n**Motivation**\n\nQuick bug fix that makes logs not appear\n\n**Description**\n\nThe function ```init_tracing(&opts)``` was being called after any\nsubcommands (import, export, etc) were read, causing these (specially\nthe import) not to output logs. This PR fixes that.",
          "timestamp": "2025-07-14T19:43:28Z",
          "tree_id": "21db8e93a6ae21ed8dea0b94c61966566a2010d4",
          "url": "https://github.com/lambdaclass/ethrex/commit/318d68b1ad651c4df08ba03a8b65b27fe50adbff"
        },
        "date": 1752563189271,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.005778060606060606,
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
          "id": "7e97d4a42213231038801327a5485b720f3dcbde",
          "message": "docs(l1): add documentation on ethereum metrics exporter use (#3538)\n\n**Motivation**\n\nWe don't have proper documentation on running the metrics introduced for\nL1 in #3061\n\n**Description**\n\nThis pr includes a quick start on how to use the new targets to display\nmetrics for running a sync on holesky or hoodi, and a more detailed\ndescription in case you want to display metrics when syncing on another\nnetwork.\n\nCloses #3207",
          "timestamp": "2025-07-14T19:58:28Z",
          "tree_id": "302f57a1d2cecd1d75639aa68bc81c9f627bc936",
          "url": "https://github.com/lambdaclass/ethrex/commit/7e97d4a42213231038801327a5485b720f3dcbde"
        },
        "date": 1752565685924,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0057531551724137936,
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
          "id": "7e97d4a42213231038801327a5485b720f3dcbde",
          "message": "docs(l1): add documentation on ethereum metrics exporter use (#3538)\n\n**Motivation**\n\nWe don't have proper documentation on running the metrics introduced for\nL1 in #3061\n\n**Description**\n\nThis pr includes a quick start on how to use the new targets to display\nmetrics for running a sync on holesky or hoodi, and a more detailed\ndescription in case you want to display metrics when syncing on another\nnetwork.\n\nCloses #3207",
          "timestamp": "2025-07-14T19:58:28Z",
          "tree_id": "302f57a1d2cecd1d75639aa68bc81c9f627bc936",
          "url": "https://github.com/lambdaclass/ethrex/commit/7e97d4a42213231038801327a5485b720f3dcbde"
        },
        "date": 1752568423702,
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
          "id": "d874b90c05456847c4b0d50657916434b4600840",
          "message": "fix(levm): ignore DB storage values for destroyed accounts (#3617)\n\n**Motivation**\nWhen executing blocks in batches an account may be destroyed and created\nagain within the same batch. This can lead to errors as we might try to\nload a storage value from the DB (such as in an `SLOAD`) that doesn't\nexist in the newly created account but that used to be part of the now\ndestroyed account, leading to the incorrect value being loaded.\nThis was detected on sepolia testnet block range 3302786-3302799 where a\nan account was destructed via `SELFDESTRUCT` and then created 6 blocks\nlater via `CREATE`. The same transaction that created it then performed\nan `SSTORE` which was charged the default fee (100 gas) as the stored\nkey and value matched the ones in the previously destroyed storage\ninstead of charging the storage creation fee (2000 gas). The value was\npreviously fetched from the DB by an `SLOAD` operation.\nThis PR solves this issue by first checking if the account was destroyed\nbefore looking up a storage value in the DB (The `Store`). If an account\nwas destroyed then whatever was stored in the DB is no longer valid, so\nwe return the default value (as we would do if the key doesn't exist)\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* (`levm` crate)`GeneralizedDatabase::get_value_from_database`: check if\nthe account was destroyed before querying the DB. If the account was\ndestroyed return default value\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-14T21:39:10Z",
          "tree_id": "23f2aaec44dced688b3ec27ba5b502a6f41983e4",
          "url": "https://github.com/lambdaclass/ethrex/commit/d874b90c05456847c4b0d50657916434b4600840"
        },
        "date": 1752571116568,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012301677419354839,
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
          "id": "d874b90c05456847c4b0d50657916434b4600840",
          "message": "fix(levm): ignore DB storage values for destroyed accounts (#3617)\n\n**Motivation**\nWhen executing blocks in batches an account may be destroyed and created\nagain within the same batch. This can lead to errors as we might try to\nload a storage value from the DB (such as in an `SLOAD`) that doesn't\nexist in the newly created account but that used to be part of the now\ndestroyed account, leading to the incorrect value being loaded.\nThis was detected on sepolia testnet block range 3302786-3302799 where a\nan account was destructed via `SELFDESTRUCT` and then created 6 blocks\nlater via `CREATE`. The same transaction that created it then performed\nan `SSTORE` which was charged the default fee (100 gas) as the stored\nkey and value matched the ones in the previously destroyed storage\ninstead of charging the storage creation fee (2000 gas). The value was\npreviously fetched from the DB by an `SLOAD` operation.\nThis PR solves this issue by first checking if the account was destroyed\nbefore looking up a storage value in the DB (The `Store`). If an account\nwas destroyed then whatever was stored in the DB is no longer valid, so\nwe return the default value (as we would do if the key doesn't exist)\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* (`levm` crate)`GeneralizedDatabase::get_value_from_database`: check if\nthe account was destroyed before querying the DB. If the account was\ndestroyed return default value\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #issue_number",
          "timestamp": "2025-07-14T21:39:10Z",
          "tree_id": "23f2aaec44dced688b3ec27ba5b502a6f41983e4",
          "url": "https://github.com/lambdaclass/ethrex/commit/d874b90c05456847c4b0d50657916434b4600840"
        },
        "date": 1752572026604,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.005985345291479821,
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
          "id": "6de7196718fcf89781c20a190872011cabc85c99",
          "message": "fix(l2): panic because of double init tracing (#3637)\n\n**Motivation**\n\nInit L2 was panicking because of a double call to init_tracing\n\n**Description**\n\n- Move back the init tracing call to after the subcommand execution\n- Inside the subcommands call init_tracing only if the subcommand is not\n`Subcommand::L2`",
          "timestamp": "2025-07-15T13:09:19Z",
          "tree_id": "367eb56892cd70c2b727e9330f073a618c389e94",
          "url": "https://github.com/lambdaclass/ethrex/commit/6de7196718fcf89781c20a190872011cabc85c99"
        },
        "date": 1752589741612,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0062080558139534885,
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
          "id": "6de7196718fcf89781c20a190872011cabc85c99",
          "message": "fix(l2): panic because of double init tracing (#3637)\n\n**Motivation**\n\nInit L2 was panicking because of a double call to init_tracing\n\n**Description**\n\n- Move back the init tracing call to after the subcommand execution\n- Inside the subcommands call init_tracing only if the subcommand is not\n`Subcommand::L2`",
          "timestamp": "2025-07-15T13:09:19Z",
          "tree_id": "367eb56892cd70c2b727e9330f073a618c389e94",
          "url": "https://github.com/lambdaclass/ethrex/commit/6de7196718fcf89781c20a190872011cabc85c99"
        },
        "date": 1752596802412,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012211637694419031,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "906de695154909601de4c10a883cc822509dc270",
          "message": "feat(l2): monitor add delay to scroll (#3616)\n\n**Motivation**\nMonitor scroll goes too fast\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nAdded a delay for the log scroll\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n**How to Test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n- Press Tab to change the Tab\n- Scroll Up and Down to test the scroll\n\nCloses\nhttps://github.com/orgs/lambdaclass/projects/37/views/10?pane=issue&itemId=118809801&issue=lambdaclass%7Cethrex%7C3514",
          "timestamp": "2025-07-15T14:01:38Z",
          "tree_id": "ad406a83542279b38ac48a3d0e98b93574f00c0d",
          "url": "https://github.com/lambdaclass/ethrex/commit/906de695154909601de4c10a883cc822509dc270"
        },
        "date": 1752597716136,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006094666666666667,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "906de695154909601de4c10a883cc822509dc270",
          "message": "feat(l2): monitor add delay to scroll (#3616)\n\n**Motivation**\nMonitor scroll goes too fast\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nAdded a delay for the log scroll\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n**How to Test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n- Press Tab to change the Tab\n- Scroll Up and Down to test the scroll\n\nCloses\nhttps://github.com/orgs/lambdaclass/projects/37/views/10?pane=issue&itemId=118809801&issue=lambdaclass%7Cethrex%7C3514",
          "timestamp": "2025-07-15T14:01:38Z",
          "tree_id": "ad406a83542279b38ac48a3d0e98b93574f00c0d",
          "url": "https://github.com/lambdaclass/ethrex/commit/906de695154909601de4c10a883cc822509dc270"
        },
        "date": 1752600492063,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012222820512820514,
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
          "id": "b0a5da487e8a2ffc4f174a3d5629bdb1e581e7a0",
          "message": "ci(l1): try running hive tests in CI with levm (#3566)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- Run most recent hive tests in CI with LEVM.\n- I had to comment out 2 of them because they don't pass, it was\nexpected since we were running tests that were 6 months old so things\nhave changed.\n\n---------\n\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-07-15T14:25:58Z",
          "tree_id": "aa7582b6c137ea6e00b405c391832b9f826d9898",
          "url": "https://github.com/lambdaclass/ethrex/commit/b0a5da487e8a2ffc4f174a3d5629bdb1e581e7a0"
        },
        "date": 1752611379744,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006094666666666667,
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
          "id": "b0a5da487e8a2ffc4f174a3d5629bdb1e581e7a0",
          "message": "ci(l1): try running hive tests in CI with levm (#3566)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- Run most recent hive tests in CI with LEVM.\n- I had to comment out 2 of them because they don't pass, it was\nexpected since we were running tests that were 6 months old so things\nhave changed.\n\n---------\n\nCo-authored-by: Copilot <175728472+Copilot@users.noreply.github.com>",
          "timestamp": "2025-07-15T14:25:58Z",
          "tree_id": "aa7582b6c137ea6e00b405c391832b9f826d9898",
          "url": "https://github.com/lambdaclass/ethrex/commit/b0a5da487e8a2ffc4f174a3d5629bdb1e581e7a0"
        },
        "date": 1752612900904,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012122906448683015,
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
          "id": "f8a6168341db73d3a593b94e0e0f0a50c1044168",
          "message": "feat(l1): peer scoring for snap requests (#3334)\n\n**Motivation**\nIntegrate and adapt the peer scoring introduced by #2115 for snap\nrequests.\nFor eth requests, we consider failure to return requested data as a peer\nfailure, but with snap the data we request is not guaranteed to be\navailable (as it might have become stale during the sync cycle) so we\ncannot asume that an empty response is a bad response that should be\npenalized. For snap requests this PR collects the ids of the peers we\nattempted to request data from, and once we get a successful peer\nresponse we confirm that the data was indeed available and reward the\nresponsive peer while penalizing the previous unresponsive peers\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Collect ids of peers on each snap request retry and penalize and\nreward peers accordingly upon a successful peer response\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3118",
          "timestamp": "2025-07-15T14:29:34Z",
          "tree_id": "98d4bd1b3523d36f75886638eca8394cb47f9400",
          "url": "https://github.com/lambdaclass/ethrex/commit/f8a6168341db73d3a593b94e0e0f0a50c1044168"
        },
        "date": 1752622497848,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0061793148148148146,
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
          "id": "f8a6168341db73d3a593b94e0e0f0a50c1044168",
          "message": "feat(l1): peer scoring for snap requests (#3334)\n\n**Motivation**\nIntegrate and adapt the peer scoring introduced by #2115 for snap\nrequests.\nFor eth requests, we consider failure to return requested data as a peer\nfailure, but with snap the data we request is not guaranteed to be\navailable (as it might have become stale during the sync cycle) so we\ncannot asume that an empty response is a bad response that should be\npenalized. For snap requests this PR collects the ids of the peers we\nattempted to request data from, and once we get a successful peer\nresponse we confirm that the data was indeed available and reward the\nresponsive peer while penalizing the previous unresponsive peers\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Collect ids of peers on each snap request retry and penalize and\nreward peers accordingly upon a successful peer response\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3118",
          "timestamp": "2025-07-15T14:29:34Z",
          "tree_id": "98d4bd1b3523d36f75886638eca8394cb47f9400",
          "url": "https://github.com/lambdaclass/ethrex/commit/f8a6168341db73d3a593b94e0e0f0a50c1044168"
        },
        "date": 1752645662112,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012200475319926875,
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
          "id": "448b778d849a2e08472c4cbbf3cac6da353ffd9e",
          "message": "feat(l2): embed proxy contract in the SDK (#3443)\n\n**Description**\n\nThis PR adds a `build.rs` build script to the SDK to embed the\n`ERC1967Proxy` contract as a constant. As part of this, it also moves\nthe functions for downloading dependencies and compiling contracts to\nanother crate, since we need to use them inside the build script.\n\nChange list:\n\n- [x] Added build script\n- [x] Added installation of `solc` for compiling in the CI\n- [x] Updated dockerfiles to install solc before compiling\n- [x] Updated `service.nix` to clone dependencies before building.\n- [x] Removed `ERC1967Proxy` compilation steps from the Deployer.\n\nRelated to #3380\n\n---------\n\nCo-authored-by: avilagaston9 <gaston.avila@lambdaclass.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>",
          "timestamp": "2025-07-15T18:33:33Z",
          "tree_id": "bac1601d0b41721457f72a0f73e2693873ccdba1",
          "url": "https://github.com/lambdaclass/ethrex/commit/448b778d849a2e08472c4cbbf3cac6da353ffd9e"
        },
        "date": 1752647214276,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012290349907918968,
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
          "id": "448b778d849a2e08472c4cbbf3cac6da353ffd9e",
          "message": "feat(l2): embed proxy contract in the SDK (#3443)\n\n**Description**\n\nThis PR adds a `build.rs` build script to the SDK to embed the\n`ERC1967Proxy` contract as a constant. As part of this, it also moves\nthe functions for downloading dependencies and compiling contracts to\nanother crate, since we need to use them inside the build script.\n\nChange list:\n\n- [x] Added build script\n- [x] Added installation of `solc` for compiling in the CI\n- [x] Updated dockerfiles to install solc before compiling\n- [x] Updated `service.nix` to clone dependencies before building.\n- [x] Removed `ERC1967Proxy` compilation steps from the Deployer.\n\nRelated to #3380\n\n---------\n\nCo-authored-by: avilagaston9 <gaston.avila@lambdaclass.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>",
          "timestamp": "2025-07-15T18:33:33Z",
          "tree_id": "bac1601d0b41721457f72a0f73e2693873ccdba1",
          "url": "https://github.com/lambdaclass/ethrex/commit/448b778d849a2e08472c4cbbf3cac6da353ffd9e"
        },
        "date": 1752648134491,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0057531551724137936,
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
          "id": "fd98ef02d3634246651f8879e9d70feb1dd0653a",
          "message": "fix(l2): install solc in missing workflows (#3649)\n\n**Motivation**\n\nIn #3443, we missed installing solc in some workflows.\n\nCloses None",
          "timestamp": "2025-07-15T21:20:05Z",
          "tree_id": "66735758ea212d38ae32deee8ccf38901cad506a",
          "url": "https://github.com/lambdaclass/ethrex/commit/fd98ef02d3634246651f8879e9d70feb1dd0653a"
        },
        "date": 1752649702624,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001203545536519387,
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
          "id": "fd98ef02d3634246651f8879e9d70feb1dd0653a",
          "message": "fix(l2): install solc in missing workflows (#3649)\n\n**Motivation**\n\nIn #3443, we missed installing solc in some workflows.\n\nCloses None",
          "timestamp": "2025-07-15T21:20:05Z",
          "tree_id": "66735758ea212d38ae32deee8ccf38901cad506a",
          "url": "https://github.com/lambdaclass/ethrex/commit/fd98ef02d3634246651f8879e9d70feb1dd0653a"
        },
        "date": 1752650721112,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.005854087719298246,
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
          "id": "5c7a30485164c7db8ed43304a4577a0d0451cc54",
          "message": "feat(l2): add support for web3signer (#2714)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nMany operators will want to use a remote signer instead of having the\nprivate keys on the same server as the sequencer.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nReplace all uses of a private key with a new `Signer` enum. This signer\ncan be either `Local` or `Remote` and can be lately extended. This aims\nto standardise the way all kind of messages are signed across the L2 and\nfacilitate the setup via flags or environment\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\n---------\n\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: fedacking <francisco.gauna@lambdaclass.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>",
          "timestamp": "2025-07-15T23:34:22Z",
          "tree_id": "166bed55b2d252034634dd4fb89fe704a900bb8e",
          "url": "https://github.com/lambdaclass/ethrex/commit/5c7a30485164c7db8ed43304a4577a0d0451cc54"
        },
        "date": 1752658236868,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012144968152866243,
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
          "id": "5c7a30485164c7db8ed43304a4577a0d0451cc54",
          "message": "feat(l2): add support for web3signer (#2714)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nMany operators will want to use a remote signer instead of having the\nprivate keys on the same server as the sequencer.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nReplace all uses of a private key with a new `Signer` enum. This signer\ncan be either `Local` or `Remote` and can be lately extended. This aims\nto standardise the way all kind of messages are signed across the L2 and\nfacilitate the setup via flags or environment\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\n---------\n\nCo-authored-by: Javier Rodríguez Chatruc <49622509+jrchatruc@users.noreply.github.com>\nCo-authored-by: fedacking <francisco.gauna@lambdaclass.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>",
          "timestamp": "2025-07-15T23:34:22Z",
          "tree_id": "166bed55b2d252034634dd4fb89fe704a900bb8e",
          "url": "https://github.com/lambdaclass/ethrex/commit/5c7a30485164c7db8ed43304a4577a0d0451cc54"
        },
        "date": 1752659152006,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0061793148148148146,
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
          "id": "ea331e09542d0ffd819d81af32d7a192a3b80f6a",
          "message": "perf(levm): add sstore bench, allow unoptimized bench contracts and improve bench makefile (#3552)\n\n**Motivation**\n\n- Adds a sstore benchmark, however we need to disable solc optimizations\nfor this contract otherwise it removes most code.\n- Improved the makefile adding a command to samply an individual\nbenchmark\n\nhttps://share.firefox.dev/44MVD2V",
          "timestamp": "2025-07-16T06:07:39Z",
          "tree_id": "05d165ee245374bc2320e881bc6c28a6c30b1895",
          "url": "https://github.com/lambdaclass/ethrex/commit/ea331e09542d0ffd819d81af32d7a192a3b80f6a"
        },
        "date": 1752660057053,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006094666666666667,
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
          "id": "ea331e09542d0ffd819d81af32d7a192a3b80f6a",
          "message": "perf(levm): add sstore bench, allow unoptimized bench contracts and improve bench makefile (#3552)\n\n**Motivation**\n\n- Adds a sstore benchmark, however we need to disable solc optimizations\nfor this contract otherwise it removes most code.\n- Improved the makefile adding a command to samply an individual\nbenchmark\n\nhttps://share.firefox.dev/44MVD2V",
          "timestamp": "2025-07-16T06:07:39Z",
          "tree_id": "05d165ee245374bc2320e881bc6c28a6c30b1895",
          "url": "https://github.com/lambdaclass/ethrex/commit/ea331e09542d0ffd819d81af32d7a192a3b80f6a"
        },
        "date": 1752664702261,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012122906448683015,
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
          "id": "8a568cabc9875a7667dd4bf5ce881ec6f26f1e82",
          "message": "refactor(l2): remove expects in L2 monitor (#3615)\n\n**Motivation**\n\nWe want to handle errors gracefully.\n\n**Description**\n\nRemoves usage of .expect\n\n**How to test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n\nCloses #3535\n\n---------\n\nCo-authored-by: Tomás Paradelo <112426153+tomip01@users.noreply.github.com>",
          "timestamp": "2025-07-16T15:02:36Z",
          "tree_id": "5803a4e78ee60df8c6ab4713467b393e4d4cfac4",
          "url": "https://github.com/lambdaclass/ethrex/commit/8a568cabc9875a7667dd4bf5ce881ec6f26f1e82"
        },
        "date": 1752682273883,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006039511312217195,
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
          "id": "8a568cabc9875a7667dd4bf5ce881ec6f26f1e82",
          "message": "refactor(l2): remove expects in L2 monitor (#3615)\n\n**Motivation**\n\nWe want to handle errors gracefully.\n\n**Description**\n\nRemoves usage of .expect\n\n**How to test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n\nCloses #3535\n\n---------\n\nCo-authored-by: Tomás Paradelo <112426153+tomip01@users.noreply.github.com>",
          "timestamp": "2025-07-16T15:02:36Z",
          "tree_id": "5803a4e78ee60df8c6ab4713467b393e4d4cfac4",
          "url": "https://github.com/lambdaclass/ethrex/commit/8a568cabc9875a7667dd4bf5ce881ec6f26f1e82"
        },
        "date": 1752686766799,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001227904323827047,
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
          "id": "faa3dec1f9358872ac18b09e9bd994a80cb1231b",
          "message": "feat(l1): decouple size of execution batch from header request size during full-sync (#3074)\n\n**Motivation**\nAllow us to configure the amount of blocks to execute in a single batch\nduring full sync. Currently, the only way to do this is by changing the\namount of block headers we ask for in each request.\nIn order to achieve this, this PR proposes adding the enum\n`BlockSyncState` with variants for Full and Snap sync so we can separate\nbehaviors between each mode and also allow each mode to keep its\nseparate state. This is key as we will need to persist headers and\nbodies through various fetch requests so we can build custom-sized\nexecution batches.\nIt also replaces the previous configurable env var `BlOCK_HEADER_LIMIT`\nwith `EXECUTE_BLOCK_BATCH`\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Add `BlockSyncState` enum as a way to differentiate between each sync\nmode's state during block syncing phase.\n* Refactor `request_and_validate_block_bodies`: it now receives a slice\nof headers and returns the requested block bodies instead of the full\nblocks. This allowed us to completely get rid of header cloning.\n* `validate_block_body` now receives a reference to the head & body\ninstead of the full block (as a result of refactoring its only user)\n* `Store::add_block_headers` now only receives the headers (This lets us\nsimplify caller code)\n* Removed `search_head` variable as having both current & search head\nserves no purpose.\n* Abtract current_head selection into `BlockSyncState::get_current_head`\n* Fix bug in condition used to decide wether to switch from snap to full\nsync\n* `start_sync` no longer receives `current_head`\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #2894\n\n---------\n\nCo-authored-by: SDartayet <44068466+SDartayet@users.noreply.github.com>\nCo-authored-by: Martin Paulucci <martin.c.paulucci@gmail.com>",
          "timestamp": "2025-07-16T15:55:06Z",
          "tree_id": "32e65b17e7d3493c84eea672f20281cf2df62aaa",
          "url": "https://github.com/lambdaclass/ethrex/commit/faa3dec1f9358872ac18b09e9bd994a80cb1231b"
        },
        "date": 1752690442126,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0061793148148148146,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "f61eecdc5b15f3b35a14edc1ddab871c8ed64468",
          "message": "feat(l2): monitor handle index slicing (#3611)\n\n**Motivation**\nMonitor had unhandled index slicing in its code.\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nAdded new variants for `MonitorError` and used them to remove the index\nslicing in the monitor\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n**How to test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses\nhttps://github.com/orgs/lambdaclass/projects/37/views/10?pane=issue&itemId=118823704&issue=lambdaclass%7Cethrex%7C3537",
          "timestamp": "2025-07-16T18:41:49Z",
          "tree_id": "e64b43c58d56f1187206e6b98a9a253805277cfb",
          "url": "https://github.com/lambdaclass/ethrex/commit/f61eecdc5b15f3b35a14edc1ddab871c8ed64468"
        },
        "date": 1752693099511,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006066963636363636,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "3cf9507c3b63fe81929bb8ae2fd32de3fa049078",
          "message": "feat(l2): make monitor quit (#3622)\n\n**Motivation**\nWhen the monitor is quitted with `Shift + Q` it closes the monitor but\ndoes not end the process\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nChanged the L2 task initialization to use `JoinSet` instead of a\n`TaskTracker`, so it can be joined and end the process if it ended.\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n**How to test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n- Press `Shift + Q` to close the monitor\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses\nhttps://github.com/orgs/lambdaclass/projects/37/views/10?pane=issue&itemId=118808771&issue=lambdaclass%7Cethrex%7C3512",
          "timestamp": "2025-07-16T20:07:24Z",
          "tree_id": "b08bdd2932e3c056a571269355a21b4a1bbfb496",
          "url": "https://github.com/lambdaclass/ethrex/commit/3cf9507c3b63fe81929bb8ae2fd32de3fa049078"
        },
        "date": 1752703186304,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0011927899910634495,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "3cf9507c3b63fe81929bb8ae2fd32de3fa049078",
          "message": "feat(l2): make monitor quit (#3622)\n\n**Motivation**\nWhen the monitor is quitted with `Shift + Q` it closes the monitor but\ndoes not end the process\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nChanged the L2 task initialization to use `JoinSet` instead of a\n`TaskTracker`, so it can be joined and end the process if it ended.\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n**How to test**\n\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Run `make test` in `crates/l2`.\n- Press `Shift + Q` to close the monitor\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses\nhttps://github.com/orgs/lambdaclass/projects/37/views/10?pane=issue&itemId=118808771&issue=lambdaclass%7Cethrex%7C3512",
          "timestamp": "2025-07-16T20:07:24Z",
          "tree_id": "b08bdd2932e3c056a571269355a21b4a1bbfb496",
          "url": "https://github.com/lambdaclass/ethrex/commit/3cf9507c3b63fe81929bb8ae2fd32de3fa049078"
        },
        "date": 1752704110514,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.005985345291479821,
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
          "id": "c7c89a4fb4109c85f04babfd1fad805c7c40fb09",
          "message": "feat(l2): make contract compilation in the SDK optional (#3665)\n\n**Motivation**\n\n#3443, caused `solc` to be a compile-time dependency of the client.\nSince the proxy bytecode is only needed in `deploy_with_proxy`, which is\nonly used by the `deployer`, this PR makes contract compilation\noptional, via an env var.\n\n**Description**\n\n- Modifies `sdk/build.rs` to check whether `COMPILE_CONTRACTS` env var\nis set before trying to compile the proxy.\n- Creates a new error `ProxyBytecodeNotFound`, which is returned if\n`deploy_with_proxy` is called without compiling the contract.\n- Removes the installation of `solc` from workflows and Dockerfiles\nwhere it is no longer needed\n\nCloses #3654",
          "timestamp": "2025-07-16T22:08:17Z",
          "tree_id": "b795bcc727701a80c595d0e5f08cab0c95f414fc",
          "url": "https://github.com/lambdaclass/ethrex/commit/c7c89a4fb4109c85f04babfd1fad805c7c40fb09"
        },
        "date": 1752713214554,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012144968152866243,
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
          "id": "c7c89a4fb4109c85f04babfd1fad805c7c40fb09",
          "message": "feat(l2): make contract compilation in the SDK optional (#3665)\n\n**Motivation**\n\n#3443, caused `solc` to be a compile-time dependency of the client.\nSince the proxy bytecode is only needed in `deploy_with_proxy`, which is\nonly used by the `deployer`, this PR makes contract compilation\noptional, via an env var.\n\n**Description**\n\n- Modifies `sdk/build.rs` to check whether `COMPILE_CONTRACTS` env var\nis set before trying to compile the proxy.\n- Creates a new error `ProxyBytecodeNotFound`, which is returned if\n`deploy_with_proxy` is called without compiling the contract.\n- Removes the installation of `solc` from workflows and Dockerfiles\nwhere it is no longer needed\n\nCloses #3654",
          "timestamp": "2025-07-16T22:08:17Z",
          "tree_id": "b795bcc727701a80c595d0e5f08cab0c95f414fc",
          "url": "https://github.com/lambdaclass/ethrex/commit/c7c89a4fb4109c85f04babfd1fad805c7c40fb09"
        },
        "date": 1752714185205,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006039511312217195,
            "unit": "Mgas/s"
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
          "id": "d8aaed209910719f7f482fd6e3b2f33aefb1aba3",
          "message": "chore(l1, l2): add claude/gemini files to .gitignore (#3653)",
          "timestamp": "2025-07-17T11:04:04Z",
          "tree_id": "c29a6ea2921f69d1cc96417821b0fc03dd1163cb",
          "url": "https://github.com/lambdaclass/ethrex/commit/d8aaed209910719f7f482fd6e3b2f33aefb1aba3"
        },
        "date": 1752752245657,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.005932142222222222,
            "unit": "Mgas/s"
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
          "id": "d8aaed209910719f7f482fd6e3b2f33aefb1aba3",
          "message": "chore(l1, l2): add claude/gemini files to .gitignore (#3653)",
          "timestamp": "2025-07-17T11:04:04Z",
          "tree_id": "c29a6ea2921f69d1cc96417821b0fc03dd1163cb",
          "url": "https://github.com/lambdaclass/ethrex/commit/d8aaed209910719f7f482fd6e3b2f33aefb1aba3"
        },
        "date": 1752758456531,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.00118327304964539,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "tomas.arjovsky@lambdaclass.com",
            "name": "Tomás Arjovsky",
            "username": "Arkenan"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "bc82ed12aee8e7d627a0ac52cbbd8287084b51b2",
          "message": "ci(l1): disable block builcing bench until it's fixed (#3670)\n\n**Motivation**\n\nThe benchmark doesn't work and it's blocking all prs",
          "timestamp": "2025-07-17T11:06:00Z",
          "tree_id": "f6f3fbc8ccbaec48aef841363a5d8a271f0b2e0f",
          "url": "https://github.com/lambdaclass/ethrex/commit/bc82ed12aee8e7d627a0ac52cbbd8287084b51b2"
        },
        "date": 1752764554042,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012200475319926875,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "tomas.arjovsky@lambdaclass.com",
            "name": "Tomás Arjovsky",
            "username": "Arkenan"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "bc82ed12aee8e7d627a0ac52cbbd8287084b51b2",
          "message": "ci(l1): disable block builcing bench until it's fixed (#3670)\n\n**Motivation**\n\nThe benchmark doesn't work and it's blocking all prs",
          "timestamp": "2025-07-17T11:06:00Z",
          "tree_id": "f6f3fbc8ccbaec48aef841363a5d8a271f0b2e0f",
          "url": "https://github.com/lambdaclass/ethrex/commit/bc82ed12aee8e7d627a0ac52cbbd8287084b51b2"
        },
        "date": 1752765273479,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006237065420560748,
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
          "id": "f21fe24bf2e2c2fc62aa9be3db6e0da0f491bcc9",
          "message": "fix(l1): fix double tracer initialization in block execution benchmark (#3671)\n\n**Motivation**\n\nCurrently the block execution benchmark is\n[broken](https://github.com/lambdaclass/ethrex/actions/runs/16344656297/job/46175367153?pr=3590)\nas a result of calling `init_tracing` twice.\n\n**Description**\n\nThis happens because, when the `--removedb` flag is used, RemoveDB is\ncalled as a command, which initializes the logger again.\n\nThis PR calls removedb directly instead.",
          "timestamp": "2025-07-17T13:11:53Z",
          "tree_id": "ebdc1d6f14e1bfb5b07a3a65e9449bb2dc729dd3",
          "url": "https://github.com/lambdaclass/ethrex/commit/f21fe24bf2e2c2fc62aa9be3db6e0da0f491bcc9"
        },
        "date": 1752769396868,
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
          "id": "f21fe24bf2e2c2fc62aa9be3db6e0da0f491bcc9",
          "message": "fix(l1): fix double tracer initialization in block execution benchmark (#3671)\n\n**Motivation**\n\nCurrently the block execution benchmark is\n[broken](https://github.com/lambdaclass/ethrex/actions/runs/16344656297/job/46175367153?pr=3590)\nas a result of calling `init_tracing` twice.\n\n**Description**\n\nThis happens because, when the `--removedb` flag is used, RemoveDB is\ncalled as a command, which initializes the logger again.\n\nThis PR calls removedb directly instead.",
          "timestamp": "2025-07-17T13:11:53Z",
          "tree_id": "ebdc1d6f14e1bfb5b07a3a65e9449bb2dc729dd3",
          "url": "https://github.com/lambdaclass/ethrex/commit/f21fe24bf2e2c2fc62aa9be3db6e0da0f491bcc9"
        },
        "date": 1752775536591,
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
          "id": "c0e0ce2933c1c72d943771abd58563355081c09f",
          "message": "ci(l1): support multiple hive versions depending on simulation. (#3661)\n\n**Motivation**\nWe want to get rid of our hive fork and use the upstream. Unfortunately,\nwe can't completely rely on it yet because it would break.\n\n**Description**\n- While we fix the upstream, lets rely on two versions of Hive, our fork\none and the upstream",
          "timestamp": "2025-07-17T13:31:45Z",
          "tree_id": "3365e4d99a9a881624b7f54dfd3e3d7e9295904a",
          "url": "https://github.com/lambdaclass/ethrex/commit/c0e0ce2933c1c72d943771abd58563355081c09f"
        },
        "date": 1752778569825,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006094666666666667,
            "unit": "Mgas/s"
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
          "id": "c0e0ce2933c1c72d943771abd58563355081c09f",
          "message": "ci(l1): support multiple hive versions depending on simulation. (#3661)\n\n**Motivation**\nWe want to get rid of our hive fork and use the upstream. Unfortunately,\nwe can't completely rely on it yet because it would break.\n\n**Description**\n- While we fix the upstream, lets rely on two versions of Hive, our fork\none and the upstream",
          "timestamp": "2025-07-17T13:31:45Z",
          "tree_id": "3365e4d99a9a881624b7f54dfd3e3d7e9295904a",
          "url": "https://github.com/lambdaclass/ethrex/commit/c0e0ce2933c1c72d943771abd58563355081c09f"
        },
        "date": 1752781276765,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012267757352941177,
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
          "id": "22b64308b7b0badb3e78279b12f8b36f69bd0642",
          "message": "perf(levm): new memory model (#3564)\n\n**Motivation**\n\nGas benchmarks show an 23% improvement on opcode based timings and 12%\non end to end.\n30% improvement in mgas for mstore (before unsafe)\n\nAfter adding unsafe we see a 30% improvement on top of the mstore\nimprovements and overall general improvements on other opcodes.",
          "timestamp": "2025-07-17T14:04:33Z",
          "tree_id": "eeb024c2f8db6140858e55a60a1250ff8fa4cd1b",
          "url": "https://github.com/lambdaclass/ethrex/commit/22b64308b7b0badb3e78279b12f8b36f69bd0642"
        },
        "date": 1752799605874,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012111905626134302,
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
          "id": "22b64308b7b0badb3e78279b12f8b36f69bd0642",
          "message": "perf(levm): new memory model (#3564)\n\n**Motivation**\n\nGas benchmarks show an 23% improvement on opcode based timings and 12%\non end to end.\n30% improvement in mgas for mstore (before unsafe)\n\nAfter adding unsafe we see a 30% improvement on top of the mstore\nimprovements and overall general improvements on other opcodes.",
          "timestamp": "2025-07-17T14:04:33Z",
          "tree_id": "eeb024c2f8db6140858e55a60a1250ff8fa4cd1b",
          "url": "https://github.com/lambdaclass/ethrex/commit/22b64308b7b0badb3e78279b12f8b36f69bd0642"
        },
        "date": 1752808838038,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006237065420560748,
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
          "id": "165b94c9a2069d2a4bb9c320f26554b0190c4e30",
          "message": "perf(levm): add AVX256 implementation of BLAKE2 (#3590)\n\n**Motivation**\n\nTo improve BLAKE2 performance.\n\n**Description**\n\nWhy AVX256 instead of AVX512? Mainly that\n[AVX512](https://github.com/rust-lang/rust/issues/111137) intrinsics are\nstill experimental.\n\nCreates a common/crypto module to house blake2. We should consider\nmoving here other cryptographic operations currently inside\nprecompiles.rs.\n\nIf avx2 is available, a permute-with-gather implementation is used.\n\nUsage of unsafe is required for SIMD loads and stores. It should be\nreviewed that alignment requirements are satisfied and that no\nout-of-bounds operations are possible.\n\nNote that aside from the obvious ones with \"load\" or \"store\" in the\nname, gather also represents a series of memory loads.\n\nUnsafe is also required to call the first avx2-enabled function, since\nwe must first ensure avx2 is actually available on the target CPU.\n\n** Benchmarks **\n\n### PR\n\n|Title|Max (MGas/s)|p50 (MGas/s)|p95 (MGas/s)|p99 (MGas/s)|Min (MGas/s)|\n\n|----|--------------|--------------|-------------|--------------|--------------|\nBlake1MRounds|120.19|93.97|93.38|99.85|91.54\nBlake1Round|226.42|175.09|170.08|166.83|166.82\nBlake1KRounds|122.36|97.28|96.09|100.90|95.87\nBlake10MRounds|174.36|110.78|104.15|124.33|103.89\n\n### Main\n\n|Title|Max (MGas/s)|p50 (MGas/s)|p95 (MGas/s)|p99 (MGas/s)|Min (MGas/s)|\n\n|----|--------------|--------------|-------------|--------------|--------------|\nBlake1MRounds|80.79|63.04|62.57|67.80|62.50\nBlake1Round|223.59|174.93|168.21|159.38|159.33\nBlake1KRounds|83.75|66.59|65.88|68.37|64.76\nBlake10MRounds|117.79|77.21|69.63|83.19|69.05",
          "timestamp": "2025-07-17T14:34:10Z",
          "tree_id": "065dfa16f0769f1776aae5132e7f7e58e22fde93",
          "url": "https://github.com/lambdaclass/ethrex/commit/165b94c9a2069d2a4bb9c320f26554b0190c4e30"
        },
        "date": 1752809503175,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0062080558139534885,
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
          "id": "14ef9bfde463af72598fe43a515c334fea6aedfb",
          "message": "fix(l2): `get_batch` failing if in validium mode (#3680)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nRollup store's`get_batch` fails when in validium mode as it's not\nfinding any blob (currently in validium mode we don't generate blobs).\nThis makes features like `ethrex_getBatchByNumber` unusable in validium\nmode.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nAccept empty blobs bundle when retrieving batches from rollup store.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-07-17T15:25:15Z",
          "tree_id": "585d1a479356a69e449df5a5763ea36b8a040686",
          "url": "https://github.com/lambdaclass/ethrex/commit/14ef9bfde463af72598fe43a515c334fea6aedfb"
        },
        "date": 1752816956612,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012324395198522623,
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
          "id": "14ef9bfde463af72598fe43a515c334fea6aedfb",
          "message": "fix(l2): `get_batch` failing if in validium mode (#3680)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\nRollup store's`get_batch` fails when in validium mode as it's not\nfinding any blob (currently in validium mode we don't generate blobs).\nThis makes features like `ethrex_getBatchByNumber` unusable in validium\nmode.\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nAccept empty blobs bundle when retrieving batches from rollup store.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-07-17T15:25:15Z",
          "tree_id": "585d1a479356a69e449df5a5763ea36b8a040686",
          "url": "https://github.com/lambdaclass/ethrex/commit/14ef9bfde463af72598fe43a515c334fea6aedfb"
        },
        "date": 1752822863901,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.005778060606060606,
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
          "id": "df3710b203a0214e243a157f46147d48b2d9d38a",
          "message": "ci(l1,l2): remove ethrex replay from releases  (#3663)\n\n**Motivation**\n\nWe don't want to make releases for ethrex replay\n\n**Description**\n\n- Remove matrix.binary from ci and only build ethrex and prover binaries\n- Update docs on how to run ethrex-replay\n- Successful run\n[here](https://github.com/lambdaclass/ethrex/actions/runs/16346228196/job/46180528866)",
          "timestamp": "2025-07-17T15:36:58Z",
          "tree_id": "e74c7e272ffd46bdc24eae77eb489c4cc5e1e7a2",
          "url": "https://github.com/lambdaclass/ethrex/commit/df3710b203a0214e243a157f46147d48b2d9d38a"
        },
        "date": 1752828377466,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012122906448683015,
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
          "id": "df3710b203a0214e243a157f46147d48b2d9d38a",
          "message": "ci(l1,l2): remove ethrex replay from releases  (#3663)\n\n**Motivation**\n\nWe don't want to make releases for ethrex replay\n\n**Description**\n\n- Remove matrix.binary from ci and only build ethrex and prover binaries\n- Update docs on how to run ethrex-replay\n- Successful run\n[here](https://github.com/lambdaclass/ethrex/actions/runs/16346228196/job/46180528866)",
          "timestamp": "2025-07-17T15:36:58Z",
          "tree_id": "e74c7e272ffd46bdc24eae77eb489c4cc5e1e7a2",
          "url": "https://github.com/lambdaclass/ethrex/commit/df3710b203a0214e243a157f46147d48b2d9d38a"
        },
        "date": 1752834799037,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0061793148148148146,
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
          "distinct": true,
          "id": "229b791477f203dde019f36d5d62be182139d63a",
          "message": "refactor(l1): make ethrex-only github actions faster (#3648)\n\n**Motivation**\n\nRunning the ethrex_only github actions job seems to be slower than those\nthat use other execution clients as well\n\n**Description**\n\nThere were 2 main reasons why this job was slower compared to the others\n- The ethrex_only job includes the EOA and BLOB transactions assertoor\nplaybooks, which are the ones being run in the other two github jobs\n- The slot time of 12 sec was making the test take to long\n\nThe slot time was modified and now the tests take 10 minutes instead of\nthe original 18\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3628",
          "timestamp": "2025-07-17T16:21:11Z",
          "tree_id": "41a3b691e05e3ff267354c99a2f50f9b45e9edc7",
          "url": "https://github.com/lambdaclass/ethrex/commit/229b791477f203dde019f36d5d62be182139d63a"
        },
        "date": 1752835490177,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0062080558139534885,
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
          "distinct": true,
          "id": "229b791477f203dde019f36d5d62be182139d63a",
          "message": "refactor(l1): make ethrex-only github actions faster (#3648)\n\n**Motivation**\n\nRunning the ethrex_only github actions job seems to be slower than those\nthat use other execution clients as well\n\n**Description**\n\nThere were 2 main reasons why this job was slower compared to the others\n- The ethrex_only job includes the EOA and BLOB transactions assertoor\nplaybooks, which are the ones being run in the other two github jobs\n- The slot time of 12 sec was making the test take to long\n\nThe slot time was modified and now the tests take 10 minutes instead of\nthe original 18\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3628",
          "timestamp": "2025-07-17T16:21:11Z",
          "tree_id": "41a3b691e05e3ff267354c99a2f50f9b45e9edc7",
          "url": "https://github.com/lambdaclass/ethrex/commit/229b791477f203dde019f36d5d62be182139d63a"
        },
        "date": 1752838167290,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012324395198522623,
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
          "id": "23191af7468828cb68d161143b460a6f25c96181",
          "message": "perf(levm): improve sstore perfomance  further (#3657)\n\n**Motivation**\nImproves sstore perfomance\n\nRequires #3564\n\nFrom 1100 to over 2200\n\n<img width=\"1896\" height=\"281\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/7f5697a3-048c-4554-91bb-22839bb91d95\"\n/>\n\nThe main change is going from Hashmaps to BTreeMaps.\n\nThey are more efficient for the type of storages we use, for small\ndatasets (1k~100k i would say) they overperform hashmaps due to avoiding\nentirely the hashing cost, which seemed to be the biggest factor.\n\nThis changes comes with 2 other minor changes, like a more efficient\nu256 to big endian and a change to backup_storage_slot.",
          "timestamp": "2025-07-17T17:29:21Z",
          "tree_id": "ad97fe646e7b6d407f1bfc5a7afa50ca64c47427",
          "url": "https://github.com/lambdaclass/ethrex/commit/23191af7468828cb68d161143b460a6f25c96181"
        },
        "date": 1752840863632,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012509203373945643,
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
          "id": "23191af7468828cb68d161143b460a6f25c96181",
          "message": "perf(levm): improve sstore perfomance  further (#3657)\n\n**Motivation**\nImproves sstore perfomance\n\nRequires #3564\n\nFrom 1100 to over 2200\n\n<img width=\"1896\" height=\"281\" alt=\"image\"\nsrc=\"https://github.com/user-attachments/assets/7f5697a3-048c-4554-91bb-22839bb91d95\"\n/>\n\nThe main change is going from Hashmaps to BTreeMaps.\n\nThey are more efficient for the type of storages we use, for small\ndatasets (1k~100k i would say) they overperform hashmaps due to avoiding\nentirely the hashing cost, which seemed to be the biggest factor.\n\nThis changes comes with 2 other minor changes, like a more efficient\nu256 to big endian and a change to backup_storage_slot.",
          "timestamp": "2025-07-17T17:29:21Z",
          "tree_id": "ad97fe646e7b6d407f1bfc5a7afa50ca64c47427",
          "url": "https://github.com/lambdaclass/ethrex/commit/23191af7468828cb68d161143b460a6f25c96181"
        },
        "date": 1752841550102,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006066963636363636,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me+git@droak.sh",
            "name": "Oak",
            "username": "d-roak"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "8272969a64958abfed0f7085cb7c4d684f2202df",
          "message": "docs(l2, levm): move crates docs to root docs (#3303)\n\n**Motivation**\nDocs are sparsed across the repo. This PR puts everything in the same\nplace\n\n**Description**\n- Added the docs that lived under `/crates/*` in the root `/docs`. Used\nthe same file structure\n- Deleted all instances of docs under `/crates/*`\n\n\nCloses: none\n\nSigned-off-by: droak <me+git@droak.sh>",
          "timestamp": "2025-07-17T18:10:12Z",
          "tree_id": "17e6af6a4ed0b436216ded6e99f2ed41de21d5c1",
          "url": "https://github.com/lambdaclass/ethrex/commit/8272969a64958abfed0f7085cb7c4d684f2202df"
        },
        "date": 1752844241214,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012370083410565339,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me+git@droak.sh",
            "name": "Oak",
            "username": "d-roak"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "8272969a64958abfed0f7085cb7c4d684f2202df",
          "message": "docs(l2, levm): move crates docs to root docs (#3303)\n\n**Motivation**\nDocs are sparsed across the repo. This PR puts everything in the same\nplace\n\n**Description**\n- Added the docs that lived under `/crates/*` in the root `/docs`. Used\nthe same file structure\n- Deleted all instances of docs under `/crates/*`\n\n\nCloses: none\n\nSigned-off-by: droak <me+git@droak.sh>",
          "timestamp": "2025-07-17T18:10:12Z",
          "tree_id": "17e6af6a4ed0b436216ded6e99f2ed41de21d5c1",
          "url": "https://github.com/lambdaclass/ethrex/commit/8272969a64958abfed0f7085cb7c4d684f2202df"
        },
        "date": 1752859360726,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006237065420560748,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me+git@droak.sh",
            "name": "Oak",
            "username": "d-roak"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "96c7eeeabfc03ea6b8a20b92f5310cb59d7a63c8",
          "message": "docs(l2): add quotes on init-prover command (#3304)\n\n**Motivation**\nCopy paste on the command provided in the docs doesn't work\n\n**Description**\n- Added quotes to the command mentioned\n\nCloses: none",
          "timestamp": "2025-07-17T18:18:56Z",
          "tree_id": "f41598ce40f7d42ba9c46c352b21d5f3ea010eed",
          "url": "https://github.com/lambdaclass/ethrex/commit/96c7eeeabfc03ea6b8a20b92f5310cb59d7a63c8"
        },
        "date": 1752862063024,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001235862962962963,
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
          "id": "6876c91b7f93ebbf65124410582e6f89513f9768",
          "message": "perf(levm): codecopy perf improvement (#3675)\n\n**Motivation**\n\nImproves from 200 mgas to 790, this bench was made with this pr along\nmemory, sstore and opcodes ones.\n\nA 295% increase in perf.\n\nRequires the pr #3564 \n\n**Description**",
          "timestamp": "2025-07-18T05:06:43Z",
          "tree_id": "295ee8748a84fa9fc38dc54a27f6f57ac03c6626",
          "url": "https://github.com/lambdaclass/ethrex/commit/6876c91b7f93ebbf65124410582e6f89513f9768"
        },
        "date": 1752872109751,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.006266347417840375,
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
          "id": "6876c91b7f93ebbf65124410582e6f89513f9768",
          "message": "perf(levm): codecopy perf improvement (#3675)\n\n**Motivation**\n\nImproves from 200 mgas to 790, this bench was made with this pr along\nmemory, sstore and opcodes ones.\n\nA 295% increase in perf.\n\nRequires the pr #3564 \n\n**Description**",
          "timestamp": "2025-07-18T05:06:43Z",
          "tree_id": "295ee8748a84fa9fc38dc54a27f6f57ac03c6626",
          "url": "https://github.com/lambdaclass/ethrex/commit/6876c91b7f93ebbf65124410582e6f89513f9768"
        },
        "date": 1752874844600,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0012211637694419031,
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
          "id": "b9f189573533b771f82ba45ef7bd65daefd02a55",
          "message": "refactor(l2): remove blockByNumber (#3752)\n\n**Motivation**\n\nWhile reviewing areas for simplification, I found that `BlockByNumber`\nis not being used.\n\n**Description**\n\nRemoves `BlockByNumber`\n\nCloses #3748",
          "timestamp": "2025-07-21T19:08:00Z",
          "tree_id": "030f712bb4c97a9bb320513b3614f976638c92cd",
          "url": "https://github.com/lambdaclass/ethrex/commit/b9f189573533b771f82ba45ef7bd65daefd02a55"
        },
        "date": 1753126401858,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.0075408587570621475,
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
          "id": "b9f189573533b771f82ba45ef7bd65daefd02a55",
          "message": "refactor(l2): remove blockByNumber (#3752)\n\n**Motivation**\n\nWhile reviewing areas for simplification, I found that `BlockByNumber`\nis not being used.\n\n**Description**\n\nRemoves `BlockByNumber`\n\nCloses #3748",
          "timestamp": "2025-07-21T19:08:00Z",
          "tree_id": "030f712bb4c97a9bb320513b3614f976638c92cd",
          "url": "https://github.com/lambdaclass/ethrex/commit/b9f189573533b771f82ba45ef7bd65daefd02a55"
        },
        "date": 1753130776179,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013441409869083586,
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
          "id": "93a885595f00e092fd597e03270b214da85114a2",
          "message": "fix(levm): preemptively resize memory before executing call (#3592)\n\n**Motivation**\nWhen executing a `CALL` opcode, a transfer might take place. In this\ncase the instruction does contain a return data offset and a return data\nsize but as we don't have return data to write into memory we don't\nexpand the memory.\nThis can cause problems with other opcodes later on (such as MSTORE,\nMLOAD, etc) which calculate their gas cost based on the difference\nbetween the current size of the memory and the new size, making them\nmore expensive as the memory will be smaller due to return data from\ntransfers not being accounted for.\nThis PR aims to solve this by preemptively resizing the memory before\nexecuting the call, so that the memory gets expanded even if no return\ndata is written to it.\nThis bug was found on Sepolia transaction:\nhttps://sepolia.etherscan.io/tx/0xa1765d420522a40d59d15f8dee1bf095499be687d6e1a7c978fc87eb85bce948\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Preemptively resize memory before executing a call in opcode `CALL`\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n**Questions**\nShould this behaviour also apply to other call types?\nCloses #issue_number",
          "timestamp": "2025-07-21T21:12:15Z",
          "tree_id": "4e31ff7d7cf6aea3a6a0e6f216926845008e439f",
          "url": "https://github.com/lambdaclass/ethrex/commit/93a885595f00e092fd597e03270b214da85114a2"
        },
        "date": 1753140601507,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001334732,
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
          "id": "93a885595f00e092fd597e03270b214da85114a2",
          "message": "fix(levm): preemptively resize memory before executing call (#3592)\n\n**Motivation**\nWhen executing a `CALL` opcode, a transfer might take place. In this\ncase the instruction does contain a return data offset and a return data\nsize but as we don't have return data to write into memory we don't\nexpand the memory.\nThis can cause problems with other opcodes later on (such as MSTORE,\nMLOAD, etc) which calculate their gas cost based on the difference\nbetween the current size of the memory and the new size, making them\nmore expensive as the memory will be smaller due to return data from\ntransfers not being accounted for.\nThis PR aims to solve this by preemptively resizing the memory before\nexecuting the call, so that the memory gets expanded even if no return\ndata is written to it.\nThis bug was found on Sepolia transaction:\nhttps://sepolia.etherscan.io/tx/0xa1765d420522a40d59d15f8dee1bf095499be687d6e1a7c978fc87eb85bce948\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n* Preemptively resize memory before executing a call in opcode `CALL`\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n**Questions**\nShould this behaviour also apply to other call types?\nCloses #issue_number",
          "timestamp": "2025-07-21T21:12:15Z",
          "tree_id": "4e31ff7d7cf6aea3a6a0e6f216926845008e439f",
          "url": "https://github.com/lambdaclass/ethrex/commit/93a885595f00e092fd597e03270b214da85114a2"
        },
        "date": 1753141158295,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007670873563218391,
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
          "id": "ec0a3eb6b536dda5668dff993369e9067f8709dd",
          "message": "chore(levm): parallelize parsing ef state tests (#3722)\n\n**Motivation**\n\nEf test parsing is slow, this parallelizes it making it faster\n\n\nRan in 2m0.225s",
          "timestamp": "2025-07-22T09:08:06Z",
          "tree_id": "791d9a8fffafcc8a4268db6202ce229dac132476",
          "url": "https://github.com/lambdaclass/ethrex/commit/ec0a3eb6b536dda5668dff993369e9067f8709dd"
        },
        "date": 1753176533309,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007293617486338798,
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
          "id": "ec0a3eb6b536dda5668dff993369e9067f8709dd",
          "message": "chore(levm): parallelize parsing ef state tests (#3722)\n\n**Motivation**\n\nEf test parsing is slow, this parallelizes it making it faster\n\n\nRan in 2m0.225s",
          "timestamp": "2025-07-22T09:08:06Z",
          "tree_id": "791d9a8fffafcc8a4268db6202ce229dac132476",
          "url": "https://github.com/lambdaclass/ethrex/commit/ec0a3eb6b536dda5668dff993369e9067f8709dd"
        },
        "date": 1753178703108,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001334732,
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
          "id": "e283db20a41622318fda4869992e08591911625e",
          "message": "feat(levm): execute arbitrary bytecode (#3626)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\nHave a runner for LEVM that expects some **inputs** like Transaction,\nFork, etc. in an `json` file and **bytecode in mnemonics** in another\nfile. Stack and memory can be preloaded within the `json`.\nMore info in the `README.md`\n\nSidenote: I had to do a refactor in LEVM setup because for me to be able\nto alter the stack and memory before executing these have to be\ninitialized in the `new()`, thing that we weren't doing. So we now\ninitialize the first callframe there and not in `execute()`.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3583\n\n---------\n\nCo-authored-by: fmoletta <99273364+fmoletta@users.noreply.github.com>\nCo-authored-by: Edgar <git@edgl.dev>",
          "timestamp": "2025-07-22T13:16:10Z",
          "tree_id": "f5786f587ea0b2b549d9262913b775de1a103a34",
          "url": "https://github.com/lambdaclass/ethrex/commit/e283db20a41622318fda4869992e08591911625e"
        },
        "date": 1753192177852,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00771521387283237,
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
          "id": "e283db20a41622318fda4869992e08591911625e",
          "message": "feat(levm): execute arbitrary bytecode (#3626)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\nHave a runner for LEVM that expects some **inputs** like Transaction,\nFork, etc. in an `json` file and **bytecode in mnemonics** in another\nfile. Stack and memory can be preloaded within the `json`.\nMore info in the `README.md`\n\nSidenote: I had to do a refactor in LEVM setup because for me to be able\nto alter the stack and memory before executing these have to be\ninitialized in the `new()`, thing that we weren't doing. So we now\ninitialize the first callframe there and not in `execute()`.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3583\n\n---------\n\nCo-authored-by: fmoletta <99273364+fmoletta@users.noreply.github.com>\nCo-authored-by: Edgar <git@edgl.dev>",
          "timestamp": "2025-07-22T13:16:10Z",
          "tree_id": "f5786f587ea0b2b549d9262913b775de1a103a34",
          "url": "https://github.com/lambdaclass/ethrex/commit/e283db20a41622318fda4869992e08591911625e"
        },
        "date": 1753194329463,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013675532786885246,
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
          "id": "2ce46bf32443be74f0d2ee8b0d5759f9c04219cb",
          "message": "refactor(levm): rewrite of state EF tests runner first iteration (#3642)\n\n**Motivation**\n\nRelated issue: #3496. \n\nThe idea is to incrementally develop a new EF Test runner (for state\ntests) that can eventually replace the current one. The main goal of the\nnew runner is to be easy to understand and as straightforward as\npossible, also making it possible to easily add any new requirement.\n\n**How to run** \nA target in the makefile was included. You can, then, from\n`ethrex/cmd/ef_tests/state/` run `make run-new-runner`. If no specific\npath is passed, it will parse anything in the `./vectors` folder.\nOtherwise you can do, for example:\n`make run-new-runner TESTS_PATH=./vectors/GeneralStateTests/Cancun` to\nspecify a path.\n\nThis command assumes you have the `vectors` directory downloaded, if not\nrun `make download-evm-ef-tests` previously.\n\n**Considerations**\n\nThe main changes are: \n- The new `Test` and `TestCase` structures in types. \n- The runner and parser simplified flows. \n\nFiles that should not be reviewed as they are full or partial copies of\nthe original files:\n- `runner_v2/deserialize.rs`\n- `runner_v2/utils.rs`\n\nThis iteration excludes report-related code, option flags and other\npossible test case errors to be considered that will be included later.\nChecks are performed only on exceptions and root hash.",
          "timestamp": "2025-07-22T18:32:15Z",
          "tree_id": "1364dd940e94383958d73b23545152bd053470bf",
          "url": "https://github.com/lambdaclass/ethrex/commit/2ce46bf32443be74f0d2ee8b0d5759f9c04219cb"
        },
        "date": 1753211884362,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013215168316831683,
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
          "id": "2ce46bf32443be74f0d2ee8b0d5759f9c04219cb",
          "message": "refactor(levm): rewrite of state EF tests runner first iteration (#3642)\n\n**Motivation**\n\nRelated issue: #3496. \n\nThe idea is to incrementally develop a new EF Test runner (for state\ntests) that can eventually replace the current one. The main goal of the\nnew runner is to be easy to understand and as straightforward as\npossible, also making it possible to easily add any new requirement.\n\n**How to run** \nA target in the makefile was included. You can, then, from\n`ethrex/cmd/ef_tests/state/` run `make run-new-runner`. If no specific\npath is passed, it will parse anything in the `./vectors` folder.\nOtherwise you can do, for example:\n`make run-new-runner TESTS_PATH=./vectors/GeneralStateTests/Cancun` to\nspecify a path.\n\nThis command assumes you have the `vectors` directory downloaded, if not\nrun `make download-evm-ef-tests` previously.\n\n**Considerations**\n\nThe main changes are: \n- The new `Test` and `TestCase` structures in types. \n- The runner and parser simplified flows. \n\nFiles that should not be reviewed as they are full or partial copies of\nthe original files:\n- `runner_v2/deserialize.rs`\n- `runner_v2/utils.rs`\n\nThis iteration excludes report-related code, option flags and other\npossible test case errors to be considered that will be included later.\nChecks are performed only on exceptions and root hash.",
          "timestamp": "2025-07-22T18:32:15Z",
          "tree_id": "1364dd940e94383958d73b23545152bd053470bf",
          "url": "https://github.com/lambdaclass/ethrex/commit/2ce46bf32443be74f0d2ee8b0d5759f9c04219cb"
        },
        "date": 1753217196576,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007805450292397661,
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
          "id": "b41d878a318aeaf8dcbd7c2292569fa697282a76",
          "message": "fix(l2): fix L1 proof sender's wallet/signer (#3747)\n\n**Motivation**\n\nThe L1 proof sender was broken in #2714 by creating an invalid ethers'\n`Wallet`\n[here](github.com/lambdaclass/ethrex/pull/2714/files#r2216602944). This\nPR fixes it but only allows running the proof sender with a local\nsigner.\n\nTo support a remote signer we must investigate if there's a way to\ncreate an ethers' signer that uses web3signer.\n\nThanks @avilagaston9 for noticing the bug!",
          "timestamp": "2025-07-22T19:31:59Z",
          "tree_id": "20136d8c01aeca6066b9529a2e69154d4cacc679",
          "url": "https://github.com/lambdaclass/ethrex/commit/b41d878a318aeaf8dcbd7c2292569fa697282a76"
        },
        "date": 1753224520621,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00771521387283237,
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
          "id": "b41d878a318aeaf8dcbd7c2292569fa697282a76",
          "message": "fix(l2): fix L1 proof sender's wallet/signer (#3747)\n\n**Motivation**\n\nThe L1 proof sender was broken in #2714 by creating an invalid ethers'\n`Wallet`\n[here](github.com/lambdaclass/ethrex/pull/2714/files#r2216602944). This\nPR fixes it but only allows running the proof sender with a local\nsigner.\n\nTo support a remote signer we must investigate if there's a way to\ncreate an ethers' signer that uses web3signer.\n\nThanks @avilagaston9 for noticing the bug!",
          "timestamp": "2025-07-22T19:31:59Z",
          "tree_id": "20136d8c01aeca6066b9529a2e69154d4cacc679",
          "url": "https://github.com/lambdaclass/ethrex/commit/b41d878a318aeaf8dcbd7c2292569fa697282a76"
        },
        "date": 1753225804399,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013675532786885246,
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
          "id": "cfe00d33bd3464bd5cd625978d92a0f1f8068f63",
          "message": "fix(l2): enable bls12_381,k256 & ecdsa sp1 precompiles (#3691)\n\n**Motivation**\n\nThe patch for bls12_381 precompile is not being applied because we are\nimporting the crate from our fork.\nAlso two other patches that were previously not compiling after #3689 is\nmerged can now be reenabled\n\n**Description**\n\n-\n[Forked](https://github.com/lambdaclass/bls12_381-patch/tree/expose-fp-struct)\nthe patch from sp1 and updated it with the same changes we have on the\nmain crate fork\n- Uncommented the previously commented patches\n\n**How to check**\n\n```\ncd crates/l2/prover/zkvm/interface/sp1\n```\nbls12_381\n```\ncargo tree -p bls12_381\n```\nreturns `bls12_381 v0.8.0\n(https://github.com/lambdaclass/bls12_381-patch/?branch=expose-fp-struct#f2242f78)`\n\necdsa\n```\ncargo tree -p ecdsa\n```\nreturns `ecdsa v0.16.9\n(https://github.com/sp1-patches/signatures?tag=patch-16.9-sp1-4.1.0#1880299a)`\n\nk256\n```\ncargo tree -p k256\n```\nreturns `k256 v0.13.4\n(https://github.com/sp1-patches/elliptic-curves?tag=patch-k256-13.4-sp1-5.0.0#f7d8998e)`\n\nComparing this to main that it either returns no patch or errors out",
          "timestamp": "2025-07-22T19:50:33Z",
          "tree_id": "556520e376bd9a93a8bfb4cc139da53a6e4531d0",
          "url": "https://github.com/lambdaclass/ethrex/commit/cfe00d33bd3464bd5cd625978d92a0f1f8068f63"
        },
        "date": 1753226293307,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00771521387283237,
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
          "id": "cfe00d33bd3464bd5cd625978d92a0f1f8068f63",
          "message": "fix(l2): enable bls12_381,k256 & ecdsa sp1 precompiles (#3691)\n\n**Motivation**\n\nThe patch for bls12_381 precompile is not being applied because we are\nimporting the crate from our fork.\nAlso two other patches that were previously not compiling after #3689 is\nmerged can now be reenabled\n\n**Description**\n\n-\n[Forked](https://github.com/lambdaclass/bls12_381-patch/tree/expose-fp-struct)\nthe patch from sp1 and updated it with the same changes we have on the\nmain crate fork\n- Uncommented the previously commented patches\n\n**How to check**\n\n```\ncd crates/l2/prover/zkvm/interface/sp1\n```\nbls12_381\n```\ncargo tree -p bls12_381\n```\nreturns `bls12_381 v0.8.0\n(https://github.com/lambdaclass/bls12_381-patch/?branch=expose-fp-struct#f2242f78)`\n\necdsa\n```\ncargo tree -p ecdsa\n```\nreturns `ecdsa v0.16.9\n(https://github.com/sp1-patches/signatures?tag=patch-16.9-sp1-4.1.0#1880299a)`\n\nk256\n```\ncargo tree -p k256\n```\nreturns `k256 v0.13.4\n(https://github.com/sp1-patches/elliptic-curves?tag=patch-k256-13.4-sp1-5.0.0#f7d8998e)`\n\nComparing this to main that it either returns no patch or errors out",
          "timestamp": "2025-07-22T19:50:33Z",
          "tree_id": "556520e376bd9a93a8bfb4cc139da53a6e4531d0",
          "url": "https://github.com/lambdaclass/ethrex/commit/cfe00d33bd3464bd5cd625978d92a0f1f8068f63"
        },
        "date": 1753230064675,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013661535312180144,
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
          "id": "c36b343d8508b62b7de0a7d87bc58a026278704a",
          "message": "fix(levm): memory bug when storing data (#3774)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nWe didn't realize that #3564 introduced a bug when storing data of\nlength zero. This aims to fix it.\nI also delete a resize check that's completely unnecessary\n\nTested the fix and it works. I now am able to execute the blocks\nmentioned in the issue of this PR without any problems at all.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3775",
          "timestamp": "2025-07-22T20:11:38Z",
          "tree_id": "db0e155e11ab6e5d6808c62ec2e43bfed8834f9a",
          "url": "https://github.com/lambdaclass/ethrex/commit/c36b343d8508b62b7de0a7d87bc58a026278704a"
        },
        "date": 1753231351110,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013427887323943662,
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
          "id": "c36b343d8508b62b7de0a7d87bc58a026278704a",
          "message": "fix(levm): memory bug when storing data (#3774)\n\n**Motivation**\n\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\nWe didn't realize that #3564 introduced a bug when storing data of\nlength zero. This aims to fix it.\nI also delete a resize check that's completely unnecessary\n\nTested the fix and it works. I now am able to execute the blocks\nmentioned in the issue of this PR without any problems at all.\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->\n\nCloses #3775",
          "timestamp": "2025-07-22T20:11:38Z",
          "tree_id": "db0e155e11ab6e5d6808c62ec2e43bfed8834f9a",
          "url": "https://github.com/lambdaclass/ethrex/commit/c36b343d8508b62b7de0a7d87bc58a026278704a"
        },
        "date": 1753231865786,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007760069767441861,
            "unit": "Mgas/s"
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
          "id": "4a3a5aec56e6b6a96942ad161b32fd2f50ccd5c7",
          "message": "refactor(l2): apply fcu only on the last block of the batch for the block fetcher (#3782)\n\n**Motivation**\n\nWith the actual implementation of the block fetcher, we apply a fork\nchoice update for every block. This is not the optimal way since we can\napply only on the last block.\n\n**Description**\n\n- Move the `apply_fork_choice` call after the loop and only call it with\nthe last block\n- Add new type of error `EmptyBatchError`",
          "timestamp": "2025-07-22T21:03:49Z",
          "tree_id": "710ac64388c952a5a69f10f5b9a1eda132ef6c9a",
          "url": "https://github.com/lambdaclass/ethrex/commit/4a3a5aec56e6b6a96942ad161b32fd2f50ccd5c7"
        },
        "date": 1753232354195,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00771521387283237,
            "unit": "Mgas/s"
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
          "id": "4a3a5aec56e6b6a96942ad161b32fd2f50ccd5c7",
          "message": "refactor(l2): apply fcu only on the last block of the batch for the block fetcher (#3782)\n\n**Motivation**\n\nWith the actual implementation of the block fetcher, we apply a fork\nchoice update for every block. This is not the optimal way since we can\napply only on the last block.\n\n**Description**\n\n- Move the `apply_fork_choice` call after the loop and only call it with\nthe last block\n- Add new type of error `EmptyBatchError`",
          "timestamp": "2025-07-22T21:03:49Z",
          "tree_id": "710ac64388c952a5a69f10f5b9a1eda132ef6c9a",
          "url": "https://github.com/lambdaclass/ethrex/commit/4a3a5aec56e6b6a96942ad161b32fd2f50ccd5c7"
        },
        "date": 1753233625569,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013523120567375886,
            "unit": "Mgas/s"
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
          "distinct": false,
          "id": "3f60642861576555f50dd330410eb75c49188447",
          "message": "feat(l2): based P2P (#2999)\n\n**Motivation**\n\nThis PR follows #2931 . We implement some basic functionality to\ncommunicate L2 based nodes via P2P.\n\n**Description**\n\n- Add new capability to the RLPx called `Based`.\n- Add new Message `NewBlock`.\n  - Behaves similar to the message `Transactions`.\n- Every interval we look to the new blocks produced and send them to the\npeer.\n- Add this message to the allowed ones to be broadcasted via the P2P\nnetwork.\n- When receiving this message we implemented a queue to be able to\nreceive them in disorder. Once a continuos interval of blocks is in the\nqueue we store them in order.\n- Add new message `BatchSealed`\n- Every interval we look in the `store_rollup` if a new batch has been\nsealed and then we send it to the peer.\n- Add this message to the allowed ones to be broadcasted via the P2P\nnetwork.\n- This two new messages are signed by the lead sequencer who proposed\nthe blocks and the batches. Every node must verify this signature\ncorrespond to the lead sequencer\n- Change `BlockFetcher` to not add a block received via the L1 if it\nalready has been received via P2P, and vice versa.\n- Add a new `SequencingStatus`: `Syncing`. It is for nodes that are not\nup to date to the last committed batch.\n\n**How to test**\n\nRead the `Run Locally` section from `crates/l2/based/README.md` to run 3\nnodes and register 2 of them as Sequencers. It is important that you\nassign different values in the nodes:\n- `--http.port <PORT>`\n- `--committer.l1-private-key <PRIVATE_KEY>`\n- `--proof-coordinator.port <PORT>`\n- `--p2p.port <P2P_PORT>`\n- `--discovery.port <PORT>`\n\n> [!TIP]\n> To enrich the review, I strongly suggest you read the documentation in\n`crates/l2/based/docs`.\n\n---------\n\nCo-authored-by: Leandro Serra <leandro.serra@lambdaclass.com>\nCo-authored-by: ilitteri <ilitteri@fi.uba.ar>\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: fkrause98 <fkrausear@gmail.com>\nCo-authored-by: Francisco Krause Arnim <56402156+fkrause98@users.noreply.github.com>",
          "timestamp": "2025-07-22T22:04:21Z",
          "tree_id": "2fbb970513ea40b72556f60e771be7c88d6540a4",
          "url": "https://github.com/lambdaclass/ethrex/commit/3f60642861576555f50dd330410eb75c49188447"
        },
        "date": 1753234128206,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00762704,
            "unit": "Mgas/s"
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
          "distinct": false,
          "id": "3f60642861576555f50dd330410eb75c49188447",
          "message": "feat(l2): based P2P (#2999)\n\n**Motivation**\n\nThis PR follows #2931 . We implement some basic functionality to\ncommunicate L2 based nodes via P2P.\n\n**Description**\n\n- Add new capability to the RLPx called `Based`.\n- Add new Message `NewBlock`.\n  - Behaves similar to the message `Transactions`.\n- Every interval we look to the new blocks produced and send them to the\npeer.\n- Add this message to the allowed ones to be broadcasted via the P2P\nnetwork.\n- When receiving this message we implemented a queue to be able to\nreceive them in disorder. Once a continuos interval of blocks is in the\nqueue we store them in order.\n- Add new message `BatchSealed`\n- Every interval we look in the `store_rollup` if a new batch has been\nsealed and then we send it to the peer.\n- Add this message to the allowed ones to be broadcasted via the P2P\nnetwork.\n- This two new messages are signed by the lead sequencer who proposed\nthe blocks and the batches. Every node must verify this signature\ncorrespond to the lead sequencer\n- Change `BlockFetcher` to not add a block received via the L1 if it\nalready has been received via P2P, and vice versa.\n- Add a new `SequencingStatus`: `Syncing`. It is for nodes that are not\nup to date to the last committed batch.\n\n**How to test**\n\nRead the `Run Locally` section from `crates/l2/based/README.md` to run 3\nnodes and register 2 of them as Sequencers. It is important that you\nassign different values in the nodes:\n- `--http.port <PORT>`\n- `--committer.l1-private-key <PRIVATE_KEY>`\n- `--proof-coordinator.port <PORT>`\n- `--p2p.port <P2P_PORT>`\n- `--discovery.port <PORT>`\n\n> [!TIP]\n> To enrich the review, I strongly suggest you read the documentation in\n`crates/l2/based/docs`.\n\n---------\n\nCo-authored-by: Leandro Serra <leandro.serra@lambdaclass.com>\nCo-authored-by: ilitteri <ilitteri@fi.uba.ar>\nCo-authored-by: Ivan Litteri <67517699+ilitteri@users.noreply.github.com>\nCo-authored-by: Avila Gastón <72628438+avilagaston9@users.noreply.github.com>\nCo-authored-by: Tomás Grüner <47506558+MegaRedHand@users.noreply.github.com>\nCo-authored-by: fkrause98 <fkrausear@gmail.com>\nCo-authored-by: Francisco Krause Arnim <56402156+fkrause98@users.noreply.github.com>",
          "timestamp": "2025-07-22T22:04:21Z",
          "tree_id": "2fbb970513ea40b72556f60e771be7c88d6540a4",
          "url": "https://github.com/lambdaclass/ethrex/commit/3f60642861576555f50dd330410eb75c49188447"
        },
        "date": 1753235415840,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013731810699588478,
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
          "id": "c1778eadda9854a3824aac5f304204150c14a97b",
          "message": "chore(l1): change logs in hive to info by default (#3767)\n\n**Motivation**\n\nIn the PR #2975 the default value for the `make run-hive` was changed to\nerror. I propose changing this to info (3), as we usually run make hive\nto try to see a problem with the test. For the CI I propose we change it\nto log level error (1), as we can't actually look at those logs.\n\n**Description**\n\n- Changed makefile `SIM_LOG_LEVEL` default value to 3 (info)\n- Added to the ci workflows `--sim.loglevel 1` which corresponds to\nerror.",
          "timestamp": "2025-07-23T13:51:18Z",
          "tree_id": "c226e0a9f7ec2b05e2e4c8136af012522784660a",
          "url": "https://github.com/lambdaclass/ethrex/commit/c1778eadda9854a3824aac5f304204150c14a97b"
        },
        "date": 1753282921165,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013124208456243855,
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
          "id": "c1778eadda9854a3824aac5f304204150c14a97b",
          "message": "chore(l1): change logs in hive to info by default (#3767)\n\n**Motivation**\n\nIn the PR #2975 the default value for the `make run-hive` was changed to\nerror. I propose changing this to info (3), as we usually run make hive\nto try to see a problem with the test. For the CI I propose we change it\nto log level error (1), as we can't actually look at those logs.\n\n**Description**\n\n- Changed makefile `SIM_LOG_LEVEL` default value to 3 (info)\n- Added to the ci workflows `--sim.loglevel 1` which corresponds to\nerror.",
          "timestamp": "2025-07-23T13:51:18Z",
          "tree_id": "c226e0a9f7ec2b05e2e4c8136af012522784660a",
          "url": "https://github.com/lambdaclass/ethrex/commit/c1778eadda9854a3824aac5f304204150c14a97b"
        },
        "date": 1753283448129,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00771521387283237,
            "unit": "Mgas/s"
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
          "id": "4edd454bf4df8dad51b2c32a810a89cd2a9479a6",
          "message": "chore(l1): avoid running EF blockchain tests on `make test` (#3772)\n\n**Motivation**\nThey take a some time and `make test` should be more of a healthcheck\nimo. They run in the CI anyway.",
          "timestamp": "2025-07-23T15:25:54Z",
          "tree_id": "bc9f06de3dbdad519a7d50581577dea43afb1fa8",
          "url": "https://github.com/lambdaclass/ethrex/commit/4edd454bf4df8dad51b2c32a810a89cd2a9479a6"
        },
        "date": 1753287509110,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00771521387283237,
            "unit": "Mgas/s"
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
          "id": "4edd454bf4df8dad51b2c32a810a89cd2a9479a6",
          "message": "chore(l1): avoid running EF blockchain tests on `make test` (#3772)\n\n**Motivation**\nThey take a some time and `make test` should be more of a healthcheck\nimo. They run in the CI anyway.",
          "timestamp": "2025-07-23T15:25:54Z",
          "tree_id": "bc9f06de3dbdad519a7d50581577dea43afb1fa8",
          "url": "https://github.com/lambdaclass/ethrex/commit/4edd454bf4df8dad51b2c32a810a89cd2a9479a6"
        },
        "date": 1753288890477,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001349577350859454,
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
          "id": "e2cb314efc88038727816005e66b3ee99def5c8c",
          "message": "feat(levm): subcommand for converting mnemonics into bytecode and accepting both kinds as arguments (#3786)\n\n**Motivation**\n\n- Add code related features to levm runner\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- Accept both raw bytecode and mnemonics as arguments for the `--code`\nflag in the `.txt` file\n- Add `--emit-bytes` for converting mnemonics into a new bytecode file\nthat can then be used for running the transaction without parsing the\nvalues.\n\nCloses #3788",
          "timestamp": "2025-07-23T15:42:14Z",
          "tree_id": "fcad77c30da72b4e4322322f55b3bc04ba4e1bd9",
          "url": "https://github.com/lambdaclass/ethrex/commit/e2cb314efc88038727816005e66b3ee99def5c8c"
        },
        "date": 1753291546221,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007415177777777778,
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
          "id": "e2cb314efc88038727816005e66b3ee99def5c8c",
          "message": "feat(levm): subcommand for converting mnemonics into bytecode and accepting both kinds as arguments (#3786)\n\n**Motivation**\n\n- Add code related features to levm runner\n\n**Description**\n\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n- Accept both raw bytecode and mnemonics as arguments for the `--code`\nflag in the `.txt` file\n- Add `--emit-bytes` for converting mnemonics into a new bytecode file\nthat can then be used for running the transaction without parsing the\nvalues.\n\nCloses #3788",
          "timestamp": "2025-07-23T15:42:14Z",
          "tree_id": "fcad77c30da72b4e4322322f55b3bc04ba4e1bd9",
          "url": "https://github.com/lambdaclass/ethrex/commit/e2cb314efc88038727816005e66b3ee99def5c8c"
        },
        "date": 1753297165491,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013294143426294822,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "56092489+ColoCarletti@users.noreply.github.com",
            "name": "Joaquin Carletti",
            "username": "ColoCarletti"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "8408fe0854a66e0a510b0a6bf474dda20edd38de",
          "message": "perf(levm): migrate EcAdd and EcMul to Arkworks (#3719)\n\nThis PR improves the performance of the precompiles by switching to\nArkworks.\nIn particular, scalar multiplication on the BN254 curve is significantly\nfaster in Arkworks compared to Lambdaworks.\n\ncloses #3726\n\n---------\n\nCo-authored-by: Leandro Serra <leandro.serra@lambdaclass.com>",
          "timestamp": "2025-07-23T16:04:55Z",
          "tree_id": "780cbcf4c7f07b65b63ff07011ea6247e03377cc",
          "url": "https://github.com/lambdaclass/ethrex/commit/8408fe0854a66e0a510b0a6bf474dda20edd38de"
        },
        "date": 1753297711539,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007456603351955308,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "56092489+ColoCarletti@users.noreply.github.com",
            "name": "Joaquin Carletti",
            "username": "ColoCarletti"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "8408fe0854a66e0a510b0a6bf474dda20edd38de",
          "message": "perf(levm): migrate EcAdd and EcMul to Arkworks (#3719)\n\nThis PR improves the performance of the precompiles by switching to\nArkworks.\nIn particular, scalar multiplication on the BN254 curve is significantly\nfaster in Arkworks compared to Lambdaworks.\n\ncloses #3726\n\n---------\n\nCo-authored-by: Leandro Serra <leandro.serra@lambdaclass.com>",
          "timestamp": "2025-07-23T16:04:55Z",
          "tree_id": "780cbcf4c7f07b65b63ff07011ea6247e03377cc",
          "url": "https://github.com/lambdaclass/ethrex/commit/8408fe0854a66e0a510b0a6bf474dda20edd38de"
        },
        "date": 1753301442598,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013124208456243855,
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
          "distinct": true,
          "id": "1802f66ed21aff9ca45056ad9a0a6a81b6a4a2b0",
          "message": "feat(l1): notebook for high-level profiling (#3633)\n\nIntroduce a new notebook to analyze contribution of eaxh part of the\nblock production process to its overall time, producing graphs for\nvisual clarity.\nInstructions included in the README.\n\nBased on #3274\nCoauthored-by: @Arkenan\n\nPart of: #3331",
          "timestamp": "2025-07-23T17:15:17Z",
          "tree_id": "90f55d482f41009e1f0aab974c2f11afaaef03e1",
          "url": "https://github.com/lambdaclass/ethrex/commit/1802f66ed21aff9ca45056ad9a0a6a81b6a4a2b0"
        },
        "date": 1753301985645,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007415177777777778,
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
          "distinct": true,
          "id": "1802f66ed21aff9ca45056ad9a0a6a81b6a4a2b0",
          "message": "feat(l1): notebook for high-level profiling (#3633)\n\nIntroduce a new notebook to analyze contribution of eaxh part of the\nblock production process to its overall time, producing graphs for\nvisual clarity.\nInstructions included in the README.\n\nBased on #3274\nCoauthored-by: @Arkenan\n\nPart of: #3331",
          "timestamp": "2025-07-23T17:15:17Z",
          "tree_id": "90f55d482f41009e1f0aab974c2f11afaaef03e1",
          "url": "https://github.com/lambdaclass/ethrex/commit/1802f66ed21aff9ca45056ad9a0a6a81b6a4a2b0"
        },
        "date": 1753303320368,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001320209693372898,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "62400508+juan518munoz@users.noreply.github.com",
            "name": "juan518munoz",
            "username": "juan518munoz"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "31808c9e890a3af68e659735c63dcbb47df85a56",
          "message": "chore(l1,l2): bump spawned version to `0.2.1` (#3780)\n\n**Motivation**\n\nUpdate Spawned to accomodate new Actor interface.\n\n**Description**\n\nSince [spawned `0.2.0`](https://github.com/lambdaclass/spawned/pull/35)\nthe state and GenServer is \"the same\".",
          "timestamp": "2025-07-23T18:10:26Z",
          "tree_id": "1e3122f81bfb5a1e4cfcd914eae36c824d663bfc",
          "url": "https://github.com/lambdaclass/ethrex/commit/31808c9e890a3af68e659735c63dcbb47df85a56"
        },
        "date": 1753303838030,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007415177777777778,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "62400508+juan518munoz@users.noreply.github.com",
            "name": "juan518munoz",
            "username": "juan518munoz"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "31808c9e890a3af68e659735c63dcbb47df85a56",
          "message": "chore(l1,l2): bump spawned version to `0.2.1` (#3780)\n\n**Motivation**\n\nUpdate Spawned to accomodate new Actor interface.\n\n**Description**\n\nSince [spawned `0.2.0`](https://github.com/lambdaclass/spawned/pull/35)\nthe state and GenServer is \"the same\".",
          "timestamp": "2025-07-23T18:10:26Z",
          "tree_id": "1e3122f81bfb5a1e4cfcd914eae36c824d663bfc",
          "url": "https://github.com/lambdaclass/ethrex/commit/31808c9e890a3af68e659735c63dcbb47df85a56"
        },
        "date": 1753305158294,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001334732,
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
          "id": "614cc6d0300718b727304672d93a2ddf6adaf21d",
          "message": "docs(l1): move install instructions to new section and embed script one-liner (#3505)\n\n**Motivation**\n\nSince the install script just builds from source using a `cargo install`\none-liner, it's preferable to show that instead of having to download\nand run an install script.\n\n**Description**\n\nThis PR removes the install script, embedding the one-liner inside the\ndocs. It also moves the installation instructions to the book, linking\nto it in the readme, and expands them with instructions on how to build\nfrom source or download the pre-built binaries.\n\n---------\n\nCo-authored-by: Manuel Iñaki Bilbao <manuel.bilbao@lambdaclass.com>",
          "timestamp": "2025-07-23T21:47:54Z",
          "tree_id": "0b46ef2d7f648cf19cf1c02cfa8af0c4501391a5",
          "url": "https://github.com/lambdaclass/ethrex/commit/614cc6d0300718b727304672d93a2ddf6adaf21d"
        },
        "date": 1753309043175,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007374209944751382,
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
          "id": "614cc6d0300718b727304672d93a2ddf6adaf21d",
          "message": "docs(l1): move install instructions to new section and embed script one-liner (#3505)\n\n**Motivation**\n\nSince the install script just builds from source using a `cargo install`\none-liner, it's preferable to show that instead of having to download\nand run an install script.\n\n**Description**\n\nThis PR removes the install script, embedding the one-liner inside the\ndocs. It also moves the installation instructions to the book, linking\nto it in the readme, and expands them with instructions on how to build\nfrom source or download the pre-built binaries.\n\n---------\n\nCo-authored-by: Manuel Iñaki Bilbao <manuel.bilbao@lambdaclass.com>",
          "timestamp": "2025-07-23T21:47:54Z",
          "tree_id": "0b46ef2d7f648cf19cf1c02cfa8af0c4501391a5",
          "url": "https://github.com/lambdaclass/ethrex/commit/614cc6d0300718b727304672d93a2ddf6adaf21d"
        },
        "date": 1753310351548,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001334732,
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
          "id": "67cd8bea1ce06c8a875599f420a1ca05f528aa07",
          "message": "feat(l2): embed contracts in deployer and system_contracts_updater (#3604)\n\n**Motivation**\n\nThis PR embeds the bytecode of the contracts used in the `deployer` and\n`system_contracts_updater` as constants within the resulting binaries.\n\n**Description**\n\n- Adds a `build.rs` script under `crates/l2/contracts/bin/build.rs` that\ndownloads all necessary dependencies and compiles all required\ncontracts.\n- Modifies `deployer` and `system_contracts_updater` to import the\nresulting bytecodes as constants using `include_bytes!`, instead of\ncompiling them at runtime.\n- Removes the `download_contract_deps` function from the SDK, as it was\nonly cloning the same two repositories and was used even when only one\nwas needed.\n- Updates the `compile_contract` function in the SDK to accept a list of\n`remappings`.\n- Adds `deploy_contract_from_bytecode` and\n`deploy_with_proxy_from_bytecode` functions to the SDK.\n- Updates tests to work with the new SDK API.\n\n> [!NOTE]\n> The new `build.rs` script checks if `COMPILE_CONTRACTS` is set to\ndecide whether to compile the contracts.\n> This prevents `cargo check --workspace` from requiring `solc` as a\ndependency.\n\nCloses #3380",
          "timestamp": "2025-07-24T12:52:01Z",
          "tree_id": "181b3933b4e4d0214fffc3f5448d06d614709de8",
          "url": "https://github.com/lambdaclass/ethrex/commit/67cd8bea1ce06c8a875599f420a1ca05f528aa07"
        },
        "date": 1753365810703,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001306,
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
          "id": "67cd8bea1ce06c8a875599f420a1ca05f528aa07",
          "message": "feat(l2): embed contracts in deployer and system_contracts_updater (#3604)\n\n**Motivation**\n\nThis PR embeds the bytecode of the contracts used in the `deployer` and\n`system_contracts_updater` as constants within the resulting binaries.\n\n**Description**\n\n- Adds a `build.rs` script under `crates/l2/contracts/bin/build.rs` that\ndownloads all necessary dependencies and compiles all required\ncontracts.\n- Modifies `deployer` and `system_contracts_updater` to import the\nresulting bytecodes as constants using `include_bytes!`, instead of\ncompiling them at runtime.\n- Removes the `download_contract_deps` function from the SDK, as it was\nonly cloning the same two repositories and was used even when only one\nwas needed.\n- Updates the `compile_contract` function in the SDK to accept a list of\n`remappings`.\n- Adds `deploy_contract_from_bytecode` and\n`deploy_with_proxy_from_bytecode` functions to the SDK.\n- Updates tests to work with the new SDK API.\n\n> [!NOTE]\n> The new `build.rs` script checks if `COMPILE_CONTRACTS` is set to\ndecide whether to compile the contracts.\n> This prevents `cargo check --workspace` from requiring `solc` as a\ndependency.\n\nCloses #3380",
          "timestamp": "2025-07-24T12:52:01Z",
          "tree_id": "181b3933b4e4d0214fffc3f5448d06d614709de8",
          "url": "https://github.com/lambdaclass/ethrex/commit/67cd8bea1ce06c8a875599f420a1ca05f528aa07"
        },
        "date": 1753366341785,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007670873563218391,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "df3a9bd81724520f527cc837775419629eebcfec",
          "message": "feat(l2): enhance monitor performance (#3757)\n\n**Motivation**\nIf a sequencer runs for a long time, it stops, and we run it again\nactivating the monitor, it takes a long time to start and is slow.\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nMakes the monitor load and work faster by simplifying the batches\nprocessing.\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n**How to Test**\n\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Let the sequencer ran for some time (at least 60 batches)\n- Kill the sequencer\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run `make init-l2-no-metrics`\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-07-24T14:16:59Z",
          "tree_id": "aea4bef1fd38b28adda61ffe55e827444f640da9",
          "url": "https://github.com/lambdaclass/ethrex/commit/df3a9bd81724520f527cc837775419629eebcfec"
        },
        "date": 1753367763566,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007670873563218391,
            "unit": "Mgas/s"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "39842759+gianbelinche@users.noreply.github.com",
            "name": "Gianbelinche",
            "username": "gianbelinche"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": false,
          "id": "df3a9bd81724520f527cc837775419629eebcfec",
          "message": "feat(l2): enhance monitor performance (#3757)\n\n**Motivation**\nIf a sequencer runs for a long time, it stops, and we run it again\nactivating the monitor, it takes a long time to start and is slow.\n<!-- Why does this pull request exist? What are its goals? -->\n\n**Description**\nMakes the monitor load and work faster by simplifying the batches\nprocessing.\n<!-- A clear and concise general description of the changes this PR\nintroduces -->\n\n**How to Test**\n\n- Run a Sequencer (I suggest `make restart` in `crates/l2`).\n- Run the prover with `make init-prover` in `crates/l2`.\n- Let the sequencer ran for some time (at least 60 batches)\n- Kill the sequencer\n- Add `--monitor` to the `init-l2-no-metrics` target in\n`crates/l2/Makefile`.\n- Run `make init-l2-no-metrics`\n\n<!-- Link to issues: Resolves #111, Resolves #222 -->",
          "timestamp": "2025-07-24T14:16:59Z",
          "tree_id": "aea4bef1fd38b28adda61ffe55e827444f640da9",
          "url": "https://github.com/lambdaclass/ethrex/commit/df3a9bd81724520f527cc837775419629eebcfec"
        },
        "date": 1753370978954,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001330739780658026,
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
          "id": "8d7a9096401de0e6ff01c6e66e19513e0c522264",
          "message": "refactor(l2): improve naming and standardize arguments in l2 tests (#3790)\n\n**Motivation**\n\nCurrently the L2 tests:\n* use unintuitive names (eth_client vs proposer_client, meaning l1 and\nl2)\n* do not have a consistent ordering of parameters\n* are inconsistent on when things (bridge address and rich private key)\nare given as parameter vs obtained from a function\n\n**Description**\n\nThis PR improves that, and gets the \"noisy\" changes out of the way for\nfurther improvements.\n\nThe rich private key was kept as a parameter to allow giving different\nones (in the future, this would allow parallelizing the tests). The\nbridge address now always uses the function, since it won't change in\nthe middle of the test.",
          "timestamp": "2025-07-24T14:31:19Z",
          "tree_id": "1923a7ff48eef83a37502db6250daa76a844c694",
          "url": "https://github.com/lambdaclass/ethrex/commit/8d7a9096401de0e6ff01c6e66e19513e0c522264"
        },
        "date": 1753374231752,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013509433198380567,
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
          "id": "8d7a9096401de0e6ff01c6e66e19513e0c522264",
          "message": "refactor(l2): improve naming and standardize arguments in l2 tests (#3790)\n\n**Motivation**\n\nCurrently the L2 tests:\n* use unintuitive names (eth_client vs proposer_client, meaning l1 and\nl2)\n* do not have a consistent ordering of parameters\n* are inconsistent on when things (bridge address and rich private key)\nare given as parameter vs obtained from a function\n\n**Description**\n\nThis PR improves that, and gets the \"noisy\" changes out of the way for\nfurther improvements.\n\nThe rich private key was kept as a parameter to allow giving different\nones (in the future, this would allow parallelizing the tests). The\nbridge address now always uses the function, since it won't change in\nthe middle of the test.",
          "timestamp": "2025-07-24T14:31:19Z",
          "tree_id": "1923a7ff48eef83a37502db6250daa76a844c694",
          "url": "https://github.com/lambdaclass/ethrex/commit/8d7a9096401de0e6ff01c6e66e19513e0c522264"
        },
        "date": 1753374760141,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00762704,
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
          "id": "7c3fffcd507ef0deb49e61a45535d6e6db0366be",
          "message": "chore(l2): bump sp1 version to 5.0.8 (#3737)\n\n**Motivation**\n\nSome PRs that updated the Cargo.lock and bumped sp1 to 5.0.8 were\nfailing because sp1up was installing version 5.0.0.\n\n**Description**\n\n- Bump and lock all versions of sp1 to 5.0.8",
          "timestamp": "2025-07-24T14:58:11Z",
          "tree_id": "61796f6914bb141eccf48801f6433f68534c9961",
          "url": "https://github.com/lambdaclass/ethrex/commit/7c3fffcd507ef0deb49e61a45535d6e6db0366be"
        },
        "date": 1753375353136,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.007670873563218391,
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
          "id": "7c3fffcd507ef0deb49e61a45535d6e6db0366be",
          "message": "chore(l2): bump sp1 version to 5.0.8 (#3737)\n\n**Motivation**\n\nSome PRs that updated the Cargo.lock and bumped sp1 to 5.0.8 were\nfailing because sp1up was installing version 5.0.0.\n\n**Description**\n\n- Bump and lock all versions of sp1 to 5.0.8",
          "timestamp": "2025-07-24T14:58:11Z",
          "tree_id": "61796f6914bb141eccf48801f6433f68534c9961",
          "url": "https://github.com/lambdaclass/ethrex/commit/7c3fffcd507ef0deb49e61a45535d6e6db0366be"
        },
        "date": 1753376665543,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.0013888990634755463,
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
          "distinct": false,
          "id": "7e6185d658f7b4f4871f56f044e39aa26528ab11",
          "message": "perf(levm): add shortcut for precompile calls (#3802)\n\n**Motivation**\n\nCurrently, calls to precompiles generate a callframe (including a stack\nand a new memory).\n\n**Description**\n\nAvoid creating call frames for precompiles.",
          "timestamp": "2025-07-24T15:36:42Z",
          "tree_id": "c1f806a73e4a7f1e7ef1c030fd1caee99ffb8a2c",
          "url": "https://github.com/lambdaclass/ethrex/commit/7e6185d658f7b4f4871f56f044e39aa26528ab11"
        },
        "date": 1753377954467,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "Risc0, RTX A6000",
            "value": 0.001346853683148335,
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
          "distinct": false,
          "id": "7e6185d658f7b4f4871f56f044e39aa26528ab11",
          "message": "perf(levm): add shortcut for precompile calls (#3802)\n\n**Motivation**\n\nCurrently, calls to precompiles generate a callframe (including a stack\nand a new memory).\n\n**Description**\n\nAvoid creating call frames for precompiles.",
          "timestamp": "2025-07-24T15:36:42Z",
          "tree_id": "c1f806a73e4a7f1e7ef1c030fd1caee99ffb8a2c",
          "url": "https://github.com/lambdaclass/ethrex/commit/7e6185d658f7b4f4871f56f044e39aa26528ab11"
        },
        "date": 1753382864540,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "SP1, RTX A6000",
            "value": 0.00771521387283237,
            "unit": "Mgas/s"
          }
        ]
      }
    ]
  }
}