# /pr help

Show all available subcommands and their descriptions.

## Instructions

Print the following help text:

```
/pr — ethrex PR Review Pipeline
════════════════════════════════

Usage: /pr <subcommand> [args]

Subcommands:

  refresh          Refresh the list of open PRs and compute priority scores.
                   Fetches all open non-draft PRs, filters out already-approved
                   and CI-failing ones, scores by type/age/size/approvals, and
                   updates the state file.

  review [NUMBER]  Review a PR and create a GitHub draft review with inline
                   comments. If NUMBER is omitted, picks the top-priority
                   unreviewed PR from the state file.

  check            Check for replies and activity on PRs where we've posted
                   reviews. Deep-checks PRs with new commits or replies to
                   determine if our comments were addressed.

  drafts           Show all pending draft reviews awaiting manual submission
                   on GitHub.

  status           Show a dashboard of the full review pipeline: pending,
                   drafts, awaiting response, and resolved PRs.

  run [N]          Run the full pipeline interactively: refresh → review N
                   PRs → check responses. Defaults to reviewing all pending
                   PRs. Replaces the cron-based approach.

  help             Show this help text.

State file: ~/.ethrex-reviews/state.md
Review guide: .claude/skills/pr/review-guide.md
```
