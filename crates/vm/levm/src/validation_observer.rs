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
//! reservation state).
//! [`canonical_paymaster_pay_frame`](ValidationObserver::canonical_paymaster_pay_frame)
//! carries that frame's index when the pay frame's resolved target has the
//! canonical runtime code hash, and the
//! `current_frame_index == canonical_paymaster_pay_frame` skip applies the
//! exemption to it.
//!
//! ## FOCIL Profile 2 replay
//! The same observer serves the inclusion-list omission check for frame
//! transactions (the FOCIL frame-transaction EIP, "Profile 2 eligibility"). That
//! replay asks a different question from mempool admission and differs from it
//! in exactly the places the EIP names: the storage rule is the
//! [`Profile2Surface`] (slots `0..AA_VOPS_SLOT_COUNT` of `sender` and `payer`,
//! nothing exempted) instead of EIP-8141's sender-only rule, and a per-list
//! [`CodeBudget`] bounds the code bodies a replay may load. Both attach through
//! [`ValidationObserver::profile2`] and [`ValidationObserver::code_budget`];
//! the canonical pay-frame exemption is never set alongside them, and the
//! surface check runs before it so it could not widen the surface if it were.

use ethrex_common::{Address, H256, U256};
use rustc_hash::FxHashSet;

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
    /// Profile 2 replay: an `SLOAD` read storage outside the validation surface,
    /// an account other than `sender` or `payer`, or a slot at or above
    /// `AA_VOPS_SLOT_COUNT`; carries the storage owner and the slot.
    StorageReadOutsideSurface { address: Address, slot: H256 },
    /// Profile 2 replay: the deploy frame wrote a `sender` slot at or above
    /// `AA_VOPS_SLOT_COUNT`. Writes are restricted to what the deploy frame may
    /// touch and must additionally stay inside the surface.
    StorageWriteOutsideSurface { address: Address, slot: H256 },
    /// Profile 2 replay: loading one more code body would exceed the per-list
    /// code budget (`MAX_VALIDATION_CODE_BODIES` bodies or
    /// `MAX_VALIDATION_CODE_BYTES` bytes). The replay does not proceed.
    CodeBudgetExceeded,
}

/// The FOCIL Profile 2 validation surface: storage reads are confined to slots
/// `0..slot_count` of `sender` and `payer`. Attached to the observer it replaces
/// EIP-8141's sender-only mempool storage rule for the duration of a replay.
/// `payer` is resolved from the prefix shape before the first frame executes,
/// so the surface is known before any read is judged against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile2Surface {
    pub payer: Address,
    /// `AA_VOPS_SLOT_COUNT`, the number of leading slots inside the surface.
    pub slot_count: U256,
}

/// Per-inclusion-list code budget (FOCIL frame-transaction EIP, "Code bound").
///
/// Every distinct `codeHash` loaded by any replay of the list is charged once,
/// one body and its byte length, and is free to every later replay of the same
/// list, at either evaluation state. An account with empty code is not a body.
/// Charges survive the verdict: a replay that loaded bodies and then failed
/// still made every attester read them, so the caller carries the budget from
/// one replay to the next unchanged by the outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBudget {
    pub max_bodies: u64,
    pub max_bytes: u64,
    pub bodies_loaded: u64,
    pub bytes_loaded: u64,
    /// The code hashes already paid for by this list.
    pub loaded: FxHashSet<H256>,
    /// Set once a charge was refused. No further charges are taken, so the
    /// replay that overran the budget does not keep loading bodies.
    pub exceeded: bool,
}

impl CodeBudget {
    pub fn new(max_bodies: u64, max_bytes: u64) -> Self {
        Self {
            max_bodies,
            max_bytes,
            bodies_loaded: 0,
            bytes_loaded: 0,
            loaded: FxHashSet::default(),
            exceeded: false,
        }
    }

    /// Charges a non-empty code body of `len` bytes under `code_hash`, unless
    /// this list already paid for it. Returns `false`, and marks the budget
    /// exceeded, when the charge does not fit.
    pub fn charge(&mut self, code_hash: H256, len: u64) -> bool {
        if self.exceeded {
            return false;
        }
        if self.loaded.contains(&code_hash) {
            return true;
        }
        let bodies = self.bodies_loaded.saturating_add(1);
        let bytes = self.bytes_loaded.saturating_add(len);
        if bodies > self.max_bodies || bytes > self.max_bytes {
            self.exceeded = true;
            return false;
        }
        self.loaded.insert(code_hash);
        self.bodies_loaded = bodies;
        self.bytes_loaded = bytes;
        true
    }
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
    /// Index of the canonical paymaster's pay frame, if any: set when the pay
    /// frame's resolved target carries the canonical runtime code hash, which is
    /// what admits the access-restriction skip (see module docs).
    pub canonical_paymaster_pay_frame: Option<usize>,
    /// Index of the EIP-8272 recent-root verifier frame, if the transaction leads
    /// with one and the code at `RECENT_ROOT_ADDRESS` is `RECENT_ROOT_CODE` (the
    /// harness checks the code before setting this). While that frame runs
    /// `RECENT_ROOT_CODE` at the top level, `SLOTNUM` and `SLOAD`s of the
    /// predeploy's own storage are permitted; nothing else changes, and no nested
    /// call inherits either permission.
    pub recent_root_verifier_frame: Option<usize>,
    /// Address of the RECENT_ROOT_ADDRESS predeploy (0x…8272).
    pub recent_root_address: Address,
    /// The opcode byte executed on the previous dispatch-loop iteration. Used to
    /// enforce the `GAS` sequential rule (`GAS` is allowed only immediately
    /// before a `*CALL`). Reset each iteration.
    pub last_opcode: u8,
    /// Sender storage slots touched (read or written) during the prefix. Recorded
    /// for the admission-time revalidation affected-set.
    pub touched_sender_slots: Vec<H256>,
    /// Whether the prefix read `TXPARAM(0x12)`, the sender's legacy account nonce
    /// (EIP-8250 §Mempool). Such a prefix depends on the legacy nonce even when
    /// the transaction's own nonce lives in a keyed domain.
    pub read_legacy_nonce: bool,
    /// FOCIL Profile 2 validation surface. `Some` only during an inclusion-list
    /// omission replay, where it replaces the sender-only storage rule.
    pub profile2: Option<Profile2Surface>,
    /// FOCIL Profile 2 per-list code budget. `Some` only during an omission
    /// replay; the harness moves it in before the replay and back out after.
    pub code_budget: Option<CodeBudget>,
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
            recent_root_verifier_frame: None,
            recent_root_address: Address::zero(),
            last_opcode: 0,
            touched_sender_slots: Vec::new(),
            read_legacy_nonce: false,
            profile2: None,
            code_budget: None,
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
            recent_root_verifier_frame: None,
            recent_root_address: Address::zero(),
            last_opcode: 0,
            touched_sender_slots: Vec::new(),
            read_legacy_nonce: false,
            profile2: None,
            code_budget: None,
            violation: None,
        }
    }

    /// Profile 2 only: whether `slot` of `address` lies inside the validation
    /// surface, that is, the owner is `sender` or `payer` and the slot is below
    /// `AA_VOPS_SLOT_COUNT`. `None` when no surface is attached, in which case
    /// the mempool's sender-only rule applies instead.
    pub fn slot_in_surface(&self, address: Address, slot: H256) -> Option<bool> {
        let surface = self.profile2?;
        let owner_in_surface = address == self.sender || address == surface.payer;
        Some(owner_in_surface && U256::from_big_endian(slot.as_bytes()) < surface.slot_count)
    }

    /// Profile 2 only: charges one code body against the per-list budget and
    /// records [`FrameSimViolation::CodeBudgetExceeded`] when it does not fit.
    /// No-op without a budget, so the mempool path pays nothing here.
    pub fn charge_code(&mut self, code_hash: H256, len: u64) {
        if let Some(budget) = self.code_budget.as_mut()
            && !budget.charge(code_hash, len)
        {
            self.record_violation(FrameSimViolation::CodeBudgetExceeded);
        }
    }

    /// Whether the recent-root verifier frame is the one executing, at the top
    /// level: the executing contract must be the predeploy itself, so a callee of
    /// the frame (impossible for `RECENT_ROOT_CODE`, but the rule is stated for the
    /// frame, not for the code) inherits nothing.
    pub fn in_recent_root_frame(&self, code_address: Address) -> bool {
        self.recent_root_verifier_frame == Some(self.current_frame_index)
            && code_address == self.recent_root_address
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
