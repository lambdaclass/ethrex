# State Tests

The state tests are individual transactions not related one to each other that test particular behavior of the EVM. Tests are usually run for multiple forks and the result of execution may vary between forks.
Some [docs](https://ethereum.github.io/execution-spec-tests/main/consuming_tests/state_test/).


## Running the tests

```bash
make run-evm-ef-tests flags=<flags>
```
or
```bash
cargo test --package ef_tests-state --test all -- <flags>
```

**Flags**
- forks: Forks for which we want to run the tests for.
- tests: Tests (.json files) we want to run
- specific_tests: For running tests with a specific name. (Sometimes a .json file has multiple tests)
- summary: For not doing a re-run with REVM of failed tests after LEVM's run.
- skip: For skipping tests
- verbose: For more info while running, like tests names being run.
- revm: For running EFTests ONLY with REVM.


Example usage: 
```bash
cargo test --package ef_tests-state --test all -- --forks Prague,Cancun --summary --tests push0.json,invalidAddr.json
```
This runs 2 specific tests with LEVM just for Prague and Cancun. If they fail they are not re-run with REVM.

Most of the tests that we run are from [this repository](https://github.com/ethereum/tests). We run the `GeneralStateTests` from that repo and also from `LegacyTests`, which is another repository that has snapshots of tests from previous forks. 


Beware: Sometimes there is a test overlap between the tests folders we have downloaded and we may run the same test for a recent fork (Cancun ATTOW) twice. The impact of this in performance is minimal because we are doing runs for other forks anyway so one more run won't harm, but we should be aware that may lead to an inaccurate test count. We chose not to handle this because it wasn't a huge problem, but be conscious about this.
