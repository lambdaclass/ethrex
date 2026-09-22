//! ere-platform entrypoint for the stateless-validator guest.
//!
//! Per-zkVM bins call [`entrypoint`] with their `Platform` implementation, which
//! supplies the input/output plumbing and cycle-count instrumentation; the crypto
//! provider is selected by cargo feature instead (see [`crate::crypto`]).
//!
//! Taking `ere-platform-core` rather than hand-mirroring each zkVM's read/write
//! convention makes the IO contract with `ere-server` structural.

use core::marker::PhantomData;
use ethrex_guest_program::l1::{GuestTrace, run_stateless_guest_traced};

pub use ere_platform_core::Platform;

/// Routes [`GuestTrace`] to a [`Platform`]'s cycle scopes, so the guest body
/// stays the single copy in `ethrex_guest_program::l1`. The scope names appear
/// in ZisK profiling output, so keep them stable.
struct PlatformTrace<P>(PhantomData<P>);

impl<P: Platform> GuestTrace for PlatformTrace<P> {
    fn scope<T>(name: &str, f: impl FnOnce() -> T) -> T {
        P::cycle_scope(name, f)
    }

    fn print(message: core::fmt::Arguments<'_>) {
        P::print(&message.to_string());
    }
}

/// Runs the stateless guest on the [`Platform`]: read `statelessInputBytes`,
/// validate, write `statelessOutputBytes`.
pub fn entrypoint<P: Platform>() {
    let input_bytes = P::cycle_scope("read_input", || P::read_input());
    let output_bytes =
        run_stateless_guest_traced::<PlatformTrace<P>>(&input_bytes, crate::crypto::crypto());
    P::cycle_scope("write_output", || P::write_output(&output_bytes));
}
