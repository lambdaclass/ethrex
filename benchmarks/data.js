window.BENCHMARK_DATA = {
  "lastUpdate": 1750345049734,
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
      }
    ]
  }
}