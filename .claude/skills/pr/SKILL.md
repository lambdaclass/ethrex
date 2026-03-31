# /pr — PR Review Pipeline

Unified command for the ethrex PR review pipeline.

**Usage:** `/pr <subcommand> [args]`

**Subcommands:**
- `refresh` — Refresh the list of open PRs and compute priority scores
- `review [NUMBER]` — Review a PR (or the top-priority one) and create a draft review
- `check` — Check for replies and activity on reviewed PRs
- `drafts` — Show pending draft reviews awaiting submission
- `status` — Show the review pipeline dashboard
- `run [N]` — Run the full review pipeline interactively (refresh → review N PRs → check responses)
- `help` — Show all subcommands and their descriptions

## Critical Rule

**NEVER approve, request changes, or submit a review.** All reviews MUST stay as PENDING drafts. Never include `"event"` in any GitHub review API call. The user reviews and submits manually via the GitHub UI.

## Instructions

Parse the first argument to determine the subcommand. Read and follow the corresponding file in this skill directory:

| Argument | File |
|----------|------|
| `refresh` | `refresh.md` |
| `review` | `review.md` |
| `check` | `check.md` |
| `drafts` | `drafts.md` |
| `status` | `status.md` |
| `run` | `run.md` |
| `help` | `help.md` |

If no argument is given, or the argument doesn't match any subcommand, read and follow `help.md` to show usage information.

Pass any remaining arguments (after the subcommand) through to the subcommand file's instructions. For example, `/pr review 6329` should pass `6329` as the PR number to `review.md`.

## Allowed Tools
Bash(gh *), Read, Write, Edit, Grep, Glob, WebFetch
