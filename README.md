# ethrex

Minimalist, stable, modular and fast implementation of the Ethereum protocol in Rust.

[![Telegram Chat][tg-badge]][tg-url]
[![license](https://img.shields.io/github/license/lambdaclass/ethrex)](/LICENSE)

[tg-badge]: https://img.shields.io/endpoint?url=https%3A%2F%2Ftg.sumanjay.workers.dev%2Fethrex_client%2F&logo=telegram&label=chat&color=neon
[tg-url]: https://t.me/ethrex_client

## Quick-start

To install the client, [first install Rust](https://www.rust-lang.org/tools/install) and run:

```sh
curl -sSL https://raw.githubusercontent.com/lambdaclass/ethrex/refs/heads/main/install.sh | sh
```

This installs the `ethrex` binary.

### Starting the L1

After running the install script, you can start the L1 by running:

```sh
ethrex --dev
```

> [!TIP]
> In case you want to start a new L1, you can remove the data of the old one by executing:
>
> ```sh
> ethrex removedb
> ```

## L1 and L2 support

This client supports running in two different modes:

- As a regular Ethereum execution client
- As a multi-prover ZK-Rollup (supporting SP1, RISC Zero and TEEs), where block execution is proven and the proof sent to an L1 network for verification, thus inheriting the L1's security. Support for based sequencing is currently in the works.

We call the first one ethrex L1 and the second one ethrex L2.

## Philosophy

Many long-established clients accumulate bloat over time. This often occurs due to the need to support legacy features for existing users or through attempts to implement overly ambitious software. The result is often complex, difficult-to-maintain, and error-prone systems.

In contrast, our philosophy is rooted in simplicity. We strive to write minimal code, prioritize clarity, and embrace simplicity in design. We believe this approach is the best way to build a client that is both fast and resilient. By adhering to these principles, we will be able to iterate fast and explore next-generation features early, either from the Ethereum roadmap or from innovations from the L2s.

Read more about our engineering philosophy [in this post of our blog](https://blog.lambdaclass.com/lambdas-engineering-philosophy/).

## Design Principles

- Ensure effortless setup and execution across all target environments.
- Be vertically integrated. Have the minimal amount of dependencies.
- Be structured in a way that makes it easy to build on top of it, i.e rollups, vms, etc.
- Have a simple type system. Avoid having generics leaking all over the codebase.
- Have few abstractions. Do not generalize until you absolutely need it. Repeating code two or three times can be fine.
- Prioritize code readability and maintainability over premature optimizations.
- Avoid concurrency split all over the codebase. Concurrency adds complexity. Only use where strictly necessary.

## 🗺️ Roadmap

You can find our current and planned features in our roadmap page.

[View the roadmap →](https://docs.ethrex.xyz/l2/roadmap.html)

## 📖 Documentation

Full documentation is available in the [`docs/`](./docs/) directory. Please refer to it for setup, usage, and development details.
For better viewing, we have it hosted in [docs.ethrex.xyz](https://docs.ethrex.xyz/).
This includes both [L1](https://docs.ethrex.xyz/l1/index.html) and [L2](https://docs.ethrex.xyz/l2/index.html) documentation.


## 📚 References and acknowledgements

The following links, repos, companies and projects have been important in the development of this repo, we have learned a lot from them and want to thank and acknowledge them.

- [Ethereum](https://ethereum.org/en/)
- [Starkware](https://starkware.co/)
- [Polygon](https://polygon.technology/)
- [Optimism](https://www.optimism.io/)
- [Arbitrum](https://arbitrum.io/)
- [ZKsync](https://zksync.io/)
- [Geth](https://github.com/ethereum/go-ethereum)
- [Taiko](https://taiko.xyz/)
- [RISC Zero](https://risczero.com/)
- [SP1](https://github.com/succinctlabs/sp1)
- [Aleo](https://aleo.org/)
- [Neptune](https://neptune.cash/)
- [Mina](https://minaprotocol.com/)
- [Nethermind](https://www.nethermind.io/)
- [Gattaca](https://github.com/gattaca-com)
- [Spire](https://www.spire.dev/)
- [Commonware](https://commonware.xyz/)

If we forgot to include anyone, please file an issue so we can add you. We always strive to reference the inspirations and code we use, but as an organization with multiple people, mistakes can happen, and someone might forget to include a reference.

## Security

We take security seriously. If you discover a vulnerability in this project, please report it responsibly.

- You can report vulnerabilities directly via the **[GitHub "Report a Vulnerability" feature](../../security/advisories/new)**.
- Alternatively, send an email to **[security@lambdaclass.com](mailto:security@lambdaclass.com)**.

For more details, please refer to our [Security Policy](./.github/SECURITY.md).
