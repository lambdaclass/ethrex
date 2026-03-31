# /pr refresh

Refresh the list of open PRs in the ethrex repository, compute priority scores, and update the state file.

## Instructions

### Prerequisites

Detect the current GitHub user:
```bash
GITHUB_USER=$(gh api user --jq .login)
```
Use this as `GITHUB_USER` throughout the instructions below.

### Step 1: Fetch open PRs

Run:
```bash
gh pr list --repo lambdaclass/ethrex --state open --draft false --json number,title,additions,deletions,reviewDecision,reviews,labels,createdAt,author,statusCheckRollup,headRefName --limit 200
```

### Step 2: Filter PRs

Exclude PRs that match ANY of:
- `isDraft` is true
- Author login is `GITHUB_USER`
- Already has 3+ approvals (count reviews where `state == "APPROVED"`, deduplicated by author)
- Already approved by `GITHUB_USER`
- CI is failing: check `statusCheckRollup` — if the most recent entry per context has `conclusion != "SUCCESS"` and `conclusion != null` (pending is ok, failure is not). If `statusCheckRollup` is empty or null, include the PR (benefit of the doubt).

### Step 3: Compute priority scores

For each PR, compute a priority score:

**Type bonus** (from PR title prefix):
- `fix(` → +20
- `perf(` → +15
- `feat(` or `refactor(` → +10
- `test(` → +5
- `docs(`, `chore(`, `ci(`, `style(`, `deps(`, `build(`, `revert(` → +0

**Age bonus**: `min(20, days_since_created)`

**Size penalty**: Based on total changes (additions + deletions):
- 0-50: 0
- 51-200: -3
- 201-500: -6
- 501-1000: -9
- 1001-3000: -12
- 3000+: -15

**Approval bonus**: `approvals * 5` (PRs with more reviews are closer to merging, worth reviewing to push them over)

Final score = type_bonus + age_bonus + size_penalty + approval_bonus

### Step 4: Read existing state file

Read `~/.ethrex-reviews/state.md`. Preserve the "Draft Reviews Posted", "Awaiting Response", "Stale", and "Resolved" sections exactly as they are.

### Step 5: Check for merged/closed PRs

For any PRs in "Draft Reviews Posted", "Awaiting Response", or "Stale" sections, check if they've been merged or closed:
```bash
gh pr view <number> --repo lambdaclass/ethrex --json state
```
Move merged/closed PRs to the "Resolved" section with their final state.

### Step 6: Write updated state file

Update `~/.ethrex-reviews/state.md`:

- Set `Last refreshed:` to current UTC timestamp
- Replace the "Pending Review" section with the new priority-sorted table
- PRs that appear in "Draft Reviews Posted", "Awaiting Response", or "Stale" should NOT appear in "Pending Review" (they're already tracked)
- Keep all other sections as-is (with any merged/closed PRs moved to Resolved)
- Preserve the "Stale" section between "Awaiting Response" and "Resolved"

The Pending Review table format:
```
| PR# | Title | +/- | Approvals | Created | Priority | Labels |
|-----|-------|-----|-----------|---------|----------|--------|
| [#1234](url) | title here | +100/-50 | 1/3 | 2026-01-15 | 35 | l1 |
```

### Step 7: Print summary

Output a brief summary:
- Total PRs found / filtered / in queue
- Top 5 by priority (include full GitHub URL for each, e.g. `#5748 https://github.com/lambdaclass/ethrex/pull/5748`)
- Any PRs moved to Resolved

**Always include the full GitHub PR URL** whenever referencing a PR — this makes it clickable in the terminal.
