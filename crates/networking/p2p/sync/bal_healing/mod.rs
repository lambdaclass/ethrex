//! BAL-replay state healing for snap/2.
//!
//! Fork gate: activate only when `chain_config.is_amsterdam_activated(header.timestamp)`
//! — the same fork that gates EIP-7928 BAL production (Task 0.4 contract).
//!
//! Phase 6 adds `apply_bal` and `advance_state_via_bals` here.
