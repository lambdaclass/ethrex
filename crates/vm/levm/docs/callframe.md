### CallFrame

The CallFrame has attributes `returndata` and `sub_return_data` to store both the return data of the current context and of the sub-context.

Opcodes like `RETURNDATACOPY` and `RETURNDATASIZE` access the return data of the subcontext (`sub_return_data`). 
Meanwhile, opcodes like `RETURN` or `REVERT` modify the return data of the current context (`returndata`).
