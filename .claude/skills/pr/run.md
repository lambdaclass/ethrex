# /pr run [N]

Run the full PR review pipeline interactively: refresh → review → check responses.

**Arguments:** `[N]` — number of PRs to review. Defaults to `all` (review every pending PR).

## Instructions

This runs the same pipeline as `review-cron.sh` but interactively within the current Claude session, avoiding the permission issues of spawning separate `claude -p` sessions.

### Prerequisites

Detect the current GitHub user:
```bash
GITHUB_USER=$(gh api user --jq .login)
```

### Step 1: Update repository

```bash
git fetch origin main 2>&1 || echo "WARNING: git fetch failed, continuing with local code"
```

### Step 2: Refresh PR list

Follow the instructions in `refresh.md` (same as `/pr refresh`). This updates the state file with the latest open PRs and priority scores.

After refresh, count the pending PRs from the state file.

### Step 3: Review PRs

Determine how many PRs to review:
- If the argument is a number N, review the top N pending PRs
- If the argument is `all` or omitted, review all pending PRs

For each PR to review (in priority order):
1. Follow the instructions in `review.md` (same as `/pr review`)
2. Print a separator line between reviews
3. If a review fails or times out (taking too long), log the error and continue to the next PR

After each review, re-read the state file to get the updated "Pending Review" table (the reviewed PR should have moved to "Draft Reviews Posted").

### Step 4: Check responses

Follow the instructions in `check.md` (same as `/pr check`). This checks for activity on previously reviewed PRs.

### Step 5: Print pipeline summary

Output a final summary:
```
═══════════════════════════
Pipeline complete
═══════════════════════════
  Refreshed: <timestamp>
  PRs reviewed: N
  Draft reviews created: N (list PR numbers)
  Response checks: N PRs checked, M with activity
═══════════════════════════
```

Include the full GitHub URL for each PR that got a new draft review.
