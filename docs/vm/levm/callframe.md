### CallFrame

The CallFrame has attributes `output` and `sub_return_data` to store both the return data of the current context and of the sub-context.

Opcodes like `RETURNDATACOPY` and `RETURNDATASIZE` access the return data of the subcontext (`sub_return_data`). 
Meanwhile, opcodes like `RETURN` or `REVERT` modify the return data of the current context (`output`).

---

### CallFrame backups

#### What is a CallFrame backup?

Each CallFrame contains a `call_frame_backup` structure, which stores the original state of accounts and their storage slots that were modified during the execution of this call frame. This is necessary to correctly revert changes if the call frame execution ends with a REVERT.

- `original_accounts_info: HashMap<Address, Account>` — the original account data (balance, nonce, code, etc.) for accounts that were modified.
- `original_account_storage_slots: HashMap<Address, HashMap<H256, U256>>` — the original values of storage slots for accounts whose storage was modified.

#### When and why is the backup used?

- **Before any account or storage modification**: if an account or storage slot is being modified for the first time in the current call frame, its original value is saved in the backup.
- **For nested calls (CALL/CREATE)**: when a new call frame is created (e.g., via `CALL`), its backup is initially empty. If the nested call completes successfully, its backup is merged into the parent call frame's backup. If the nested call REVERTs, all changes recorded in its backup are reverted (restored).

#### Example: generic_call

- Non-precompile path (`CALL` into regular bytecode):
  - A new call frame is pushed onto the stack.
  - If needed, value transfer occurs — this change is tracked for possible revert.
  - After that, `backup_substate()` is called — a snapshot of the current substate is made for possible rollback.
  - If the nested call ends with a REVERT, the substate is restored from the backup (see `handle_state_backup`) and the cache is restored from the callframe backup (see `restore_cache_state`).
  - If the nested call completes successfully, its callframe backup is merged into the parent (`merge_call_frame_backup_with_parent`).

- Precompile path: no callframe is pushed. The precompile is executed directly and, on success, value is transferred. No callframe backup or substate backup is needed here because there is no nested EVM execution to revert.

In `CREATE`/`CREATE2`, the sender (deployer) nonce is incremented before pushing the child call frame (this change is not reverted), then after pushing the frame the new contract's nonce and the value transfer are performed and `backup_substate()` is taken so those changes can be reverted if the initcode REVERTs.

#### Why is the order of actions important?

- **Value transfer** happens after pushing the call frame but before the backup. This is important: if the nested call REVERTs, the value transfer must also be reverted.
- **Nonce increment of the sender** (e.g., for CREATE) happens before pushing the call frame and before the backup, because this change should not be reverted even if the call frame REVERTs.

#### Summary of revert/merge logic

- If a call frame ends with a REVERT, all account and storage changes recorded in its backup are reverted (restored).
- If a call frame completes successfully, its backup is merged into the parent backup:
  - For each account/slot already present in the parent backup, nothing is done.
  - For new accounts/slots from the child call frame's backup, they are added to the parent backup.
