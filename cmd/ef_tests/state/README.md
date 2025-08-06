# State Tests

The state tests are individual transactions not related one to each other that test particular behavior of the EVM. Tests are usually run for multiple forks and the result of execution may vary between forks.
Some [docs](https://ethereum.github.io/execution-spec-tests/main/consuming_tests/state_test/).

## Running the tests

```bash
make run-state-tests flags=<flags>
```
or
```bash
cargo test --package ef_tests-state --test all --release -- <flags>
```

All tests are first run on levm for the most recent forks (Merge,Shangai,Cancun and Prague), and then any failing tests are re-run on revm. If you want to run the tests with a different set up, you can see how on the following sections.

## Setting up the tests if you are running flamegraphs

```bash
make download-state-tests
```

## Refreshing tests when outdated

Sometimes we update the version of tests we run, for removing the old files and downloading the new ones you can use this command:
```bash
make refresh-state-tests
```

### Flags
- `forks`: Forks for which we want to run the tests for.
- `tests`: Tests (.json files) we want to run
- `specific-tests`: For running tests with a specific name. (Sometimes a .json file has multiple tests)
- `summary`: For not doing a re-run with REVM of failed tests after LEVM's run.
- `skip`: For skipping tests
- `verbose`: For more info while running, like tests names being run.
- `revm`: For running EFTests ONLY with REVM.
- `path`: For running particular tests that have their specified paths listed with the tests flag.


**Example usage**: 
```bash
cargo test --package ef_tests-state --test all --release -- --forks Prague,Cancun --summary --tests push0.json,invalidAddr.json
```
This runs 2 specific tests with LEVM just for Prague and Cancun. If they fail they are not re-run with REVM.

```bash
cargo test --package ef_tests-state --test all --release -- --forks Prague,Cancun --summary --paths --tests LegacyTests/Cancun/GeneralStateTests/Shanghai/stEIP3855-push0/push0.json,GeneralStateTests/Shanghai/stEIP3855-push0/push0.json,GeneralStateTests/stBadOpcode/invalidAddr.json,LegacyTests/Cancun/GeneralStateTests/stBadOpcode/invalidAddr.json
```
This runs the same 2 tests with LEVM as before, but by specifying the files you want to run. If they fail they are not re-run with REVM.


Most of the tests that we run are from [this repository](https://github.com/ethereum/tests). We run the `GeneralStateTests` from that repo and also from `LegacyTests`, which is another repository that has snapshots of tests from previous forks. 


Beware: Sometimes there is a test overlap between the tests folders we have downloaded and we may run the same test for a recent fork (Cancun ATTOW) twice. The impact of this in performance is minimal because we are doing runs for other forks anyway so one more run won't harm, but we should be aware that may lead to an inaccurate test count. We chose not to handle this because it wasn't a huge problem, but be conscious about this.

## Running all the tests with either levm or revm

```bash
make test-levm
```
or
```bash
make test-revm
```
