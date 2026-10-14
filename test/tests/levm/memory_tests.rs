#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
use ethrex_common::U256;
use ethrex_levm::memory::Memory;

#[test]
fn test_basic_store_data() {
    let mut mem = Memory::new();

    mem.store_data(0, &[1, 2, 3, 4, 0, 0, 0, 0, 0, 0]).unwrap();

    assert_eq!(&mem.buffer.borrow()[0..10], &[1, 2, 3, 4, 0, 0, 0, 0, 0, 0]);
    assert_eq!(mem.len(), 32);
}

#[test]
fn test_words() {
    let mut mem = Memory::new();

    mem.store_word(0, U256::from(4)).unwrap();

    assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
    assert_eq!(mem.len(), 32);
}

#[test]
fn test_copy_word_within() {
    {
        let mut mem = Memory::new();

        mem.store_word(0, U256::from(4)).unwrap();
        mem.copy_within(0, 32, 32).unwrap();

        assert_eq!(mem.load_word(32).unwrap(), U256::from(4));
        assert_eq!(mem.len(), 64);
    }

    {
        let mut mem = Memory::new();

        mem.store_word(32, U256::from(4)).unwrap();
        mem.copy_within(32, 0, 32).unwrap();

        assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
        assert_eq!(mem.len(), 64);
    }

    {
        let mut mem = Memory::new();

        mem.store_word(0, U256::from(4)).unwrap();
        mem.copy_within(0, 0, 32).unwrap();

        assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
        assert_eq!(mem.len(), 32);
    }

    {
        let mut mem = Memory::new();

        mem.store_word(0, U256::from(4)).unwrap();
        mem.copy_within(32, 0, 32).unwrap();

        assert_eq!(mem.load_word(0).unwrap(), U256::zero());
        assert_eq!(mem.len(), 64);
    }
}

/// A call's arguments are a view of the caller's memory, not a copy, and stay correct
/// after the callee's memory growth moves the shared buffer to a new allocation.
#[test]
fn calldata_view_reads_caller_memory_in_place() {
    use ethrex_levm::memory::CallData;

    let mut caller = Memory::new();
    let args: Vec<u8> = (1..=40).collect();
    caller.store_data(8, &args).unwrap();

    let view = caller.calldata_view(8, 40).unwrap();
    assert!(
        matches!(view, CallData::CallerMemory(_)),
        "arguments were copied"
    );

    let mut callee = caller.next_memory();
    let before = callee.buffer.borrow().as_ptr();
    callee.store_data(1 << 20, &[0xff]).unwrap();
    assert_ne!(
        before,
        callee.buffer.borrow().as_ptr(),
        "the buffer did not move"
    );

    assert_eq!(view.with_slice(<[u8]>::to_vec).unwrap(), args);
    assert_eq!(view.len(), 40);
    let mut word = [0u8; 32];
    word[..8].copy_from_slice(&args[32..]);
    assert_eq!(view.load_word(32).unwrap(), U256::from_big_endian(&word));
    assert_eq!(view.load_word(40).unwrap(), U256::zero());
    assert_eq!(view.load_word(usize::MAX).unwrap(), U256::zero());
}

/// Arguments past the caller's memory read as zeros, as memory expansion requires, and
/// an empty argument region is no view at all.
#[test]
fn calldata_view_past_memory_reads_zeros() {
    let mut caller = Memory::new();
    let view = caller.calldata_view(100, 50).unwrap();
    assert_eq!(view.with_slice(<[u8]>::to_vec).unwrap(), vec![0; 50]);
    assert_eq!(caller.len(), 160);

    assert!(caller.calldata_view(1 << 30, 0).unwrap().is_empty());
}

/// CALLDATACOPY from a view of the caller's memory copies what is there and zero-fills
/// the rest, like a copy from owned call data.
#[test]
fn store_calldata_copies_and_zero_fills() {
    use bytes::Bytes;
    use ethrex_levm::memory::CallData;

    let args: Vec<u8> = (1..=40).collect();
    let mut caller = Memory::new();
    caller.store_data(0, &args).unwrap();
    let view = caller.calldata_view(0, 40).unwrap();
    let owned = CallData::from(Bytes::from(args.clone()));

    for calldata in [&view, &owned] {
        let mut callee = caller.next_memory();
        callee.store_data(0, &[0xaa; 128]).unwrap();

        callee.store_calldata(4, calldata, 30, 20).unwrap();
        let mut expected = vec![0xaa; 4];
        expected.extend_from_slice(&args[30..]);
        expected.resize(24, 0);
        expected.resize(32, 0xaa);
        assert_eq!(callee.live_bytes()[..32], expected[..]);

        callee.store_calldata(64, calldata, usize::MAX, 8).unwrap();
        assert_eq!(callee.live_bytes()[64..72], [0; 8]);

        callee.store_calldata(200, calldata, 0, 40).unwrap();
        assert_eq!(callee.live_bytes()[200..240], args[..]);
        assert_eq!(callee.len(), 256);
    }
}
