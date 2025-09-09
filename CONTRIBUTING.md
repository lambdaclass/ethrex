# Contributing Guidelines

Thank you for your interest in contributing to ethrex! Please read the following guidelines to help us review and accept your contributions smoothly.

## How to Submit a Pull Request

1. Fork the repository and create your branch from `main`.
2. Make your changes, following the code style guidelines below.
3. Run tests locally to ensure nothing is broken.
4. Open a pull request with a descriptive title (see PR naming rules below).
5. Fill in the PR template if available, and link related issues.

## Pull Request Naming Rules

All PR titles must follow the enforced semantic format:

`<type>(<scope>[,<scope2>,...]): <subject>`

- **Allowed types:** `feat`, `fix`, `perf`, `refactor`, `revert`, `deps`, `build`, `ci`, `test`, `style`, `chore`, `docs`
- **Allowed scopes:** `l1`, `l2`, `levm` (multiple scopes can be used, separated by commas)
- **Scope(s) are required**
- **Subject must not start with an uppercase character**

**Examples:**
- `fix(l1,levm): handle edge case in snap sync`
- `docs(l2): update documentation for L2`

PRs not following this convention will fail automated checks.

## Commit Signature Verification

All commits must have a verified signature.

- Sign your commits using GPG or SSH so that GitHub marks them as 'Verified'.
- Unsigned or unverified commits may be rejected during review.
- For instructions, see [GitHub: Signing commits](https://docs.github.com/en/authentication/managing-commit-signature-verification/signing-commits).

## Issue Reporting

- Use GitHub Issues to report bugs or request features.
- Include steps to reproduce, expected behavior, and environment details.

## Review Process

- All PRs require review and approval by multiple maintainers.
- You may be asked to make changes before merging.
- Automated checks must pass before merge.


## Developer Documentation

For detailed guides, architecture, and API references, see the [Developer Docs](docs/developers/README.md).

### Documentation Contributions

- See [`docs/CONTRIBUTING_DOCS.md`](docs/CONTRIBUTING_DOCS.md) for documentation-specific guidelines.


## Getting Started: Good First Issues

If you're new to ethrex, the best way to get involved is by tackling a "good first issue". These are beginner-friendly tasks that help you learn the codebase and contribute quickly.

You can find the current list here:
[Good First Issues on GitHub](https://github.com/lambdaclass/ethrex/issues?q=state%3Aopen%20label%3A%22good%20first%20issue%22)

If there are no open "good first issues" at the moment, feel free to browse other issues or reach out in the community chat for guidance on where to start.

We recommend starting with one of these issues to get familiar with the project and our workflow.

## Contact / Support

- Ask questions in the community chat ([Telegram](https://t.me/ethrex_client)) or open an issue.

---

We appreciate your help in making ethrex better!
