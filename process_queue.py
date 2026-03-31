#!/usr/bin/env python3
"""Process PR data and output the Pending Review table."""
import json
import re
from datetime import datetime, timezone

GITHUB_USER = "ElFantasma"
TODAY = datetime(2026, 2, 10, tzinfo=timezone.utc)

# PR data from REST API (page 1+2+3)
# Format: {number: {title, author, created_at, draft, labels, additions, deletions}}
pr_data = {}

# Reviews: {number: [{author, state}]}
reviews_data = {}

# States: {number: {state, merged}}
states_data = {}

# Details: {number: {additions, deletions}}
details_data = {}

# Parse the output file
with open("/Users/esteban/.claude-ergodic/projects/-Users-esteban-dev-lambda-claude-ethrex/d4219cb0-ebb1-4cbc-9c8a-d79f62d10326/tool-results/toolu_01KxVPeF3R1DJ3t7NpfmHP6H.txt") as f:
    section = None
    for line in f:
        line = line.strip()
        if not line or line.startswith("  ..."):
            continue
        if line == "=== REVIEWS ===":
            section = "reviews"
            continue
        elif line == "=== STATES ===":
            section = "states"
            continue
        elif line == "=== DETAILS ===":
            section = "details"
            continue
        elif line == "=== DONE ===":
            break

        if section == "reviews":
            parts = line.split("|", 1)
            if len(parts) == 2:
                pr_num = int(parts[0])
                try:
                    reviews_data[pr_num] = json.loads(parts[1])
                except json.JSONDecodeError:
                    reviews_data[pr_num] = []

        elif section == "states":
            parts = line.split("|", 1)
            if len(parts) == 2:
                pr_num = int(parts[0])
                try:
                    states_data[pr_num] = json.loads(parts[1])
                except json.JSONDecodeError:
                    states_data[pr_num] = {"state": "unknown", "merged": False}

        elif section == "details":
            parts = line.split("|", 1)
            if len(parts) == 2:
                pr_num = int(parts[0])
                try:
                    details_data[pr_num] = json.loads(parts[1])
                except json.JSONDecodeError:
                    details_data[pr_num] = {"additions": None, "deletions": None}

# PR metadata from the REST list endpoints
# Manually populate from the fetched data
raw_prs = {
    6168: {"title": "chore: fix \"unsuported fork\" typo in whole repo", "author": "gap-editor", "created_at": "2026-02-10T18:46:15Z", "draft": False, "labels": []},
    6164: {"title": "feat(l1): add interactive REPL", "author": "ilitteri", "created_at": "2026-02-10T03:12:04Z", "draft": False, "labels": ["L1"]},
    6159: {"title": "perf(l1): optimize snap sync insertion and healing write paths", "author": "pablodeymo", "created_at": "2026-02-09T15:52:34Z", "draft": False, "labels": ["performance", "L1", "snapsync"]},
    6156: {"title": "refactor(l1,l2): remove duplicate bytes-to-nibbles conversion", "author": "viktorking7", "created_at": "2026-02-08T08:17:51Z", "draft": False, "labels": []},
    6152: {"title": "feat(l1): integrate GenServer block builder into ethrex --dev mode", "author": "ilitteri", "created_at": "2026-02-06T19:49:56Z", "draft": False, "labels": []},
    6151: {"title": "test(l1): add restart stall reproduction test using eth-docker", "author": "pablodeymo", "created_at": "2026-02-06T18:46:48Z", "draft": False, "labels": ["L1"]},
    6147: {"title": "chore(l1): replace unjustified panics with proper error propagation", "author": "iovoid", "created_at": "2026-02-06T14:58:41Z", "draft": False, "labels": ["L1"]},
    6146: {"title": "feat(l1): add on-demand CPU profiling HTTP endpoint", "author": "ilitteri", "created_at": "2026-02-06T04:34:53Z", "draft": False, "labels": ["L1"]},
    6144: {"title": "perf(l1): throttle discovery, deduplicate KZG validation, and add RocksDB block cache", "author": "ilitteri", "created_at": "2026-02-06T02:42:07Z", "draft": False, "labels": ["performance", "L1"]},
    6131: {"title": "ci(l1): add cleanup step to daily snapsync workflow", "author": "ilitteri", "created_at": "2026-02-05T14:58:18Z", "draft": False, "labels": ["L1"]},
    6126: {"title": "refactor(l1): improve error types with actionable context", "author": "pablodeymo", "created_at": "2026-02-04T21:59:52Z", "draft": False, "labels": ["L1"]},
    6124: {"title": "fix(p2p): store validated ENR from handshake in peer table", "author": "Himess", "created_at": "2026-02-04T16:17:02Z", "draft": False, "labels": []},
    6123: {"title": "fix(l2): normalize error codes in based OnChainProposer", "author": "Himess", "created_at": "2026-02-04T15:52:56Z", "draft": False, "labels": []},
    6121: {"title": "chore(deps): bump the cargo group across 1 directory with 1 update", "author": "dependabot[bot]", "created_at": "2026-02-04T14:29:30Z", "draft": False, "labels": ["dependencies", "rust"]},
    6120: {"title": "feat(l1): add mainnet fork-id test cases (EIP-2124)", "author": "dik654", "created_at": "2026-02-04T13:24:53Z", "draft": False, "labels": []},
    6116: {"title": "perf(rpc): avoid decode+re-encode in newPayload transactions root", "author": "jrchatruc", "created_at": "2026-02-03T20:11:08Z", "draft": False, "labels": ["performance"]},
    6114: {"title": "docs(l2): add L2 roadmap", "author": "avilagaston9", "created_at": "2026-02-03T19:26:50Z", "draft": False, "labels": ["L2"]},
    6113: {"title": "perf(l1): replace synchronous disk I/O with async operations in snap sync", "author": "pablodeymo", "created_at": "2026-02-03T18:58:57Z", "draft": False, "labels": ["performance", "L1", "snapsync"]},
    6112: {"title": "docs(l1): snapsync roadmap", "author": "pablodeymo", "created_at": "2026-02-03T18:55:51Z", "draft": False, "labels": ["L1", "snapsync"]},
    6109: {"title": "feat(l1): store validated ENR from handshake in peer table", "author": "mikhailofff", "created_at": "2026-02-03T16:11:42Z", "draft": False, "labels": []},
    6108: {"title": "feat(l1): add snap sync benchmark tool for performance measurement", "author": "pablodeymo", "created_at": "2026-02-03T15:29:23Z", "draft": False, "labels": ["performance", "L1", "snapsync"]},
    6107: {"title": "docs(l1): add UX/DevEx roadmap", "author": "iovoid", "created_at": "2026-02-03T15:11:19Z", "draft": False, "labels": ["L1"]},
    6103: {"title": "test(l1): add storage trie branches reorg test", "author": "Himess", "created_at": "2026-02-03T09:43:33Z", "draft": False, "labels": []},
    6099: {"title": "refactor(l1): extract snapshot dumping helpers in snap client", "author": "pablodeymo", "created_at": "2026-02-02T19:26:19Z", "draft": False, "labels": ["L1", "snapsync"]},
    6097: {"title": "fix(l2): correctly update in-memory operations counts", "author": "GarmashAlex", "created_at": "2026-02-02T13:52:29Z", "draft": False, "labels": []},
    6095: {"title": "chore(l1, l2): enable clippy::unused_async lint", "author": "akronim26", "created_at": "2026-01-31T16:55:55Z", "draft": False, "labels": []},
    6077: {"title": "ci(l1): auto-trigger performance Docker build on merge of labeled PRs", "author": "ilitteri", "created_at": "2026-01-30T17:39:35Z", "draft": False, "labels": ["L1"]},
    6072: {"title": "perf(l1): reduce allocations in account range verification", "author": "MozirDmitriy", "created_at": "2026-01-29T20:17:38Z", "draft": False, "labels": []},
    6068: {"title": "feat(l1): add new reth metrics in grafana", "author": "Arkenan", "created_at": "2026-01-29T18:44:22Z", "draft": False, "labels": ["L1"]},
    6064: {"title": "fix(l1): prevent panic from legacy transaction v value overflow", "author": "SchnobiTobi", "created_at": "2026-01-29T13:20:56Z", "draft": False, "labels": []},
    6061: {"title": "fix(l1,l2): allow non-empty datadir without existing DB", "author": "Forostovec", "created_at": "2026-01-28T22:09:40Z", "draft": False, "labels": []},
    6060: {"title": "fix(l1): auto-switch from snap to full sync when node has synced state", "author": "ilitteri", "created_at": "2026-01-28T21:53:55Z", "draft": False, "labels": ["L1"]},
    6059: {"title": "perf(l1): parallelize header download with state download during snap sync", "author": "pablodeymo", "created_at": "2026-01-28T21:36:54Z", "draft": False, "labels": ["performance", "L1", "snapsync"]},
    6057: {"title": "perf(l1): use Bytes for trie values to enable O(1) clones", "author": "pablodeymo", "created_at": "2026-01-28T21:00:04Z", "draft": False, "labels": ["performance", "L1", "snapsync"]},
    6050: {"title": "docs(l1,l2): jemalloc memory profiling endpoint", "author": "Oppen", "created_at": "2026-01-28T13:54:48Z", "draft": False, "labels": ["L2", "L1"]},
    6045: {"title": "fix(levm): reorder fee token validations to prevent storage rollback issue", "author": "ilitteri", "created_at": "2026-01-27T19:47:46Z", "draft": False, "labels": ["levm", "audit"]},
    6044: {"title": "fix(levm): prevent gas underflow in privileged L2 transactions to precompiles", "author": "ilitteri", "created_at": "2026-01-27T18:20:54Z", "draft": False, "labels": ["levm", "audit"]},
    6043: {"title": "test(levm): add EIP-7702 delegation gas behavior tests", "author": "ilitteri", "created_at": "2026-01-27T17:30:01Z", "draft": False, "labels": ["levm", "audit"]},
    6036: {"title": "fix(l1): use shared decode_hex helper in eth_getCode client", "author": "prestoalvarez", "created_at": "2026-01-27T13:58:22Z", "draft": False, "labels": []},
    6031: {"title": "feat(l1): add HTTP/2 support for RPC servers", "author": "ilitteri", "created_at": "2026-01-26T20:53:37Z", "draft": False, "labels": ["L1"]},
    6029: {"title": "feat(metrics): add p50/p99 summary metrics", "author": "jrchatruc", "created_at": "2026-01-26T20:16:19Z", "draft": False, "labels": []},
    6025: {"title": "perf(l2): remove redundant Transaction clones", "author": "radik878", "created_at": "2026-01-26T17:30:20Z", "draft": False, "labels": []},
    6019: {"title": "ci(l2): add reproducible ELF builds with ere-compiler and minisign signing", "author": "ilitteri", "created_at": "2026-01-26T02:00:12Z", "draft": False, "labels": ["L2"]},
    6014: {"title": "feat(l2): add ZisK zkVM backend for L2 proving", "author": "tomip01", "created_at": "2026-01-23T18:59:29Z", "draft": False, "labels": ["L2"]},
    6013: {"title": "chore(deps): bump lodash from 4.17.21 to 4.17.23", "author": "dependabot[bot]", "created_at": "2026-01-23T18:41:28Z", "draft": False, "labels": ["dependencies", "javascript"]},
    6009: {"title": "chore(l1): update hive tests for Amsterdam fork", "author": "azteca1998", "created_at": "2026-01-23T10:16:56Z", "draft": False, "labels": ["L1"]},
    6007: {"title": "chore(l1): bump the cargo group across 2 directories with 1 update", "author": "dependabot[bot]", "created_at": "2026-01-22T23:15:54Z", "draft": False, "labels": ["L1", "dependencies", "rust"]},
    6006: {"title": "perf(l2): compute base blob fee once per block", "author": "xqft", "created_at": "2026-01-22T20:26:32Z", "draft": False, "labels": ["performance", "L2"]},
    5981: {"title": "perf(l1): add BOLT post-link optimization setup", "author": "Oppen", "created_at": "2026-01-21T22:17:31Z", "draft": False, "labels": ["performance", "L1"]},
    5967: {"title": "feat(l2): add zkvm-bench toolkit for zkVM optimization workflow", "author": "xqft", "created_at": "2026-01-21T18:18:39Z", "draft": False, "labels": ["L2"]},
    5951: {"title": "perf(l1): implement missing length functions", "author": "Arkenan", "created_at": "2026-01-20T19:23:27Z", "draft": False, "labels": ["performance", "L1"]},
    5933: {"title": "perf(l1,l2): use rwlock for trie_cache", "author": "Oppen", "created_at": "2026-01-20T17:57:20Z", "draft": False, "labels": ["performance", "L2", "L1"]},
    5908: {"title": "perf(l2): cache l2 metrics registry for gather", "author": "Fibonacci747", "created_at": "2026-01-19T15:25:42Z", "draft": False, "labels": []},
    5905: {"title": "refactor(l1,l2): extract duplicate witness generation logic to helper", "author": "Itodo-S", "created_at": "2026-01-19T13:22:27Z", "draft": False, "labels": []},
    5904: {"title": "feat(l1): add --p2p.bind-addr to separate bind and advertised addresses", "author": "catwith1hat", "created_at": "2026-01-19T06:38:32Z", "draft": False, "labels": []},
    5903: {"title": "perf(snap-sync): add 4 performance optimizations for faster sync", "author": "unbalancedparentheses", "created_at": "2026-01-19T02:56:01Z", "draft": False, "labels": ["performance"]},
    5887: {"title": "refactor(l2): move monitor to tooling workspace", "author": "ilitteri", "created_at": "2026-01-18T01:50:20Z", "draft": False, "labels": ["L2"]},
    5880: {"title": "docs(l1,l2,levm): add comprehensive performance optimization documentation", "author": "Arkenan", "created_at": "2026-01-16T18:43:14Z", "draft": False, "labels": ["levm", "L2", "L1"]},
    5872: {"title": "docs(l2): add zkVM ecosystem documentation", "author": "ilitteri", "created_at": "2026-01-16T14:18:50Z", "draft": False, "labels": ["L2"]},
    5867: {"title": "feat(trie): implement Erigon-style grid-based Patricia trie", "author": "diegokingston", "created_at": "2026-01-15T21:00:49Z", "draft": False, "labels": []},
    5865: {"title": "perf: add quick-win optimizations for storage and EVM", "author": "diegokingston", "created_at": "2026-01-15T19:08:30Z", "draft": False, "labels": []},
    5855: {"title": "fix(l2): align FeeToken tx_type metric label", "author": "Forostovec", "created_at": "2026-01-14T22:19:35Z", "draft": False, "labels": []},
    5844: {"title": "chore(l1, l2): export block number and execution latency together", "author": "jrchatruc", "created_at": "2026-01-14T15:55:01Z", "draft": False, "labels": ["L2", "L1"]},
    5830: {"title": "perf(l2): reduce allocations in blob reconstruction and blob utils", "author": "GarmashAlex", "created_at": "2026-01-13T20:23:51Z", "draft": False, "labels": []},
    5822: {"title": "perf(l1,l2): avoid unnecessary Arc::make_mut in trie iterator", "author": "Snezhkko", "created_at": "2026-01-13T12:40:39Z", "draft": False, "labels": []},
    5811: {"title": "fix(l1): avoid double authdata allocation in discv5 header", "author": "Bashmunta", "created_at": "2026-01-12T12:08:33Z", "draft": False, "labels": []},
    5808: {"title": "refactor(l1): reuse fork blob schedule in fee history loop", "author": "Bashmunta", "created_at": "2026-01-11T20:12:27Z", "draft": False, "labels": []},
    5807: {"title": "refactor(l2): use decode_hex utility in calldata module", "author": "sashaodessa", "created_at": "2026-01-11T18:30:18Z", "draft": False, "labels": []},
    5797: {"title": "fix(l2): remove redundant padding in generic call helper", "author": "MozirDmitriy", "created_at": "2026-01-09T16:58:05Z", "draft": False, "labels": []},
    5788: {"title": "test(l1): add fuzzing tests and security workflows", "author": "ManuelBilbao", "created_at": "2026-01-08T20:26:36Z", "draft": False, "labels": ["L1"]},
    5786: {"title": "feat(l1, l2): introduce a trait for vm tracers", "author": "lakshya-sky", "created_at": "2026-01-08T18:14:17Z", "draft": False, "labels": []},
    5783: {"title": "fix(l1): include sender in MempoolTransaction RLP encoding", "author": "Forostovec", "created_at": "2026-01-08T15:56:47Z", "draft": False, "labels": []},
    5748: {"title": "fix(l1,l2): avoid creating datadir when removing database", "author": "phrwlk", "created_at": "2026-01-06T19:04:18Z", "draft": False, "labels": []},
    5747: {"title": "fix(l1): add context to trie validation count mismatch error", "author": "RaveenaBhasin", "created_at": "2026-01-06T18:53:06Z", "draft": False, "labels": []},
    5742: {"title": "fix(l2): correctly accumulate operations counts in in-memory rollup store", "author": "Fibonacci747", "created_at": "2026-01-05T21:25:07Z", "draft": False, "labels": []},
    5740: {"title": "chore(deps): bump the cargo group across 3 directories with 1 update", "author": "dependabot[bot]", "created_at": "2026-01-05T14:21:40Z", "draft": False, "labels": ["dependencies", "rust"]},
    5736: {"title": "chore(l2): remove unused ELASTICITY MULTIPLIER import", "author": "Olexandr88", "created_at": "2026-01-04T13:58:50Z", "draft": False, "labels": []},
    5729: {"title": "fix(l2): log message typo", "author": "andreogle", "created_at": "2025-12-29T16:47:07Z", "draft": False, "labels": []},
    5728: {"title": "fix(l2): cache pending L1 messages in L1->L2 monitor widget", "author": "phrwlk", "created_at": "2025-12-29T16:28:35Z", "draft": False, "labels": []},
    5727: {"title": "docs: fix LEVM FAQ U256 casting example and error naming", "author": "radik878", "created_at": "2025-12-29T11:55:06Z", "draft": False, "labels": []},
    5725: {"title": "perf(l1,l2): dedup execution witness codes and trie nodes", "author": "Snezhkko", "created_at": "2025-12-24T11:03:48Z", "draft": False, "labels": []},
    5693: {"title": "feat(l1): expose fkv progress", "author": "iovoid", "created_at": "2025-12-19T19:33:36Z", "draft": False, "labels": ["L1"]},
    5687: {"title": "fix(l1): align mempool error messages with their actual usage", "author": "MozirDmitriy", "created_at": "2025-12-19T10:25:24Z", "draft": False, "labels": []},
    5684: {"title": "refactor(l1): reduce hashing in engine_getBlobsV2", "author": "lakshya-sky", "created_at": "2025-12-18T20:30:20Z", "draft": False, "labels": []},
    5682: {"title": "feat(l1): display ascii art during startup", "author": "MegaRedHand", "created_at": "2025-12-18T19:00:28Z", "draft": False, "labels": ["L1"]},
    5681: {"title": "fix(l1): add error handling to init_rpc_api()", "author": "figtracer", "created_at": "2025-12-18T18:56:30Z", "draft": False, "labels": []},
    5649: {"title": "perf(l1): remove redundant clones in RPC receipt building", "author": "phrwlk", "created_at": "2025-12-16T08:49:58Z", "draft": False, "labels": []},
    5641: {"title": "fix(l1): avoid doing collect in peer table get_contact functions", "author": "MegaRedHand", "created_at": "2025-12-15T18:19:53Z", "draft": False, "labels": ["L1"]},
    5628: {"title": "test(l1): add FCU test for StateNotReachable case", "author": "madisoncarter1234", "created_at": "2025-12-13T16:08:43Z", "draft": False, "labels": []},
    5627: {"title": "fix(l1): exit client on pre-merge fork instead of warning", "author": "madisoncarter1234", "created_at": "2025-12-13T16:04:09Z", "draft": False, "labels": []},
    5618: {"title": "chore(l1): move tools from levm into tooling folder", "author": "JereSalo", "created_at": "2025-12-12T17:21:37Z", "draft": False, "labels": []},
    5608: {"title": "fix(l2): validate block header and body match", "author": "MegaRedHand", "created_at": "2025-12-11T20:26:26Z", "draft": False, "labels": ["L2"]},
    5599: {"title": "refactor(l1): use threads::Genserver for synchronic threaded code", "author": "lakshya-sky", "created_at": "2025-12-11T01:30:32Z", "draft": False, "labels": []},
    5554: {"title": "feat(l2): introduce EncodedTrie as a zkVM performant MPT", "author": "xqft", "created_at": "2025-12-09T15:48:16Z", "draft": False, "labels": ["L2"]},
    5546: {"title": "chore(l1): use NodeRecordPairs to hold enr entries", "author": "lakshya-sky", "created_at": "2025-12-06T16:54:33Z", "draft": False, "labels": []},
    5537: {"title": "docs(l1,l2): add pre-release checklist section", "author": "ilitteri", "created_at": "2025-12-05T19:12:01Z", "draft": False, "labels": ["L2", "L1"]},
    5531: {"title": "refactor(l1): avoid extra allocations in RLPx handshake", "author": "Snezhkko", "created_at": "2025-12-05T12:35:50Z", "draft": False, "labels": []},
    5524: {"title": "docs: add checklist to PR template", "author": "Oppen", "created_at": "2025-12-04T18:48:15Z", "draft": False, "labels": []},
    5519: {"title": "chore(l1): allow PR title to begin with UPPERCASE", "author": "JereSalo", "created_at": "2025-12-04T15:12:17Z", "draft": False, "labels": ["L1"]},
    5483: {"title": "feat(l1): refactor chain config (#5233)", "author": "fmoletta", "created_at": "2025-12-01T22:50:54Z", "draft": False, "labels": ["L1"]},
    5469: {"title": "chore(l1): add trie hash bench", "author": "edg-l", "created_at": "2025-12-01T13:42:41Z", "draft": False, "labels": ["L1"]},
    5440: {"title": "feat(l2): add support for Pico backend", "author": "xqft", "created_at": "2025-11-27T13:54:31Z", "draft": False, "labels": ["L2"]},
    5438: {"title": "refactor(l1): introduce get_block_headers and bulk fetching in gas_tip_estimator", "author": "figtracer", "created_at": "2025-11-27T11:48:06Z", "draft": False, "labels": []},
    5415: {"title": "fix(l2): Fix EIP-4844 blob fee bump to use percentage scale", "author": "GarmashAlex", "created_at": "2025-11-25T12:04:13Z", "draft": False, "labels": ["L2"]},
    5414: {"title": "chore(l1): add rlp decode benches (part 1)", "author": "azteca1998", "created_at": "2025-11-25T11:33:38Z", "draft": False, "labels": ["L1"]},
    5413: {"title": "chore(l1): add rlp encode benches", "author": "edg-l", "created_at": "2025-11-25T08:58:15Z", "draft": False, "labels": ["L1"]},
    5406: {"title": "docs(l2): fixes broken links", "author": "letmehateu", "created_at": "2025-11-22T10:45:07Z", "draft": False, "labels": []},
    5404: {"title": "chore(l2): use ZisK SDK instead of subprocesses", "author": "xqft", "created_at": "2025-11-21T19:52:35Z", "draft": False, "labels": ["L2"]},
    5401: {"title": "feat(l1,l2): sort accounts by address in state test reports", "author": "FredPhilipy", "created_at": "2025-11-21T12:02:10Z", "draft": False, "labels": []},
    5376: {"title": "perf(levm): remove unnecessary calldata clone", "author": "phrwlk", "created_at": "2025-11-17T18:54:40Z", "draft": False, "labels": []},
    5373: {"title": "fix(l2): return None when branch becomes empty in remove", "author": "sashass1315", "created_at": "2025-11-17T11:05:59Z", "draft": False, "labels": []},
    5352: {"title": "chore(l1): Remove unnecessary NodeRef clone before compute_hash()", "author": "radik878", "created_at": "2025-11-14T15:08:46Z", "draft": False, "labels": []},
    5340: {"title": "chore: remove unused ThreadJoinError variant from TrieGenerationError", "author": "Fibonacci747", "created_at": "2025-11-13T21:58:20Z", "draft": False, "labels": []},
    5331: {"title": "feat(l1): add clear-data target for tooling/sync/Makefile", "author": "fmoletta", "created_at": "2025-11-13T20:07:22Z", "draft": False, "labels": ["L1"]},
    5291: {"title": "fix(l1,l2): make ARM code a bit more portable", "author": "Oppen", "created_at": "2025-11-12T14:19:41Z", "draft": False, "labels": ["L2", "L1"]},
    5249: {"title": "chore(levm): avoid duplicate BackupHook in L2 stateless_execute", "author": "Galoretka", "created_at": "2025-11-10T09:31:18Z", "draft": False, "labels": []},
    5210: {"title": "chore(l1,l2): remove dead Environment fields difficulty and block_blob_gas_used", "author": "Galoretka", "created_at": "2025-11-06T10:42:07Z", "draft": False, "labels": []},
    5207: {"title": "perf(l2): avoid redundant String allocations in BlocksTable::render", "author": "Forostovec", "created_at": "2025-11-05T22:45:10Z", "draft": False, "labels": []},
    5177: {"title": "perf(l1,l2): avoid redundant hashing with Code::from_hashed_bytecode and update call sites", "author": "VolodymyrBg", "created_at": "2025-11-04T13:36:41Z", "draft": False, "labels": []},
    5173: {"title": "perf(l2): remove unnecessary allocation and clone", "author": "radik878", "created_at": "2025-11-03T22:55:53Z", "draft": False, "labels": []},
    5158: {"title": "perf(levm): improve compatibility of blake2b NEON implementation", "author": "azteca1998", "created_at": "2025-11-03T09:48:27Z", "draft": False, "labels": ["performance", "levm"]},
    5157: {"title": "perf(l1): optimize Nibbles::skip_prefix to avoid tail allocation and remove redundant clone", "author": "GarmashAlex", "created_at": "2025-11-03T06:24:55Z", "draft": False, "labels": ["L1"]},
    5155: {"title": "fix(2): revert sealed batch if witness generation fails", "author": "Galoretka", "created_at": "2025-11-01T14:18:02Z", "draft": False, "labels": ["L2"]},
    5150: {"title": "chore(l1): review peer handler and misc logs", "author": "fedacking", "created_at": "2025-10-31T21:34:19Z", "draft": False, "labels": ["L1"]},
    5065: {"title": "chore(l1): bump jupyterlab from 4.4.4 to 4.4.8", "author": "dependabot[bot]", "created_at": "2025-10-27T16:53:45Z", "draft": False, "labels": ["L1", "dependencies"]},
    5030: {"title": "test(l1): parallelize test file parsing in state_v2 runner", "author": "crStiv", "created_at": "2025-10-23T20:52:54Z", "draft": False, "labels": []},
    4922: {"title": "fix(l1,l2): rename CLI arg --metrics to --metrics.enabled for naming consistency", "author": "FredPhilipy", "created_at": "2025-10-17T15:49:34Z", "draft": False, "labels": ["L2", "L1"]},
    4899: {"title": "fix(l2): correct deposit_erc20 log to say Depositing instead of Claiming", "author": "radik878", "created_at": "2025-10-16T09:28:14Z", "draft": False, "labels": ["L2"]},
    4873: {"title": "chore(l2): estimate L2 tps", "author": "xqft", "created_at": "2025-10-14T23:19:35Z", "draft": False, "labels": ["L2"]},
    4864: {"title": "feat(l1): initial WIP Nix flake.", "author": "samoht9277", "created_at": "2025-10-14T17:06:24Z", "draft": False, "labels": ["L1"]},
    4736: {"title": "fix(blockchain): avoid awaiting under payloads mutex in get_payload", "author": "Forostovec", "created_at": "2025-10-02T09:40:05Z", "draft": False, "labels": []},
    4727: {"title": "chore(l1, l2): use default options for rocksdb", "author": "jrchatruc", "created_at": "2025-10-01T17:44:34Z", "draft": False, "labels": ["L2", "L1"]},
    4709: {"title": "feat(l2): add typed Error enum, alias rkyv error, and guard missing ELF", "author": "radik878", "created_at": "2025-09-30T18:08:32Z", "draft": False, "labels": ["L2"]},
    4687: {"title": "docs(l2): add Upgrade an L2 guide and sidebar link", "author": "Galoretka", "created_at": "2025-09-29T15:39:43Z", "draft": False, "labels": ["L2"]},
    4629: {"title": "fix(l2): remove duplicate Following log in determine_new_status", "author": "Galoretka", "created_at": "2025-09-24T08:52:47Z", "draft": False, "labels": ["L2"]},
    4587: {"title": "feat(l2): risc0-ethereum-trie", "author": "xqft", "created_at": "2025-09-19T20:59:50Z", "draft": False, "labels": ["L2"]},
    4523: {"title": "fix(l2): correct Current Block off-by-one, unify Current Batch as next", "author": "radik878", "created_at": "2025-09-17T09:26:51Z", "draft": False, "labels": ["L2"]},
    4522: {"title": "feat(l2): implement /health for based components", "author": "FredPhilipy", "created_at": "2025-09-17T05:14:12Z", "draft": False, "labels": ["L2"]},
    4407: {"title": "fix(l1): prevent panic on oversized trie node responses", "author": "Fibonacci747", "created_at": "2025-10-03T04:18:08Z", "draft": False, "labels": []},
    4330: {"title": "feat(l2): change the block timestamp precision to ms", "author": "viktorking7", "created_at": "2025-10-01T14:44:49Z", "draft": False, "labels": []},
    4046: {"title": "perf(levm): use rug instead of big int to peform modexp", "author": "crStiv", "created_at": "2025-09-03T00:00:00Z", "draft": False, "labels": []},
    3621: {"title": "ci(l2): add sp1 proving for based test", "author": "avilagaston9", "created_at": "2025-08-01T00:00:00Z", "draft": False, "labels": []},
    3341: {"title": "feat(l2): full sync for based feature", "author": "tomip01", "created_at": "2025-07-01T00:00:00Z", "draft": False, "labels": []},
}


def count_approvals(reviews):
    """Count unique approvals (deduplicated by author, last state wins)."""
    author_state = {}
    for r in reviews:
        author = r["author"]
        # Skip bots
        if "[bot]" in author:
            continue
        author_state[author] = r["state"]
    approvals = sum(1 for s in author_state.values() if s == "APPROVED")
    return approvals, author_state


def has_user_approved(reviews, user):
    """Check if user's last review state is APPROVED."""
    author_state = {}
    for r in reviews:
        if r["author"] == user:
            author_state[user] = r["state"]
    return author_state.get(user) in ("APPROVED",)


def type_bonus(title):
    t = title.lower()
    if t.startswith("fix("):
        return 20
    if t.startswith("perf("):
        return 15
    if t.startswith("feat(") or t.startswith("refactor("):
        return 10
    if t.startswith("test("):
        return 5
    if any(t.startswith(p) for p in ["docs(", "chore(", "ci(", "style(", "deps(", "build(", "revert("]):
        return 0
    return 0


def age_bonus(created_at_str):
    created = datetime.fromisoformat(created_at_str.replace("Z", "+00:00"))
    days = (TODAY - created).days
    return min(20, days)


def size_penalty(additions, deletions):
    total = (additions or 0) + (deletions or 0)
    if total <= 50:
        return 0
    elif total <= 200:
        return -3
    elif total <= 500:
        return -6
    elif total <= 1000:
        return -9
    elif total <= 3000:
        return -12
    else:
        return -15


# Draft Reviews Posted PR numbers
draft_review_prs = {5788, 5867, 5736, 5727, 5524, 6164, 5352, 5340, 5249, 5740, 5210, 5546, 5554, 5786, 5177, 5376, 4330, 5865, 5830, 4522, 5742, 5415, 5155, 4922, 4899, 4629, 4523, 4407, 5687, 6107, 6112, 6159, 6114, 6151, 6152, 6147, 6103, 6110, 6095, 6120, 6009, 6121, 5414, 6156, 6099, 6108, 6146, 5413, 6050, 6059, 6131, 5440, 6126, 5618, 6109, 6029, 5469, 5406, 6057}

# Awaiting Response PR numbers
awaiting_response_prs = {6122, 5627, 6019, 5872, 6144, 5628, 5844, 5158, 5783, 5797, 5855, 5608, 5641, 5748, 4736, 5537, 5747, 5728, 5373, 5682, 5681, 5291, 5725, 5693, 5649, 5173, 5157, 5684, 6045, 5331, 6060, 5531, 5401, 4727, 5030, 5967, 5981, 5483, 5887, 5438, 5599, 4864}

# Already tracked PRs (should not appear in Pending Review)
tracked = draft_review_prs | awaiting_response_prs

# Find merged/closed tracked PRs
moved_to_resolved = []
for pr_num in tracked:
    st = states_data.get(pr_num, {})
    if st.get("merged"):
        moved_to_resolved.append((pr_num, "MERGED"))
    elif st.get("state") == "closed":
        moved_to_resolved.append((pr_num, "CLOSED"))


# Build pending review queue
pending = []
for pr_num, data in raw_prs.items():
    if data["draft"]:
        continue
    if data["author"] == GITHUB_USER:
        continue
    if pr_num in tracked:
        continue

    reviews = reviews_data.get(pr_num, [])
    approvals, author_states = count_approvals(reviews)

    if approvals >= 3:
        continue
    if has_user_approved(reviews, GITHUB_USER):
        continue

    additions = details_data.get(pr_num, {}).get("additions") or 0
    deletions = details_data.get(pr_num, {}).get("deletions") or 0

    tb = type_bonus(data["title"])
    ab = age_bonus(data["created_at"])
    sp = size_penalty(additions, deletions)
    approval_bonus = approvals * 5
    score = tb + ab + sp + approval_bonus

    labels_str = ", ".join(data["labels"]) if data["labels"] else ""
    created_short = data["created_at"][:10]

    pending.append({
        "number": pr_num,
        "title": data["title"],
        "additions": additions,
        "deletions": deletions,
        "approvals": approvals,
        "created": created_short,
        "priority": score,
        "labels": labels_str,
    })

# Sort by priority descending
pending.sort(key=lambda x: (-x["priority"], x["number"]))

# Output results
print("=== PENDING REVIEW TABLE ===")
print("| PR# | Title | +/- | Approvals | Created | Priority | Labels |")
print("|-----|-------|-----|-----------|---------|----------|--------|")
for p in pending:
    url = f"https://github.com/lambdaclass/ethrex/pull/{p['number']}"
    print(f"| [#{p['number']}]({url}) | {p['title']} | +{p['additions']}/-{p['deletions']} | {p['approvals']}/3 | {p['created']} | {p['priority']} | {p['labels']} |")

print()
print("=== MOVED TO RESOLVED ===")
for pr_num, state in moved_to_resolved:
    title = raw_prs.get(pr_num, {}).get("title", "unknown")
    print(f"{pr_num}|{state}|{title}")

print()
print("=== SUMMARY ===")
total_open = len(raw_prs)
filtered_out = total_open - len(pending)
print(f"Total open (non-draft, non-author): {total_open}")
print(f"Filtered out (3+ approvals, already approved, or tracked): {filtered_out}")
print(f"In queue: {len(pending)}")
print(f"Moved to resolved: {len(moved_to_resolved)}")

print()
print("=== TOP 5 ===")
for p in pending[:5]:
    url = f"https://github.com/lambdaclass/ethrex/pull/{p['number']}"
    print(f"#{p['number']} {url} (priority {p['priority']}) - {p['title']}")
