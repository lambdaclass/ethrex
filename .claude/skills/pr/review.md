# /pr review [NUMBER]

Review a pull request and create a GitHub draft review with inline comments.

**Arguments:** `[PR_NUMBER]` — optional. If omitted, pick the top unreviewed PR from the state file.

## Critical Rule

**NEVER approve, request changes, or submit a review.** All reviews MUST be created as PENDING drafts only. This means:
- Never include `"event"` in the review API payload (omitting it = PENDING)
- Never use `APPROVE`, `REQUEST_CHANGES`, or `COMMENT` as event values
- Never call the submit review endpoint
- The user reviews drafts and submits them manually via the GitHub UI

## Instructions

Read `review-guide.md` in this skill directory for what to look for and what to skip.

### Prerequisites

Detect the current GitHub user:
```bash
GITHUB_USER=$(gh api user --jq .login)
```
Use this as `GITHUB_USER` throughout the instructions below.

### Step 0: Determine PR number

If a PR number was provided as argument, use it. Otherwise:
1. Read `~/.ethrex-reviews/state.md`
2. Pick the first PR in the "Pending Review" table (highest priority)
3. If the table is empty, tell the user to run `/pr refresh` first and stop

### Step 1: Check if PR is reviewable

```bash
gh pr view PR_NUMBER --repo lambdaclass/ethrex --json isDraft,state --jq '{isDraft, state}'
```

- If `isDraft` is `true`, tell the user "PR #NNNN is a draft — skipping review." and stop.
- If `state` is `MERGED` or `CLOSED`, tell the user "PR #NNNN is already \<state\> — skipping review." and stop.

### Step 1b: Check for existing pending review

```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/reviews --jq '[.[] | select(.user.login == "GITHUB_USER" and .state == "PENDING")] | length'
```

If there's already a pending draft review, warn the user and stop. They should submit or dismiss it first.

### Step 2: Fetch PR information

Run these commands to gather context:
```bash
# PR metadata
gh pr view PR_NUMBER --repo lambdaclass/ethrex --json title,body,files,additions,deletions,headRefName,baseRefName,author,labels,commits

# Full diff
gh pr diff PR_NUMBER --repo lambdaclass/ethrex
```

### Step 3: Read relevant source files

For each file in the diff:
- Read the full file from the local codebase to understand context (make sure you're on the right branch or use the file content from the diff)
- Look at surrounding code not in the diff to understand usage patterns
- For modified functions, check callers if the signature or behavior changed

Use Glob and Grep to explore the codebase as needed. Be thorough — a good review requires understanding the context, not just reading the diff.

### Step 4: Analyze the changes

Following the review guide, look for:
- Bugs, logic errors, edge cases
- Ethereum-specific correctness issues (RLP symmetry, gas accounting, fork boundaries)
- Concurrency issues
- Security concerns
- Performance issues (only egregious ones)

For style, naming, formatting, and docs issues, prefix comments with "nit:" to signal lower priority.

### Step 5: Build the review payload

Construct a JSON payload for the GitHub API. **Important:** Do NOT include an `"event"` field — omitting it creates a PENDING (draft) review. The valid event values are `APPROVE`, `REQUEST_CHANGES`, `COMMENT`, but we want none of those.

**Keep the review body concise.** The inline comments carry the detail — the body should be a short summary (2-4 sentences max) that gives the PR author the big picture without repeating what the inline comments already say. Mention the most important finding and any observations not tied to specific lines. Don't restate every inline comment in the body.

```json
{
  "body": "## Review Summary\n\n<2-4 sentence overall assessment, referencing key inline comments by line number rather than restating them>\n\n<any findings not tied to specific diff lines>",
  "comments": [
    {
      "path": "relative/path/to/file.rs",
      "line": 42,
      "side": "RIGHT",
      "body": "The specific comment about this line"
    }
  ]
}
```

**Critical: Line number calculation for inline comments**

Inline comments can ONLY be placed on lines that appear in the diff. You must:

1. Parse the diff for each file to find valid line ranges
2. Diff hunk headers look like: `@@ -old_start,old_count +new_start,new_count @@`
3. The `line` field in the comment must be a line number in the NEW version of the file (right side)
4. Only lines that appear in the diff hunks (added lines `+`, context lines ` `, or removed lines `-`) are valid targets
5. For comments on removed lines, use `"side": "LEFT"` and the OLD file line number
6. For comments on added or context lines, use `"side": "RIGHT"` and the NEW file line number

**If a finding is on a line NOT in the diff**, include it in the review body instead of as an inline comment. Prefix with the file path and line number like: `**file.rs:123** — description of issue`.

**If there are zero findings worth commenting on**, create a short approving review body with no inline comments. Say something like "Looks good. [brief note about what you checked]". Still omit the `event` field so it stays as a PENDING draft — the user can decide whether to submit as approval or comment.

### Step 6: Submit the draft review

**Important: Use the `Write` tool (NOT `cat >` or shell redirections) to create temp files.** Shell redirections like `cat > /tmp/file` and `> /tmp/file` are blocked in headless mode. Always use the Write tool:

1. Use the **Write** tool to write the JSON payload to `/tmp/review_pr.json` (always the same file — avoids "create new file" prompts after first use)
2. Submit via `gh api`:
```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/reviews \
  --method POST \
  --input /tmp/review_pr.json
```
3. Capture the review ID from the response (`id` field).

**Handling 422 errors (invalid line numbers):**

If the API returns a 422 (typically "pull_request_review_thread.line must be part of the diff" or similar), the line number calculation was wrong for one or more comments. Recover as follows:

1. Try submitting with just the review body and no inline comments to confirm the body works
2. Then add inline comments back one at a time (or in small batches) to identify which ones are invalid
3. For any comment that fails, move it to the review body as: `**path/to/file.rs:LINE** — comment text`

This ensures no findings are lost even if line mapping is wrong — they just end up in the body instead of inline.

### Step 7: Validate comment placement

After the draft review is successfully submitted, fetch the created comments back from GitHub and verify they landed on the intended code:

```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/reviews/REVIEW_ID/comments \
  --jq '.[] | {id, path, line, side, body: (.body | .[0:80]), diff_hunk: (.diff_hunk | split("\n") | last)}'
```

For each comment:
1. **Check `diff_hunk`**: GitHub returns the diff context where the comment was placed. The last line of `diff_hunk` is the actual code line the comment is attached to. Verify this matches the code you intended to comment on.
2. **Check `path` and `line`**: Confirm they match your intended target.
3. **Flag mismatches**: If the `diff_hunk` trailing line doesn't match the code you were commenting about, the comment landed on the wrong line.

**Auto-fix misplaced comments:**

For each misplaced comment:
1. Search the diff for the actual code line you intended to comment on — scan the diff hunks for that file to find the correct line number
2. If found: delete the misplaced comment and re-create it at the correct line:
   ```bash
   # Delete misplaced comment
   gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/comments/COMMENT_ID \
     --method DELETE

   # Re-create at correct line (as a standalone review comment)
   gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/comments \
     --method POST \
     -f body="comment text" \
     -f path="file.rs" \
     -F line=CORRECT_LINE \
     -f side="RIGHT" \
     -f commit_id="$(gh pr view PR_NUMBER --repo lambdaclass/ethrex --json headRefOid --jq .headRefOid)"
   ```
3. If the correct line can't be found in the diff: delete the misplaced comment and append the finding to the review body instead:
   ```bash
   # Delete misplaced comment
   gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/comments/COMMENT_ID \
     --method DELETE
   ```
   Then update the review body to include: `**file.rs:LINE** — comment text`
   ```bash
   gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/reviews/REVIEW_ID \
     --method PUT \
     -f body="updated body with moved comments"
   ```

**Generate a validation report** and append it to the state file entry for this review:

```markdown
<details>
<summary>Line placement validation</summary>

**Result:** X/Y comments correctly placed, Z auto-fixed

| # | File | Line | Expected code | Actual code | Status |
|---|------|------|---------------|-------------|--------|
| 1 | path/file.rs | 42 | `let x = foo();` | `let x = foo();` | OK |
| 2 | path/file.rs | 87 | `return Err(...)` | `}` | FIXED → line 92 |
| 3 | path/file.rs | 15 | `let gas = ...` | `use std::io;` | MOVED TO BODY |

</details>
```

This validation step may be removed in the future once line mapping proves reliable.

### Step 8: Update state file

Read `~/.ethrex-reviews/state.md` and update it:

1. Remove this PR from the "Pending Review" table
2. Add it to "Draft Reviews Posted" section with the following format (include the validation report from Step 7):

```markdown
### [#NUMBER](url) Title
- **Draft created:** TIMESTAMP
- **Review ID:** ID
- **Comments:** N inline + body

<details>
<summary>Full review text</summary>

**Body:**
> the review body text

**Inline comments:**
- `path/to/file.rs:LINE` — the comment text
- `path/to/file.rs:LINE` — the comment text

</details>
```

### Step 9: Print summary

Output:
- PR number, title, and full GitHub URL (e.g. `https://github.com/lambdaclass/ethrex/pull/1234`) on its own line for easy clicking
- Number of inline comments + body findings
- Brief list of the findings (one line each)
- Reminder that this is a DRAFT review — user must go to GitHub to review/edit/submit it

**Always include the full GitHub PR URL** whenever referencing a PR — this makes it clickable in the terminal.
