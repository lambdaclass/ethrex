### Debug Mode

Debug mode currently enables printing in solidity using a private `print()` function that does an `MSTORE` with a specific offset. If the VM is in debug mode it will recognize the offset as the "key" for printing the value that the user sent to the function.
You can find a test of this in the [repository test data](../../../../test_data/levm_print/PrintTest.sol)
