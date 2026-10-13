//! Allocation counts for code that runs inside zkVM guests, whose bump allocators never
//! free: there, every allocation made per call or per CREATE is memory the guest never gets
//! back. A counting global allocator counts the allocations made on the current thread, so
//! tests running in parallel don't see each other's.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

use ethrex_common::{
    Address,
    types::{Code, InitcodeBuffer},
};
use ethrex_levm::precompiles::PrecompileCache;

struct CountingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn count_allocation() {
    // `try_with` because the allocator also runs while thread-locals are torn down.
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

// SAFETY: every method forwards to `System` unchanged; counting has no effect on the memory.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    // A bump allocator serves a realloc with a new allocation, so it counts as one.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_allocation();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Runs `f` and returns how many allocations it made on this thread.
fn allocations_in(f: impl FnOnce()) -> usize {
    let before = ALLOCATIONS.with(Cell::get);
    f();
    ALLOCATIONS.with(Cell::get) - before
}

#[test]
fn a_repeated_precompile_cache_insert_copies_nothing() {
    let cache = PrecompileCache::new();
    let address = Address::from_low_u64_be(1);
    let calldata = [7u8; 64];
    let output = [9u8; 32];
    cache.insert(address, &calldata, &output, 1);
    assert!(
        cache.get(&address, &calldata).is_some(),
        "the first insert was not cached"
    );

    // The warmer and the executor both insert the calls they compute. The second insert
    // finds the entry, and must not copy the calldata and output only to drop them.
    let allocations = allocations_in(|| cache.insert(address, &calldata, &output, 1));

    assert_eq!(allocations, 0);
}

#[test]
fn the_jumpdest_bitmap_is_built_in_one_allocation() {
    // Every byte a JUMPDEST, so the bitmap is kept rather than swapped for the shared empty
    // one. Building it in a Vec and then moving it into an Arc allocates it twice.
    let code = [0x5b; 64];

    let allocations = allocations_in(|| drop(Code::compute_jumpdests(&code)));

    assert_eq!(allocations, 1);
}

#[test]
fn empty_code_gets_the_shared_empty_bitmap_without_allocating() {
    // The first use builds the shared empty bitmap; count from the second.
    drop(Code::compute_jumpdests(&[]));

    let allocations = allocations_in(|| drop(Code::compute_jumpdests(&[])));

    assert_eq!(allocations, 0);
}

#[test]
fn initcode_of_the_same_size_reuses_every_allocation_of_the_one_before() {
    let initcode = [0x5b; 1024];
    let mut buffer = InitcodeBuffer::default();
    buffer.set(&initcode);
    let mut buffer = Code::from_initcode_buffer(buffer).into_initcode_buffer();

    // The bytecode buffer already has room for the padding, so it becomes the code's bytes
    // without growing and comes back whole, and the bitmap comes back with it.
    let allocations = allocations_in(|| {
        buffer.set(&initcode);
        buffer = Code::from_initcode_buffer(std::mem::take(&mut buffer)).into_initcode_buffer();
    });

    assert_eq!(allocations, 0);
}
