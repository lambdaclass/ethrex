### Debug Mode

Debug mode currently enables printing in solidity by using a `print()` function that does an `MSTORE` with a specific offset to toggle the "print mode". If the VM is in debug mode it will recognize the offset as the "key" for enabling/disabling print mode. If print mode is enabled, MSTORE opcode stores into a buffer the data that the user wants to print, and when there is no more data left to read it prints it and disables the print mode so that execution continues normally.
You can find the solidity code in the [test data](../../../../test_data/levm_print/PrintTest.sol) of this repository. It can be tested with the `PrintTest` contract and it can be imported into another contracts.
