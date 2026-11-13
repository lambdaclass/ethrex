pub mod cli;
pub mod initializers;
pub mod integration;
#[cfg(feature = "l2")]
pub mod l2;
#[cfg(feature = "partial")]
pub mod partial;
pub mod utils;

pub mod decode;

#[cfg(feature = "jemalloc")]
#[doc(hidden)]
pub use tikv_jemallocator;

/// Declares the global allocator of a node binary, and its tuning. Invoke it once, at
/// the top level of the binary's `main.rs`.
///
/// With the `jemalloc` feature of this crate (on by default) the allocator is
/// jemalloc; without it the macro declares nothing and the binary uses the system
/// allocator.
// Tune jemalloc for throughput: use a background thread for memory purging instead
// of doing it inline during malloc/free (which adds ~12% of total block execution CPU time
// to jemalloc purge calls, measured via perf profiling on mainnet blocks).
#[cfg(all(feature = "jemalloc", not(feature = "jemalloc_profiling")))]
#[macro_export]
macro_rules! global_allocator {
    () => {
        #[cfg(not(target_env = "msvc"))]
        #[global_allocator]
        static ALLOC: $crate::tikv_jemallocator::Jemalloc = $crate::tikv_jemallocator::Jemalloc;

        #[cfg(not(target_env = "msvc"))]
        #[allow(non_upper_case_globals)]
        #[unsafe(export_name = "malloc_conf")]
        pub static malloc_conf: &[u8] =
            b"background_thread:true,dirty_decay_ms:30000,muzzy_decay_ms:30000\0";
    };
}

/// See the documentation of the default variant of this macro.
// This could be also enabled via `MALLOC_CONF` env var, but for consistency with the previous jemalloc feature
// usage, we keep it in the code and enable the profiling feature only with the `jemalloc_profiling` feature flag.
#[cfg(feature = "jemalloc_profiling")]
#[macro_export]
macro_rules! global_allocator {
    () => {
        #[cfg(not(target_env = "msvc"))]
        #[global_allocator]
        static ALLOC: $crate::tikv_jemallocator::Jemalloc = $crate::tikv_jemallocator::Jemalloc;

        #[cfg(not(target_env = "msvc"))]
        #[allow(non_upper_case_globals)]
        #[unsafe(export_name = "malloc_conf")]
        pub static malloc_conf: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19,background_thread:true,dirty_decay_ms:30000,muzzy_decay_ms:30000\0";
    };
}

/// See the documentation of the default variant of this macro.
#[cfg(not(feature = "jemalloc"))]
#[macro_export]
macro_rules! global_allocator {
    () => {};
}
