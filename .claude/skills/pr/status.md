# /pr status

Show a dashboard of the PR review pipeline status.

## Instructions

Read `~/.ethrex-reviews/state.md` and print a summary dashboard. No file modifications.

### Step 1: Read the state file

Read `~/.ethrex-reviews/state.md`.

If the file doesn't exist or is empty, say "No state file found. Run `/pr refresh` first." and stop.

### Step 2: Extract the "Last refreshed" timestamp

From line 2 of the file, extract the timestamp after `Last refreshed: `.

### Step 3: Parse each section

#### Pending Review
Lines between `## Pending Review` and the next `## ` header. Count rows in the markdown table (lines starting with `| [#`).

#### Draft Reviews Posted
Lines between `## Draft Reviews Posted` and `## Awaiting Response`. Each entry starts with `### [#NUM](URL) TITLE`. For each entry, extract:
- PR number and title from the `### ` heading
- Comment count from the `**Comments:**` line (e.g., "3 inline + body", "0 inline + body")
- The URL from the heading link

#### Awaiting Response
Lines between `## Awaiting Response` and `## Resolved`. Count rows in the markdown table (lines starting with `| [#`). Each row has columns: PR#, Title, Submitted, Last Checked, Activity, Review.

Group by the Activity column (second-to-last column):
- `none` — no activity since review submitted
- `new-commits` — author pushed new commits
- `replies` — author or others replied to review comments
- `replies+new-commits` — both of the above

Also extract the Review column (last column) for each active PR: `approved`, `changes`, `commented`, or `-`.

#### Resolved
Lines between `## Resolved` and end of file. Count rows in the markdown table (lines starting with `| [#`).

### Step 4: Print the dashboard

Print the dashboard in this exact format (adjust counts and entries to match actual data):

```
ethrex PR Review Dashboard
═══════════════════════════
Last refreshed: <timestamp>

Action needed:
  <N> draft reviews to submit
  ─────────────────────────
  #NNNN  <title>                                          (<comments>)
         https://github.com/lambdaclass/ethrex/pull/NNNN
  ...

  <N> PRs with author activity
  ─────────────────────────
  #NNNN  <title>                                          (new-commits) ✓approved
         https://github.com/lambdaclass/ethrex/pull/NNNN
  #NNNN  <title>                                          (replies)
         https://github.com/lambdaclass/ethrex/pull/NNNN
  ...

No action needed:
  <N> awaiting response (no activity)
  <N> pending review
  <N> resolved
```

Rules:
- Under "draft reviews to submit", list ALL draft review entries with their PR#, title, and comment count from the `**Comments:**` line.
- Under "PRs with author activity", list ONLY awaiting-response PRs where Activity is NOT `none`. Show the activity type in parentheses. If the Review column is `approved`, append ` ✓approved` after the activity tag.
- If a sub-section has 0 entries, show the count line but skip the detail lines and separator.
- The "No action needed" section is always summary counts only (no individual PR listings).
- Keep titles as-is from the state file (don't truncate).
