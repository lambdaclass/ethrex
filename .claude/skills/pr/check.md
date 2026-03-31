# /pr check

Check for replies and activity on PRs where we've submitted reviews.

## Instructions

### Prerequisites

Detect the current GitHub user:
```bash
GITHUB_USER=$(gh api user --jq .login)
```
Use this as `GITHUB_USER` throughout the instructions below.

### Step 1: Read state file and classify entries

Read `~/.ethrex-reviews/state.md`. Identify PRs in the "Awaiting Response" section.

If the section is empty, say "No PRs awaiting response" and stop.

**Deduplicate first:** If any PR number appears more than once in the table, keep only the newest entry (by Submitted date) and remove duplicates.

**Classify entries into tiers by age** (days since Submitted date):
- **Active** (< 7 days old): Full deep-check
- **Recent** (7–30 days old): State-only check
- **Stale** (> 30 days old): Fast batch check only

### Step 2: Batch state check (all tiers)

Do a single bulk check to determine which PRs are still open. Fetch the list of open PRs once:

```bash
gh pr list --repo lambdaclass/ethrex --state open --json number --limit 300 --jq '.[].number'
```

Compare against ALL entries in Awaiting Response. Any PR NOT in the open list is either merged or closed — verify individually:

```bash
gh pr view PR_NUMBER --repo lambdaclass/ethrex --json state --jq .state
```

Move merged/closed PRs to the "Resolved" section immediately.

### Step 3: Fast-check stale entries

For **Stale** entries (> 30 days) that are still open after Step 2, do a lightweight activity check to catch revamped PRs:

```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER --jq '{updated_at: .updated_at, comments: .comments, review_comments: .review_comments}'
```

Run these calls in parallel (multiple `gh api` calls in a single message, NOT a for loop).

**Promote to Active tier** if the PR's `updated_at` is within the last 7 days, OR if comment/review_comment counts increased since last check. These will get the full deep-check in Step 5.

All remaining stale entries: update "Last Checked" to now, keep Activity as-is, and leave them in place.

### Step 4: Check Recent entries (7–30 days, state-only)

For **Recent** entries still open after Step 2:

Check for new commits only (no reply checking, no deep analysis):
```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/commits --jq 'last.commit.committer.date'
```

Run these in parallel. Compare latest commit date against Submitted date. Update Activity to `new-commits` if newer commits exist, otherwise keep as-is.

Also check our review state:
```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/reviews --jq '[.[] | select(.user.login == "GITHUB_USER")] | sort_by(.submitted_at) | last | .state'
```

Update "Last Checked" to now.

### Step 5: Deep-check Active entries (< 7 days)

For **Active** entries (including any stale entries promoted in Step 3):

#### 5a. Check PR state (already done in Step 2)

#### 5b. Check for new commits since our review
```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/commits --jq 'last.commit.committer.date'
```
Compare the latest commit date against the "Submitted" date in our state file. If there are newer commits, flag as "new commits pushed".

#### 5c. Check for replies to our review comments
```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/comments --jq '.[] | {id, user: .user.login, body: .body, in_reply_to_id: .in_reply_to_id, created_at: .created_at, path: .path, line: .line}'
```

Look for comments that:
- Have `in_reply_to_id` matching one of our comment IDs
- Were created after our review submission date
- Are from users other than `GITHUB_USER`

#### 5d. Check issue-level comments
```bash
gh api repos/lambdaclass/ethrex/issues/PR_NUMBER/comments --jq '.[] | {user: .user.login, body: .body, created_at: .created_at}'
```

Look for comments after our review date that might reference our feedback.

#### 5e. Update state

Update Activity to one of:
- `none` — no new activity
- `replies` — direct replies to our comments
- `new-commits` — new commits pushed after our review
- `replies+new-commits` — both

Also check our review state:
```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/reviews --jq '[.[] | select(.user.login == "GITHUB_USER")] | sort_by(.submitted_at) | last | .state'
```

Set the **Review** column to one of:
- `approved` — our latest review is APPROVED
- `changes` — our latest review is CHANGES_REQUESTED
- `commented` — our latest review is COMMENTED
- `-` — no review found or other state

Update "Last Checked" to now.

### Step 6: Deep analysis for Active PRs with activity

For each Active PR flagged with `new-commits`, `replies`, or `replies+new-commits` in Step 5, do a deeper analysis to determine whether our review comments were actually addressed.

#### 6a. Retrieve our original comments

Get the full text of our review comments from the state file's "Draft Reviews Posted" history or from the GitHub API:
```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/comments --jq '.[] | select(.user.login == "GITHUB_USER") | {id, path, line, body}'
```

#### 6b. Fetch the latest diff and changed files

```bash
gh pr diff PR_NUMBER --repo lambdaclass/ethrex
```

For files we commented on, also fetch the full file from the PR branch to see the current state:
```bash
gh api "repos/lambdaclass/ethrex/contents/PATH" -X GET -F ref=BRANCH_NAME --jq '.content' | base64 -d
```

Get the branch name with:
```bash
gh pr view PR_NUMBER --repo lambdaclass/ethrex --json headRefName --jq .headRefName
```

#### 6c. Check each comment against the code and replies

For each of our review comments:

1. **Check replies:** Look at any direct reply threads (from step 5c). Did the author acknowledge, disagree, or explain?
2. **Check code changes:** Compare the new diff/file state against what our comment asked for. Did the code change in the way we suggested?

Classify each comment as one of:
- **addressed** — the code was changed to fix the issue, or the reply explains why it's already correct
- **partially addressed** — some aspect was fixed but not all
- **acknowledged** — author replied but hasn't changed the code yet
- **not addressed** — no reply and no code change related to this comment
- **wont-fix** — author explicitly declined the suggestion

#### 6d. Produce a per-PR comment status table

For each PR with activity, include a comment status table in the summary output:

```
#6122  | new commits      | 2/6 comments addressed
       https://github.com/lambdaclass/ethrex/pull/6122

  Comment status:
  1. report.rs:117 — fork list duplication       → not addressed
  2. report.rs:287 — NaN on zero fork_tests      → partially addressed (slack/github fixed, shell not)
```

### Step 7: Auto-archive

After all checks, auto-archive entries that meet these criteria:

1. **Stale + no activity:** Entries > 30 days old with Activity = `none` → move to "Stale" section
2. **Approved + no activity after 7 days:** Entries with Review = `approved` AND Activity = `none` AND Submitted > 7 days ago → move to "Stale" section

The **Stale** section sits between "Awaiting Response" and "Resolved":

```markdown
## Stale
<!-- Old reviewed PRs with no activity. Fast-checked periodically for revamps. -->
| PR# | Title | Submitted | Last Checked | Activity | Review |
|-----|-------|-----------|--------------|----------|--------|
```

If the Stale section doesn't exist yet, create it.

**Important:** Stale entries are NOT deleted — they're moved to the Stale section where Step 3's fast-check can catch them if they get revamped (promoted back to Active).

### Step 8: Also check Draft Reviews Posted

Check if any PRs in "Draft Reviews Posted" have had their draft review submitted (no longer in PENDING state):

```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/reviews --jq '[.[] | select(.user.login == "GITHUB_USER")] | sort_by(.submitted_at) | last | .state'
```

If the review state is no longer "PENDING" (e.g., "COMMENTED", "APPROVED", "CHANGES_REQUESTED"), move the PR from "Draft Reviews Posted" to "Awaiting Response" with the submission timestamp. Record the review state (approved/changes/commented).

### Step 9: Print summary

Start with a **"Draft Reviews Submitted"** section listing any drafts that were submitted since the last check:

```
### Draft Reviews Submitted

#6190  | changes_requested | 4 inline + body
       https://github.com/lambdaclass/ethrex/pull/6190
```

Then output the **Activity** section — only Active PRs with actual activity (skip `none`):

```
### Activity

#1234  | 2 replies        | @author replied to gas accounting comment
       https://github.com/lambdaclass/ethrex/pull/1234
#5678  | new commits      | 2/5 comments addressed
       https://github.com/lambdaclass/ethrex/pull/5678
```

For PRs with activity (from Step 6), include the per-comment status table.
For PRs with replies, show the reply text briefly (first 100 chars).

Then a **Stats** line:
```
### Stats
- Checked: X active, Y recent, Z stale
- Resolved: N (merged/closed)
- Archived to Stale: M
- Promoted from Stale: P
- Remaining in Awaiting Response: R
```

**Always include the full GitHub PR URL** whenever referencing a PR — this makes it clickable in the terminal.

### Awaiting Response table format

```
| PR# | Title | Submitted | Last Checked | Activity | Review |
|-----|-------|-----------|--------------|----------|--------|
| [#1234](url) | title | 2026-02-05 | 2026-02-06T15:00:00Z | replies | approved |
```
