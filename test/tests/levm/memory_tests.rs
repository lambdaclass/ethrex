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
