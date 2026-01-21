//! Stencil definitions for copy-and-patch JIT compilation.
//!
//! Stencils are the machine code templates for each opcode. They are
//! compiled from Rust source at build time and extracted as raw bytes
//! with relocation information.
//!
//! ## Structure
//!
//! - `context`: Minimal JitContext for stencil compilation
//! - `markers`: External symbols that create relocations for patching
//! - `arithmetic`, `stack`, `control`: Stencil source code (Rust)
//! - `generated`: Auto-generated stencil bytes and relocations (created by build.rs)

// NOTE: context.rs, markers.rs, arithmetic.rs, stack.rs, control.rs exist in this
// directory but are NOT compiled as part of the main crate. They are only used by
// build.rs to generate stencil bytes. Do not add them to the module tree here.

// Generated stencil bytes - created by build.rs
#[cfg(feature = "jit")]
mod generated;

/// A compiled stencil ready for copying and patching
#[derive(Debug, Clone)]
pub struct Stencil {
    /// The raw machine code bytes
    pub bytes: &'static [u8],
    /// Relocations that need to be patched
    pub relocations: &'static [Relocation],
}

/// A relocation within a stencil that needs patching
#[derive(Debug, Clone, Copy)]
pub struct Relocation {
    /// Offset within the stencil bytes where the relocation is
    pub offset: usize,
    /// What kind of relocation this is
    pub kind: RelocKind,
    /// Size of the relocation in bytes (4 for 32-bit relative, 8 for 64-bit absolute)
    pub size: u8,
}

/// Types of relocations in stencils
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocKind {
    /// Address of the next stencil in sequence
    NextStencil,
    /// Exit JIT execution
    ExitJit,
    /// Immediate value (for PUSH instructions)
    ImmediateValue,
    /// Jump table entry (for JUMP/JUMPI)
    JumpTableEntry,
}

impl Stencil {
    /// Get the size of this stencil in bytes
    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}

// Re-export generated stencils when jit feature is enabled
#[cfg(feature = "jit")]
pub use generated::*;

// Placeholder stencils when jit feature is disabled (for compilation)
#[cfg(not(feature = "jit"))]
pub static STENCIL_STOP: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_ADD: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_SUB: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_MUL: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_POP: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_PUSH: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_JUMPDEST: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_PC: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_GAS: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_LT: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_GT: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_EQ: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};

#[cfg(not(feature = "jit"))]
pub static STENCIL_ISZERO: Stencil = Stencil {
    bytes: &[],
    relocations: &[],
};
