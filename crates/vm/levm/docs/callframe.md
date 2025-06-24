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

In the `generic_call` function (see `opcode_handlers/system.rs`):
- A new call frame is pushed onto the stack.
- If needed, value transfer (transfer) occurs — this change is also tracked in the backup.
- After that, `backup_substate()` is called — a snapshot of the current substate is made for possible rollback.
- If the nested call ends with a REVERT, the state is restored from the backup (see `handle_state_backup` and `restore_cache_state`).
- If the nested call completes successfully, its backup is merged into the parent backup (`merge_call_frame_backup_with_parent`).

#### Why is the order of actions important?

- **Value transfer** happens after pushing the call frame but before the backup. This is important: if the nested call REVERTs, the value transfer must also be reverted.
- **Nonce increment** (e.g., for CREATE) happens before pushing the call frame and before the backup, because this change should not be reverted even if the call frame REVERTs.

#### Summary of revert/merge logic

- If a call frame ends with a REVERT, all account and storage changes recorded in its backup are reverted (restored).
- If a call frame completes successfully, its backup is merged into the parent backup:
  - For each account/slot already present in the parent backup, nothing is done.
  - For new accounts/slots from the child call frame's backup, they are added to the parent backup.

#### Where to find the implementation

- Structures: `CallFrame`, `CallFrameBackup` (see `call_frame.rs`)
- Backup/restore/merge logic: VM methods — `backup_substate`, `restore_cache_state`, `merge_call_frame_backup_with_parent` (see `utils.rs`, `call_frame.rs`, `opcode_handlers/system.rs`)
- Usage example: the `generic_call` function (see `opcode_handlers/system.rs`)
