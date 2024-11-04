## Substate

`accessed_addresses` and `accessed_storage_keys` belong to the Substate but in our VM implementation they are not there because we already know what the warm addresses and storage keys are by looking at the `Cache` structure.
