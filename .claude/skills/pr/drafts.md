# /pr drafts

Show all pending draft reviews awaiting your approval for submission.

## Instructions

### Prerequisites

Detect the current GitHub user:
```bash
GITHUB_USER=$(gh api user --jq .login)
```
Use this as `GITHUB_USER` throughout the instructions below.

1. Read `~/.ethrex-reviews/state.md`
2. Parse the "Draft Reviews Posted" section
3. For each entry, extract: PR number, title, draft created date, comment count
4. Print a clean summary with a clickable link per PR:

```
Draft reviews awaiting submission:

#5701  fix(l1): fix InvalidPayloadAttributes RPC message  (0 inline, body only)
       https://github.com/lambdaclass/ethrex/pull/5701

#5641  fix(l1): avoid collect in peer table  (2 inline + body)
       https://github.com/lambdaclass/ethrex/pull/5641
```

**Always include the full GitHub URL on its own line under each PR** — this makes it clickable in the terminal.

4. If the section is empty, say "No pending draft reviews."

5. Optionally, if you want to verify against GitHub (only if the user asks), check each review is still PENDING:
```bash
gh api repos/lambdaclass/ethrex/pulls/PR_NUMBER/reviews --jq '[.[] | select(.user.login == "GITHUB_USER" and .state == "PENDING")] | length'
```
