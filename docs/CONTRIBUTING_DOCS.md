# Contributing to the Documentation

We welcome contributions to the documentation! If you want to help improve or expand the docs, please follow these guidelines:

## How to Edit the Docs

- All documentation lives in this `docs/` directory and its subfolders.
- The documentation is written in Markdown and rendered using [mdBook](https://rust-lang.github.io/mdBook/).
- To preview your changes locally, [install the dependencies](#documentation-dependencies) and run:

  ```sh
  make docs-serve
  ```

  This will start a local server and open the docs in your browser.

## Adding or Editing Content

- To add a new page, create a new `.md` file in the appropriate subdirectory and add a link to it in `SUMMARY.md`.
- To edit an existing page, simply modify the relevant `.md` file.
- For style and formatting, try to keep a consistent tone and structure with the rest of the documentation.

## Documentation dependencies

We use some mdBook preprocessors and backends for extra features:

- [`mdbook-alerts`](https://github.com/lambdalisue/rs-mdbook-alerts) for custom markdown syntax.
- [`mdbook-mermaid`](https://github.com/badboy/mdbook-mermaid) for diagrams.
- [`lychee`](https://github.com/lycheeverse/lychee) for checking broken links (optional, runs automatically in CI).

You can install mdBook and all dependencies with:

```sh
make docs-deps
```

## Submitting Changes

- Please open a Pull Request with your proposed changes.
- If you are adding new content, update `SUMMARY.md` so it appears in the navigation.
- If you have questions, open an issue or ask in the community chat.

Thank you for helping improve the documentation!
