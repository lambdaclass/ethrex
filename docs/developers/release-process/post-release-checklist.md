# Etherx Post-Release Checklist

This checklist helps ensure the successful completion of the ethrex release process. After publishing a new release, follow the steps below to verify that everything is in order.

Refer to the ethrex documentation for detailed setup instructions. All tests should be run on a clean environment to avoid interference from previous builds.

## 1. Check Homebrew

```shell
brew install lambdaclass/tap/ethrex
ethrex --version
```

This should return the new release version. Check that both `--dev` and `l2 --dev` are working.

```shell
ethrex --dev
```

```shell
ethrex l2 --dev
```

## 2. Check apt packages

Last commit in [ethrex-apt](https://github.com/lambdaclass/ethrex-apt/tree/gh-pages) repo (branch `gh-pages`) should have the new release version.

Follow the instructions in the `main` branch of that repo, among with the previous step, to confirm the installation works.

> [!TIP]
> You can use Docker to test the installation in a clean environment:
> ```shell
> docker run --rm -it ubuntu
> ```
