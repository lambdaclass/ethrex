#!/usr/bin/env python3
"""Update the state file with new Pending Review queue and move merged/closed PRs."""
import re

STATE_FILE = "/Users/esteban/.ethrex-reviews/state.md"

with open(STATE_FILE, "r") as f:
    content = f.read()

# 1. Update timestamp
content = re.sub(r"Last refreshed: .*", "Last refreshed: 2026-02-10T19:50:00Z", content)

# 2. Replace Pending Review section
new_pending = """## Pending Review
<!-- Priority-sorted table. Columns: PR#, Title, +/-, Approvals, Created, Priority, Labels -->
| PR# | Title | +/- | Approvals | Created | Priority | Labels |
|-----|-------|-----|-----------|---------|----------|--------|
| [#5822](https://github.com/lambdaclass/ethrex/pull/5822) | perf(l1,l2): avoid unnecessary Arc::make_mut in trie iterator | +4/-4 | 1/3 | 2026-01-13 | 40 |  |
| [#6036](https://github.com/lambdaclass/ethrex/pull/6036) | fix(l1): use shared decode_hex helper in eth_getCode client | +1/-7 | 1/3 | 2026-01-27 | 38 |  |
| [#5207](https://github.com/lambdaclass/ethrex/pull/5207) | perf(l2): avoid redundant String allocations in BlocksTable::render | +4/-4 | 0/3 | 2025-11-05 | 35 |  |
| [#6025](https://github.com/lambdaclass/ethrex/pull/6025) | perf(l2): remove redundant Transaction clones | +5/-5 | 1/3 | 2026-01-26 | 34 |  |
| [#5905](https://github.com/lambdaclass/ethrex/pull/5905) | refactor(l1,l2): extract duplicate witness generation logic to helper | +40/-99 | 1/3 | 2026-01-19 | 32 |  |
| [#6061](https://github.com/lambdaclass/ethrex/pull/6061) | fix(l1,l2): allow non-empty datadir without existing DB | +16/-6 | 0/3 | 2026-01-28 | 32 |  |
| [#5951](https://github.com/lambdaclass/ethrex/pull/5951) | perf(l1): implement missing length functions | +559/-57 | 1/3 | 2026-01-20 | 31 | performance, L1 |
| [#6116](https://github.com/lambdaclass/ethrex/pull/6116) | perf(rpc): avoid decode+re-encode in newPayload transactions root | +18/-3 | 2/3 | 2026-02-03 | 31 | performance |
| [#4046](https://github.com/lambdaclass/ethrex/pull/4046) | perf(levm): use rug instead of big int to peform modexp | +151/-314 | 0/3 | 2025-09-03 | 29 |  |
| [#5908](https://github.com/lambdaclass/ethrex/pull/5908) | perf(l2): cache l2 metrics registry for gather | +121/-107 | 0/3 | 2026-01-19 | 29 |  |
| [#6064](https://github.com/lambdaclass/ethrex/pull/6064) | fix(l1): prevent panic from legacy transaction v value overflow | +50/-4 | 0/3 | 2026-01-29 | 28 |  |
| [#4709](https://github.com/lambdaclass/ethrex/pull/4709) | feat(l2): add typed Error enum, alias rkyv error, and guard missing ELF | +51/-9 | 0/3 | 2025-09-30 | 27 | L2 |
| [#5904](https://github.com/lambdaclass/ethrex/pull/5904) | feat(l1): add --p2p.bind-addr to separate bind and advertised addresses | +44/-7 | 0/3 | 2026-01-19 | 27 |  |
| [#6097](https://github.com/lambdaclass/ethrex/pull/6097) | fix(l2): correctly update in-memory operations counts | +26/-5 | 0/3 | 2026-02-02 | 27 |  |
| [#6072](https://github.com/lambdaclass/ethrex/pull/6072) | perf(l1): reduce allocations in account range verification | +4/-8 | 0/3 | 2026-01-29 | 26 |  |
| [#6123](https://github.com/lambdaclass/ethrex/pull/6123) | fix(l2): normalize error codes in based OnChainProposer | +3/-3 | 0/3 | 2026-02-04 | 25 |  |
| [#6124](https://github.com/lambdaclass/ethrex/pull/6124) | fix(p2p): store validated ENR from handshake in peer table | +7/-0 | 0/3 | 2026-02-04 | 25 |  |
| [#5903](https://github.com/lambdaclass/ethrex/pull/5903) | perf(snap-sync): add 4 performance optimizations for faster sync | +1044/-14 | 0/3 | 2026-01-19 | 23 | performance |
| [#6013](https://github.com/lambdaclass/ethrex/pull/6013) | chore(deps): bump lodash from 4.17.21 to 4.17.23 | +3/-3 | 1/3 | 2026-01-23 | 22 | dependencies, javascript |
| [#4873](https://github.com/lambdaclass/ethrex/pull/4873) | chore(l2): estimate L2 tps | +514/-168 | 2/3 | 2025-10-14 | 21 | L2 |
| [#3621](https://github.com/lambdaclass/ethrex/pull/3621) | ci(l2): add sp1 proving for based test | +251/-70 | 1/3 | 2025-08-01 | 19 |  |
| [#3341](https://github.com/lambdaclass/ethrex/pull/3341) | feat(l2): full sync for based feature | +2260/-173 | 0/3 | 2025-07-01 | 18 |  |
| [#4587](https://github.com/lambdaclass/ethrex/pull/4587) | feat(l2): risc0-ethereum-trie | +1336/-161 | 0/3 | 2025-09-19 | 18 | L2 |
| [#4687](https://github.com/lambdaclass/ethrex/pull/4687) | docs(l2): add Upgrade an L2 guide and sidebar link | +110/-0 | 0/3 | 2025-09-29 | 17 | L2 |
| [#5404](https://github.com/lambdaclass/ethrex/pull/5404) | chore(l2): use ZisK SDK instead of subprocesses | +1205/-337 | 0/3 | 2025-11-21 | 8 | L2 |
| [#6168](https://github.com/lambdaclass/ethrex/pull/6168) | chore: fix "unsuported fork" typo in whole repo | +16/-16 | 0/3 | 2026-02-10 | -1 |  |"""

# Find the Pending Review section and replace it
pending_start = content.find("## Pending Review")
draft_start = content.find("## Draft Reviews Posted")
if pending_start >= 0 and draft_start >= 0:
    content = content[:pending_start] + new_pending + "\n\n\n" + content[draft_start:]

# 3. Remove #6110 from Draft Reviews Posted
# Find the ### [#6110] block and remove it completely
pattern_6110_start = content.find("### [#6110]")
if pattern_6110_start >= 0:
    # Find next ### or ## section
    next_section = content.find("###", pattern_6110_start + 10)
    if next_section < 0:
        next_section = content.find("## ", pattern_6110_start + 10)
    if next_section >= 0:
        content = content[:pattern_6110_start] + content[next_section:]

# 4. Remove #6122 from Awaiting Response
content = content.replace("| [#6122](https://github.com/lambdaclass/ethrex/pull/6122) | chore(l1): amsterdam daily slack report | 2026-02-06T20:02:18Z | 2026-02-10T14:15:00Z | new-commits |\n", "")

# 5. Add to Resolved section
resolved_marker = "<!-- Merged/closed/done PRs. -->"
resolved_table_header = "| PR# | Title | Final State | Date |\n|-----|-------|-------------|------|\n"
new_resolved_entries = "| [#6110](https://github.com/lambdaclass/ethrex/pull/6110) | docs(l1): forks team roadmap | MERGED | 2026-02-10 |\n| [#6122](https://github.com/lambdaclass/ethrex/pull/6122) | chore(l1): amsterdam daily slack report | CLOSED | 2026-02-10 |\n"

# Insert new resolved entries after the table header
resolved_insert_pos = content.find(resolved_table_header)
if resolved_insert_pos >= 0:
    insert_at = resolved_insert_pos + len(resolved_table_header)
    content = content[:insert_at] + new_resolved_entries + content[insert_at:]

with open(STATE_FILE, "w") as f:
    f.write(content)

print("State file updated successfully.")
print(f"File: {STATE_FILE}")
