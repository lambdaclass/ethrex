//! EIP-8141 mempool validation-trace observer.
//!
//! A lightweight, opt-in observer attached to [`VM`](crate::vm::VM) that enforces
//! the ERC-7562-style validation-trace rules during *mempool* simulation of a
//! frame transaction's validation prefix (the verify/pay/deploy frames that must
//! run before the transaction's payer is established). It is a **local peer
//! policy**, never a consensus rule: it is active ONLY behind
//! [`LEVM::simulate_frame_validation_prefix`](crate::vm::VM) (the mempool entry
//! point) and is constructed with [`ValidationObserver::disabled`] everywhere
//! else, so normal block execution and block building pay a single `if active`
//! branch in the dispatch loop and nothing more (mirrors `LevmOpcodeTracer`).
//!
//! ## OQ4 — SETDELEGATE is vacuous
//! The draft EIP's validation-trace rules reference a `SETDELEGATE` operation
//! used by deploy frames to install an EIP-7702 delegation for the sender. There
//! is no `SETDELEGATE` opcode in `opcodes.rs` (the ethrex LEVM opcode set), so
//! the deploy-frame delegation allowance reduces to what the actual opcode set
//! can express: `CREATE`/`CREATE2` plus `SSTORE` writing to the sender's own
//! storage. The observer therefore permits exactly those state effects inside
//! the deploy frame and bans all other state writes; there is no separate
//! `SETDELEGATE` allowance to model.
//!
//! ## Canonical-pay-frame exemption
//! ERC-7562 exempts the canonical paymaster's pay frame from the storage/call
//! access restrictions (a canonical paymaster is trusted to touch shared
//! reservation state). ethrex cannot resolve the canonical paymaster bytecode
//! (OQ1: not pinned in the draft EIP or any reference implementation), so
//! [`canonical_paymaster_pay_frame`](ValidationObserver::canonical_paymaster_pay_frame)
//! is always `None` here and the exemption never fires. The field and the
//! `current_frame_index == canonical_paymaster_pay_frame` skip are wired up so
//! the exemption flips on for free once the canonical code hash is pinned.

use ethrex_common::{Address, H256};

/// A validation-trace rule violation detected during prefix simulation.
///
/// The first violation observed is recorded; the simulation harness treats any
/// recorded violation as a failed validation (the transaction is rejected from
/// the mempool).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameSimViolation {
    /// A banned opcode was executed (carries the raw opcode byte). The banned
    /// set is enforced in the dispatch loop; see
    /// [`VM::run_execution`](crate::vm::VM::run_execution).
    BannedOpcode(u8),
    /// A state write (`SSTORE`/`CREATE`/`CREATE2`) occurred outside the deploy
    /// frame, or an `SSTORE` inside the deploy frame targeted storage other than
    /// the sender's own account.
    StateWriteOutsideDeploy,
    /// An `SLOAD` read storage of an account other than the sender.
    StorageReadNonSender,
    /// A `CALL`/`CALLCODE`/`DELEGATECALL`/`STATICCALL`/`EXTCODE*` referenced a
    /// target that does not exist (no code, no balance, no nonce, not a
    /// precompile) or that is EIP-7702-delegated; carries the offending target.
    CallToNonexistentOrDelegated(Address),
    /// A deploy frame finished without leaving non-empty code installed at the
    /// sender's address.
    DeployInstalledNoCode,
}

/// EIP-8141 validation-trace observer. Inert (`active == false`) in every VM
/// construction except mempool prefix simulation.
///
/// The dispatch-loop and handler hooks read this only behind
/// `if self.validation_observer.active`, so an inactive observer has zero
/// overhead on the perf-sensitive execution path (one branch), exactly like
/// [`LevmOpcodeTracer`](crate::opcode_tracer::LevmOpcodeTracer).
#[derive(Debug, Clone)]
pub struct ValidationObserver {
    /// Whether this observer is active. `false` disables every hook.
    pub active: bool,
    /// The frame transaction's sender. SLOAD/SSTORE are restricted to this
    /// address; CALL/EXTCODE targets equal to this address are exempt.
    pub sender: Address,
    /// Index (into `FrameTransaction.frames`) of the deploy frame, if the
    /// prefix has one. `SSTORE`/`CREATE`/`CREATE2` are allowed only while this
    /// frame is executing.
    pub deploy_frame_index: Option<usize>,
    /// Index of the currently executing prefix frame. Set by the harness before
    /// each frame runs.
    pub current_frame_index: usize,
    /// Execution mode of the currently executing prefix frame (raw `mode` byte:
    /// 0 = DEFAULT, 1 = VERIFY, 2 = SENDER). Set by the harness before each
    /// frame runs.
    pub current_frame_mode: u8,
    /// Address of the canonical EXPIRY_VERIFIER predeploy (0x…8141). `TIMESTAMP`
    /// is permitted only when `current_call_frame.code_address` equals this value,
    /// which allows TIMESTAMP inside a nested call *into* the predeploy while
    /// correctly banning it in any callee of an expiry frame that routes execution
    /// elsewhere (an under-reject the per-top-frame boolean could not prevent).
    pub expiry_verifier: Address,
    /// Index of the canonical paymaster's pay frame, if any. Always `None`
    /// (OQ1, see module docs); the access-restriction skip is wired for the
    /// future canonical-paymaster case.
    pub canonical_paymaster_pay_frame: Option<usize>,
    /// The opcode byte executed on the previous dispatch-loop iteration. Used to
    /// enforce the `GAS` sequential rule (`GAS` is allowed only immediately
    /// before a `*CALL`). Reset each iteration.
    pub last_opcode: u8,
    /// Sender storage slots touched (read or written) during the prefix. Recorded
    /// for the admission-time revalidation affected-set (Phase 3).
    pub touched_sender_slots: Vec<H256>,
    /// First violation observed, if any.
    pub violation: Option<FrameSimViolation>,
}

impl ValidationObserver {
    /// Returns an inactive observer. No allocations; zero overhead on the hot
    /// path (every hook is gated by `if active`).
    pub fn disabled() -> Self {
        Self {
            active: false,
            sender: Address::zero(),
            deploy_frame_index: None,
            current_frame_index: 0,
            current_frame_mode: 0,
            expiry_verifier: Address::zero(),
            canonical_paymaster_pay_frame: None,
            last_opcode: 0,
            touched_sender_slots: Vec::new(),
            violation: None,
        }
    }

    /// Returns an active observer for simulating the validation prefix of a
    /// frame transaction sent by `sender`, whose deploy frame (if any) is at
    /// `deploy_frame_index`.
    pub fn new(
        sender: Address,
        deploy_frame_index: Option<usize>,
        expiry_verifier: Address,
    ) -> Self {
        Self {
            active: true,
            sender,
            deploy_frame_index,
            current_frame_index: 0,
            current_frame_mode: 0,
            expiry_verifier,
            canonical_paymaster_pay_frame: None,
            last_opcode: 0,
            touched_sender_slots: Vec::new(),
            violation: None,
        }
    }

    /// Records the first violation observed; later violations are ignored (the
    /// transaction is already rejected).
    pub fn record_violation(&mut self, violation: FrameSimViolation) {
        if self.violation.is_none() {
            self.violation = Some(violation);
        }
    }

    /// Whether the currently executing frame is the canonical paymaster pay
    /// frame (always `false` while `canonical_paymaster_pay_frame` is `None`).
    pub fn in_canonical_pay_frame(&self) -> bool {
        self.canonical_paymaster_pay_frame == Some(self.current_frame_index)
    }

    /// Whether the currently executing frame is the deploy frame.
    pub fn in_deploy_frame(&self) -> bool {
        self.deploy_frame_index == Some(self.current_frame_index)
    }
}
