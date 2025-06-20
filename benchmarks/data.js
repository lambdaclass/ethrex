window.BENCHMARK_DATA = {
  "lastUpdate": 1750413114056,
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
      }
    ]
  }
}