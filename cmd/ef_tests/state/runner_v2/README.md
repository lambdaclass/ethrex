# EF State Tests Runner
This module includes a runner for the EF state tests. The main steps it performs include:
- Parsing the tests, that are written in `.json` files.
- Preparing the necessary execution enviroment for each test case.
- Executing the transaction described in the test case.
- Verifying the expected post state against the obtained one.
- Generating a report with the result of executing the tests.

## What are EF State Tests?
The Ethereum Foundation State Tests are individual transactions not related to one another that test particular behavior of the EVM. Tests are usually run for multiple forks and the result of execution may vary between forks.

For more information on these tests check [the docs](https://eest.ethereum.org/main/running_tests/test_formats/state_test/#fixtureconfig).

## How to run the tests?

First, make sure you have the EF tests downloaded. This can be achieved by running:
`make download-evm-ef-tests`

> Note: this command will only work if you are in `ethrex/cmd/ef_tests/state`.

After the `vectors/` directory is set up, run:

`make run-new-runner`

This will parse and execute everything in the `./vectors` directory by default.

> You can also run `cargo test --package ef_tests-state --test new_runner --release`

## Execution options
In case you do not want to parse and execute everything in the `vectors/` directory there are three flags that can be used to specify files to be run:

- `path`: it can be used to specify the path where the tests of interest are. It can either be a file or a directory. By default this flag is set to the `./vectors` directory.

_Example:_

`make run-new-runner FLAGS="--path ./vectors/GeneralStateTests"`

`make run-new-runner FLAGS="--path ./vectors/GeneralStateTests/stChainId/chainId.json"`


- `json-files`: it can be used to specify the `.json` files of interest (no need for full path). If this flag is set to some value, the `path` flag will be ignored.
_Example:_

`make run-new-runner FLAGS="--json-files chainId.json,transStorageReset.json"`

> Note that different `.json` files are separated by a comma.


- `skip-files`: it can be used to skip certaing `.json` files.
_Example:_

`make run-new-runner FLAGS="--skip-files chainId.json,transStorageReset.json"`

