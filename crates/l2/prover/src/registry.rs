use std::collections::HashMap;
use std::sync::Arc;

use ethrex_guest_program::traits::GuestProgram;

/// Registry mapping `program_id` → [`GuestProgram`] implementations.
///
/// The registry is created once at prover startup and is immutable during
/// the prover's lifetime.  Each registered [`GuestProgram`] provides ELF
/// binaries and serialization logic for a specific guest program type
/// (e.g. `"evm-l2"`, `"transfer"`).
pub struct GuestProgramRegistry {
    programs: HashMap<String, Arc<dyn GuestProgram>>,
    default_program_id: String,
}

impl GuestProgramRegistry {
    /// Create a new empty registry with the given default program id.
    ///
    /// The default is used when a batch does not specify a `program_id`
    /// (backward compatibility with pre-modularization protocol).
    pub fn new(default_program_id: &str) -> Self {
        Self {
            programs: HashMap::new(),
            default_program_id: default_program_id.to_string(),
        }
    }

    /// Register a guest program.  The program's [`GuestProgram::program_id`]
    /// is used as the key; registering a program with a duplicate id replaces
    /// the previous entry.
    pub fn register(&mut self, program: Arc<dyn GuestProgram>) {
        self.programs
            .insert(program.program_id().to_string(), program);
    }

    /// Look up a guest program by id.
    pub fn get(&self, program_id: &str) -> Option<&Arc<dyn GuestProgram>> {
        self.programs.get(program_id)
    }

    /// Return the default guest program, if registered.
    pub fn default_program(&self) -> Option<&Arc<dyn GuestProgram>> {
        self.programs.get(&self.default_program_id)
    }

    /// Return the default program id.
    pub fn default_program_id(&self) -> &str {
        &self.default_program_id
    }

    /// Return all registered program ids.
    pub fn program_ids(&self) -> Vec<&str> {
        self.programs.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::unwrap_used
)]
mod tests {
    use super::*;
    use ethrex_guest_program::traits::GuestProgramError;

    /// Minimal stub for testing the registry.
    struct StubProgram {
        id: &'static str,
    }

    impl GuestProgram for StubProgram {
        fn program_id(&self) -> &str {
            self.id
        }
        fn elf(&self, _backend: &str) -> Option<&[u8]> {
            None
        }
        fn vk_bytes(&self, _backend: &str) -> Option<Vec<u8>> {
            None
        }
        fn program_type_id(&self) -> u8 {
            99
        }
        fn serialize_input(&self, raw: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
            Ok(raw.to_vec())
        }
        fn encode_output(&self, raw: &[u8]) -> Result<Vec<u8>, GuestProgramError> {
            Ok(raw.to_vec())
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = GuestProgramRegistry::new("stub-a");
        reg.register(Arc::new(StubProgram { id: "stub-a" }));
        reg.register(Arc::new(StubProgram { id: "stub-b" }));

        assert!(reg.get("stub-a").is_some());
        assert!(reg.get("stub-b").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn default_program() {
        let mut reg = GuestProgramRegistry::new("stub-a");
        reg.register(Arc::new(StubProgram { id: "stub-a" }));

        let default = reg.default_program().expect("default should exist");
        assert_eq!(default.program_id(), "stub-a");
    }

    #[test]
    fn default_program_missing() {
        let reg = GuestProgramRegistry::new("nonexistent");
        assert!(reg.default_program().is_none());
    }

    #[test]
    fn program_ids() {
        let mut reg = GuestProgramRegistry::new("a");
        reg.register(Arc::new(StubProgram { id: "a" }));
        reg.register(Arc::new(StubProgram { id: "b" }));

        let mut ids = reg.program_ids();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn duplicate_registration_replaces() {
        let mut reg = GuestProgramRegistry::new("x");
        reg.register(Arc::new(StubProgram { id: "x" }));
        reg.register(Arc::new(StubProgram { id: "x" }));
        assert_eq!(reg.program_ids().len(), 1);
    }

    // ── Integration tests with real guest program implementations ────

    use ethrex_guest_program::programs::{
        EvmL2GuestProgram, TokammonGuestProgram, ZkDexGuestProgram,
    };

    /// Mirrors `create_default_registry()` from prover.rs.
    fn test_registry() -> GuestProgramRegistry {
        let mut reg = GuestProgramRegistry::new("evm-l2");
        reg.register(Arc::new(EvmL2GuestProgram));
        reg.register(Arc::new(ZkDexGuestProgram));
        reg.register(Arc::new(TokammonGuestProgram));
        reg
    }

    #[test]
    fn default_registry_has_three_programs() {
        let reg = test_registry();
        let mut ids = reg.program_ids();
        ids.sort();
        assert_eq!(ids, vec!["evm-l2", "tokamon", "zk-dex"]);
    }

    #[test]
    fn default_program_is_evm_l2() {
        let reg = test_registry();
        let default = reg.default_program().expect("default should exist");
        assert_eq!(default.program_id(), "evm-l2");
        assert_eq!(default.program_type_id(), 1);
    }

    #[test]
    fn zk_dex_program_type_id() {
        let reg = test_registry();
        let prog = reg.get("zk-dex").expect("zk-dex should be registered");
        assert_eq!(prog.program_type_id(), 2);
    }

    #[test]
    fn tokamon_program_type_id() {
        let reg = test_registry();
        let prog = reg.get("tokamon").expect("tokamon should be registered");
        assert_eq!(prog.program_type_id(), 3);
    }

    #[test]
    fn all_programs_have_unique_type_ids() {
        let reg = test_registry();
        let mut type_ids: Vec<u8> = reg
            .program_ids()
            .iter()
            .map(|id| reg.get(id).unwrap().program_type_id())
            .collect();
        type_ids.sort();
        type_ids.dedup();
        assert_eq!(type_ids.len(), 3, "all type IDs must be unique");
    }

    #[test]
    fn zk_dex_circuit_through_registry() {
        use ethrex_guest_program::common::app_execution::{AppCircuit, AppOperation};
        use ethrex_guest_program::programs::zk_dex::circuit::{
            DexCircuit, OP_TOKEN_TRANSFER, TOKEN_TRANSFER_GAS,
        };

        let reg = test_registry();
        let prog = reg.get("zk-dex").expect("zk-dex registered");

        // Verify the program provides correct metadata.
        assert_eq!(prog.program_id(), "zk-dex");
        assert_eq!(prog.program_type_id(), 2);

        // Verify DexCircuit implements AppCircuit correctly.
        let circuit = DexCircuit {
            contract_address: ethrex_common::H160([0xDE; 20]),
        };

        // Verify gas cost for token transfer operation.
        let op = AppOperation {
            op_type: OP_TOKEN_TRANSFER,
            params: vec![0; 96],
        };
        assert_eq!(circuit.gas_cost(&op), TOKEN_TRANSFER_GAS);
        assert_eq!(circuit.gas_cost(&op), 65_000);

        // serialize_input now converts ProgramInput → AppProgramInput,
        // so arbitrary bytes will fail deserialization.
        let raw = b"some bytes";
        assert!(
            prog.serialize_input(raw).is_err(),
            "serialize_input should reject invalid rkyv bytes"
        );
    }

    #[test]
    fn tokamon_execution_through_registry() {
        use ethrex_guest_program::programs::tokamon::execution::execution_program;
        use ethrex_guest_program::programs::tokamon::types::{
            ActionType, GameAction, TokammonProgramInput,
        };

        let reg = test_registry();
        let prog = reg.get("tokamon").expect("tokamon registered");

        assert_eq!(prog.program_id(), "tokamon");

        let input = TokammonProgramInput {
            initial_state_root: [0xBB; 32],
            actions: vec![
                GameAction {
                    player: [0x11; 20],
                    action_type: ActionType::ClaimReward,
                    target_id: 0,
                    payload: vec![],
                },
                GameAction {
                    player: [0x22; 20],
                    action_type: ActionType::CreateSpot,
                    target_id: 1,
                    payload: vec![0u8; 16],
                },
            ],
        };
        let output = execution_program(input).expect("should succeed");
        assert_eq!(output.action_count, 2);
        assert_eq!(output.rewards_claimed, 1);
        assert_eq!(output.spots_created, 1);
        assert_ne!(output.final_state_root, output.initial_state_root);

        let encoded = output.encode();
        assert_eq!(encoded.len(), 88); // 32 + 32 + 8 + 8 + 8
    }

    #[test]
    fn rkyv_roundtrip_through_registry_tokamon() {
        use ethrex_guest_program::programs::tokamon::types::{
            ActionType, GameAction, TokammonProgramInput,
        };

        let input = TokammonProgramInput {
            initial_state_root: [0xEE; 32],
            actions: vec![GameAction {
                player: [0x55; 20],
                action_type: ActionType::FeedTokamon,
                target_id: 99,
                payload: vec![],
            }],
        };

        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&input).expect("serialize");
        let restored: TokammonProgramInput =
            rkyv::from_bytes::<TokammonProgramInput, rkyv::rancor::Error>(&bytes)
                .expect("deserialize");
        assert_eq!(restored.initial_state_root, input.initial_state_root);
        assert_eq!(restored.actions.len(), 1);
        assert_eq!(restored.actions[0].target_id, 99);
    }
}
